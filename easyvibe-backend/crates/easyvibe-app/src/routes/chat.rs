//! 资源域：对话/会话/压缩/视图。

use crate::state::*;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use easyvibe_common::ApiError;
use easyvibe_ai_agent::QaClient as _;
use tracing::info;
use easyvibe_ai_agent::compaction;
use easyvibe_db::{SettingsRepository as _, TaskRepository as _};
use easyvibe_db::{ConversationMessageRow, ConversationRepository as _};

/// 近期窗口原文保留的消息条数（3 轮问答不动，三层策略第 1 层）
pub(crate) const KEEP_RECENT_MESSAGES: usize = 6;

pub(crate) const DEFAULT_CONTEXT_BUDGET: i64 = 256_000; // §10 #2：默认 256K，高级设置可调

pub(crate) const DEFAULT_COMPACT_THRESHOLD: i64 = 80;   // §10a：触发 80% → 压到 40%

/// 高级设置解析：仓库行覆盖全局行（同 resolve_llm 的两级哲学）
pub(crate) async fn resolve_adv_i64(st: &AppState, repo_id: &str, key: &str, default: i64) -> i64 {
    for scope in [repo_id, "global"] {
        if let Ok(Some(row)) = st.settings_repo.get(scope, key).await {
            if let Ok(v) = serde_json::from_str::<i64>(&row.value) { return v; }
            if let Ok(v) = serde_json::from_str::<f64>(&row.value) { return v as i64; }
        }
    }
    default
}

/// 未压缩消息折叠为 (q, a) 对（容错奇数/乱序；系统消息不参与）
pub(crate) fn fold_pairs(messages: &[ConversationMessageRow]) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut pending_q: Option<String> = None;
    for m in messages {
        match m.role.as_str() {
            "user" => pending_q = Some(m.content.clone()),
            "assistant" => {
                if let Some(q) = pending_q.take() {
                    pairs.push((q, m.content.clone()));
                }
            }
            _ => {}
        }
    }
    pairs
}

/// 压缩执行（auto 与手动共用）：返回留痕消息（"上下文已压缩：82%→34%"）。
/// 存储分离（§11 🟡5）：水位前消息标 compacted（原文保留可回放），运行态只剩摘要+窗口；
/// 摘要必带会话状态（§11 🟡6）：compact_stub/compact_with_llm 的结构化段落保证。
/// 调用方约定：auto 路径（chat 第 5 步）必须吞错降级——压缩绝不可打断已成功的对话（审查 🔴）；
/// 手动路径（compact_chat）传播错误，那里没有已落库的回答可损失。
pub(crate) async fn maybe_compact(st: &AppState, repo_id: &str, conv_id: Option<&str>, budget: i64, threshold: i64, force: bool) -> Result<Option<String>, ApiError> {
    // M4-2：压缩按会话（缺省=该仓库最近活跃会话）
    let conv = resolve_conv(st, repo_id, conv_id).await?;
    let fresh = st.conversation_repo.list_uncompacted(&conv.id).await?;
    // 触发口径 = 真实装配口径：未压缩窗口 + 既有摘要（摘要自身增长也会再触发，护栏闭环）
    let total: i64 = fresh.iter().map(|m| m.tokens).sum::<i64>()
        + conv.summary.as_deref().map(easyvibe_ai_agent::estimate_tokens).unwrap_or(0);
    if !force && !compaction::needs_compaction(total, budget, threshold) {
        return Ok(None);
    }
    let Some(wm) = compaction::compaction_watermark(&fresh, KEEP_RECENT_MESSAGES) else {
        return Ok(None); // 不足一个窗口不压
    };
    let old: Vec<ConversationMessageRow> = fresh.iter().filter(|m| m.id <= wm).cloned().collect();
    if old.is_empty() {
        return Ok(None);
    }
    let result = match *st.llm_mode {
        LlmMode::Stub => compaction::compact_stub(conv.summary.as_deref(), &old, budget, total),
        LlmMode::Anthropic => {
            let cfg = resolve_llm(st, repo_id, "chat").await;
            if cfg.api_key.is_empty() {
                // 无 key 退化 stub（诚实标注），压缩不可用不该打断对话
                compaction::compact_stub(conv.summary.as_deref(), &old, budget, total)
            } else {
                let llm = easyvibe_ai_agent::AnthropicClient::new(&cfg.base_url, &cfg.api_key, &cfg.model);
                compaction::compact_with_llm(&llm, conv.summary.as_deref(), &old, budget, total).await?
            }
        }
    };
    st.conversation_repo
        .apply_compaction(&conv.id, result.before_id, &result.summary, result.prompt_tokens, result.completion_tokens)
        .await?;
    let trace = format!("上下文已压缩：{}%→{}%", result.before_pct, result.after_pct);
    st.conversation_repo
        .append_message(&conv.id, "system", &trace, easyvibe_ai_agent::estimate_tokens(&trace))
        .await?;
    info!("[chat] {} 压缩 {}%→{}%（水位 {}）", repo_id, result.before_pct, result.after_pct, result.before_id);
    Ok(Some(trace))
}

/// 恢复对话（R1 分页：?before=<id>&limit=50——切换页签/重启/刷新均恢复；
/// hasMore 为真时前端给"加载更早"入口，不再全表读）

/// M4-2 多会话：解析目标会话（缺省=该仓库最近活跃的会话）
pub(crate) async fn resolve_conv(st: &AppState, repo: &str, conv: Option<&str>) -> Result<easyvibe_db::ConversationRow, ApiError> {
    match conv {
        Some(cid) => {
            let rows = st.conversation_repo.list_by_repo(repo).await?;
            rows.into_iter().find(|c| c.id == cid).ok_or_else(|| ApiError::NotFound(format!("会话 {cid} 不存在于仓库 {repo}")))
        }
        None => st.conversation_repo.get_or_create(repo).await,
    }
}

/// 会话摘要（AionUI TConversationRuntimeSummary 精简版）：state + pending 审批数 + 消息数
pub(crate) async fn conversation_summary(st: &AppState, c: &easyvibe_db::ConversationRow) -> serde_json::Value {
    let tasks = st.task_repo.list_by_conversation(&c.id).await.unwrap_or_default();
    let pending = tasks.iter().filter(|t| t.status == "awaiting_approval").count();
    let running = tasks.iter().filter(|t| t.status == "running" || t.status == "pending").count();
    let msg_count = st.conversation_repo.count_messages(&c.id).await.unwrap_or(0);
    serde_json::json!({
        "id": c.id, "title": c.title, "repo": c.repo,
        "createdAt": c.created_at, "updatedAt": c.updated_at,
        "messageCount": msg_count,
        "usage": { "promptTokens": c.prompt_tokens, "completionTokens": c.completion_tokens },
        "runtime": {
            "state": if running > 0 { "running" } else if pending > 0 { "waiting_confirmation" } else { "idle" },
            "pendingConfirmations": pending,
            "runningTasks": running,
        },
    })
}

pub(crate) async fn list_conversations(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let convs = st.conversation_repo.list_by_repo(&id).await?;
    let mut items = Vec::new();
    for c in &convs {
        items.push(conversation_summary(&st, c).await);
    }
    Ok(Json(serde_json::json!({ "success": true, "data": items })).into_response())
}

#[derive(serde::Deserialize)]
pub(crate) struct CreateConversationRequest {
    #[serde(default)]
    title: Option<String>,
}

pub(crate) async fn create_conversation(State(st): State<AppState>, Path(id): Path<String>, Json(body): Json<CreateConversationRequest>) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let cid = format!("chat:{id}:{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
    let conv = st.conversation_repo.create(&cid, &id, body.title.as_deref()).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": conversation_summary(&st, &conv).await })).into_response())
}

#[derive(serde::Deserialize)]
pub(crate) struct RenameConversationRequest {
    title: String,
}

pub(crate) async fn rename_conversation(State(st): State<AppState>, Path((id, cid)): Path<(String, String)>, Json(body): Json<RenameConversationRequest>) -> Result<Response, AppError> {
    if body.title.trim().is_empty() {
        return Err(AppError(ApiError::BadRequest("会话名不能为空".into())));
    }
    resolve_conv(&st, &id, Some(&cid)).await?;
    st.conversation_repo.rename(&cid, body.title.trim()).await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

pub(crate) async fn delete_conversation(State(st): State<AppState>, Path((id, cid)): Path<(String, String)>) -> Result<Response, AppError> {
    resolve_conv(&st, &id, Some(&cid)).await?;
    let convs = st.conversation_repo.list_by_repo(&id).await?;
    if convs.len() <= 1 {
        return Err(AppError(ApiError::BadRequest("每个仓库至少保留一个会话".into())));
    }
    st.conversation_repo.delete(&cid).await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

pub(crate) async fn get_chat(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    // M4-2 多会话：?conv=<id> 选择会话（缺省=最近活跃）
    let conv = resolve_conv(&st, &id, q.get("conv").map(|v| v.as_str())).await?;
    let before = q.get("before").and_then(|v| v.parse::<i64>().ok());
    let limit: i64 = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50).clamp(1, 200);
    let messages = st.conversation_repo.list_messages(&conv.id, before, limit).await?;
    let has_more = messages.len() as i64 == limit;
    // 内联审批数据源：该会话关联任务中等待审批的门（AionUI 内联审批卡模式）
    let pending_approvals: Vec<serde_json::Value> = st.task_repo.list_by_conversation(&conv.id).await.unwrap_or_default()
        .into_iter()
        .filter(|t| t.status == "awaiting_approval")
        .map(|t| serde_json::json!({ "taskId": t.id, "title": t.title, "gate": t.gate }))
        .collect();
    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "conversation": { "id": conv.id, "title": conv.title },
            "summary": conv.summary,
            "messages": messages,
            "hasMore": has_more,
            "pendingApprovals": pending_approvals,
            "usage": { "promptTokens": conv.prompt_tokens, "completionTokens": conv.completion_tokens },
        }
    }))
    .into_response())
}

#[derive(serde::Deserialize)]
pub(crate) struct ChatHttpRequest {
    message: String,
    /// 图片附件：dataURL 数组（随消息发给视觉模型；不持久化原文，库中只留占位）
    #[serde(default)]
    images: Vec<String>,
    /// D9 @模块：用户显式钉住的模块 id 列表（只作上下文提示，不参与过滤）
    #[serde(default)]
    module_refs: Vec<String>,
    /// M4-2 多会话：目标会话 id（缺省=该仓库最近活跃会话）
    #[serde(default)]
    conv: Option<String>,
}

/// 入口对话（M2-5 + M3-5 持久化）：服务端是会话事实源——
/// 用户消息落库 → 运行态上下文=摘要+未压缩窗口 → 问答 → 回答落库+token 记账 → auto-compact 检查
pub(crate) async fn chat(State(st): State<AppState>, Path(id): Path<String>, Json(body): Json<ChatHttpRequest>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    if body.message.trim().is_empty() {
        return Err(AppError(ApiError::BadRequest("消息不能为空".into())));
    }
    let snap = st.map_service.load_map(&repo).await?;
    // 会话写串行化（审查 🟡1）：持锁覆盖 落库→装配→compact 全段
    let _guard = st.chat_lock.lock().await;
    let conv = resolve_conv(&st, &id, body.conv.as_deref()).await?;

    // 1) 用户消息落库（会话持久化：历史不再只活在前端 state）
    let persisted = if body.images.is_empty() {
        body.message.clone()
    } else {
        format!("{}
[图片附件 {} 张（未持久化，重开对话后不可见）]", body.message, body.images.len())
    };
    st.conversation_repo
        .append_message(&conv.id, "user", &persisted, easyvibe_ai_agent::estimate_tokens(&persisted))
        .await?;

    // 2) 运行态上下文：未压缩消息（近期窗口原文 + 此前由摘要代表）
    let fresh = st.conversation_repo.list_uncompacted(&conv.id).await?;
    let pairs = fold_pairs(&fresh);

    // 3) 问答（槽位配置 M3-1）
    // D9 @模块：把命中的模块概要拼成聚焦块前置给 LLM（原文落库，聚焦块只在本次调用的消息头）
    let snap_json: &serde_json::Value = &snap.json;
    let focus_block = {
        let mods = snap_json.get("modules").and_then(|m| m.as_array()).cloned().unwrap_or_default();
        let known: Vec<String> = body
            .module_refs
            .iter()
            .filter(|id| mods.iter().any(|m| m.get("id").and_then(|v| v.as_str()) == Some(id.as_str())))
            .cloned()
            .collect();
        if known.is_empty() {
            String::new()
        } else {
            let lines: Vec<String> = known
                .iter()
                .filter_map(|id| mods.iter().find(|m| m.get("id").and_then(|v| v.as_str()) == Some(id.as_str())))
                .map(|m| {
                    let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let resp = m.get("responsibility").and_then(|v| v.as_str()).unwrap_or("");
                    let score = m.get("health").and_then(|h| h.get("score")).and_then(|v| v.as_i64()).unwrap_or(-1);
                    format!("- {name}（{id}）：{resp}　健康分 {score}")
                })
                .collect();
            format!("【本轮聚焦模块】（用户显式 @ 引用，回答请优先围绕这些模块展开）\n{}\n\n", lines.join("\n"))
        }
    };
    let llm_message = format!("{focus_block}{}", body.message);
    let answer: easyvibe_ai_agent::QaAnswer = match *st.llm_mode {
        LlmMode::Stub => easyvibe_ai_agent::StubQaClient::new().ask(&snap.json, &llm_message, &pairs, &body.images).await?,
        LlmMode::Anthropic => {
            let cfg = resolve_llm(&st, &id, "chat").await;
            if cfg.api_key.is_empty() {
                return Err(AppError(ApiError::BadRequest("未配置 LLM API key（设置面板或 EASYVIBE_LLM_API_KEY）".into())));
            }
            let llm = easyvibe_ai_agent::AnthropicClient::new(&cfg.base_url, &cfg.api_key, &cfg.model);
            // S1-3：user_entry 插槽注入（§9 #4——仅用户入口对话；透明 agent 的空插槽装配永不注入）
            let skills = st.harness.read().await.user_entry_skills.join("\n\n---\n\n");
            let prefix = if skills.trim().is_empty() {
                String::new()
            } else {
                format!("\n\n## 对话技能（grill-me：需求有歧义时主动用选择题澄清）\n\n{skills}\n\n---\n")
            };
            easyvibe_ai_agent::LlmQaClient::new_with_prefix(llm, &prefix).ask(&snap.json, &llm_message, &pairs, &body.images).await?
        }
    };

    // 4) 回答落库 + token 用量累计（§10 #4）
    st.conversation_repo
        .append_message(&conv.id, "assistant", &answer.reply, easyvibe_ai_agent::estimate_tokens(&answer.reply))
        .await?;
    st.conversation_repo
        .add_tokens(&conv.id, answer.prompt_tokens as i64, answer.completion_tokens as i64)
        .await?;

    // 5) auto-compact（§10a：80% 触发，全自动不打断）——压缩失败降级不传播（审查 🔴：
    //    回答已落库，绝不能让第 5 步把本轮问答变成 500）
    let budget = resolve_adv_i64(&st, &id, "adv.contextBudget", DEFAULT_CONTEXT_BUDGET).await;
    let threshold = resolve_adv_i64(&st, &id, "adv.compactThreshold", DEFAULT_COMPACT_THRESHOLD).await;
    let compaction_trace = match maybe_compact(&st, &id, body.conv.as_deref(), budget, threshold, false).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("[chat] {} auto-compact 失败（不阻断对话）: {e}", id);
            None
        }
    };

    // 累计口径（前端"累计 tokens"与 GET 恢复一致；POST 的 usage 是单次调用增量）
    let usage = {
        let c = st.conversation_repo.get_or_create(&id).await?;
        serde_json::json!({ "promptTokens": c.prompt_tokens, "completionTokens": c.completion_tokens })
    };

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "reply": answer.reply,
            "refs": answer.refs,
            "clarify": answer.clarify,
            "compaction": compaction_trace,
            "usage": usage,
        }
    }))
    .into_response())
}

/// 手动压缩（§10a：对话界面"压缩上下文"按钮；自动阈值兜底之外的主动手段）
pub(crate) async fn compact_chat(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let _guard = st.chat_lock.lock().await;
    let budget = resolve_adv_i64(&st, &id, "adv.contextBudget", DEFAULT_CONTEXT_BUDGET).await;
    let trace = maybe_compact(&st, &id, q.get("conv").map(|v| v.as_str()), budget, 0, true).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "compacted": trace.is_some(), "trace": trace } })).into_response())
}

/// 新对话：清空消息与摘要（会话行保留，token 计数归零）
pub(crate) async fn reset_chat(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let _guard = st.chat_lock.lock().await;
    let conv = resolve_conv(&st, &id, q.get("conv").map(|v| v.as_str())).await?;
    st.conversation_repo.reset(&conv.id).await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

#[derive(serde::Deserialize)]
pub(crate) struct SaveViewRequest {
    name: String,
    #[serde(default)]
    nodes: Vec<String>, // ["module:exam-core", ...]
    #[serde(default)]
    edges: Vec<serde_json::Value>,
    #[serde(default)]
    annotations: Vec<serde_json::Value>,
}

/// 视图列表（F1b 读侧）：.easyvibe/views/*.json 引用式视图，供前端"视图"页签消费
pub(crate) async fn list_views(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let dir = repo.root.join(".easyvibe/views");
    let mut items: Vec<serde_json::Value> = vec![];
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else { continue };
            items.push(serde_json::json!({
                "slug": path.file_stem().and_then(|s| s.to_str()).unwrap_or(""),
                "name": v["name"],
                "createdAt": v["created_at"],
                "nodes": v["nodes"].as_array().map(|a| a.len()).unwrap_or(0),
                "view": v,
            }));
        }
    }
    items.sort_by(|a, b| b["createdAt"].as_str().cmp(&a["createdAt"].as_str()));
    Ok(Json(serde_json::json!({ "success": true, "data": items })).into_response())
}

/// 删除视图（F1b 读侧闭环）：slug 复用保存时的安全字符集，防线同 save_view
pub(crate) async fn delete_view(State(st): State<AppState>, Path((id, slug)): Path<(String, String)>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let safe: String = slug
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || ('\u{4e00}'..='\u{9fff}').contains(&c) { c } else { '-' })
        .collect();
    if safe.is_empty() || safe != slug {
        return Err(AppError(ApiError::BadRequest("非法视图标识".into())));
    }
    let path = repo.root.join(".easyvibe/views").join(format!("{safe}.json"));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| ApiError::Internal(format!("删除视图失败: {e}")))?;
    }
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

#[derive(serde::Deserialize)]
pub(crate) struct RenameViewRequest {
    name: String,
}

/// 视图改名（重审 P1：此前只能删了重建——保存冲突还会静默另存新文件造成列表膨胀）。
/// 改名 = 新 slug 写文件 + 删旧文件；slug 冲突 409（同 slug = 纯改名，原地更新 name 字段）。
pub(crate) async fn rename_view(State(st): State<AppState>, Path((id, slug)): Path<(String, String)>, Json(body): Json<RenameViewRequest>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    // 旧 slug 校验与 delete_view 同防线
    let safe_old: String = slug
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || ('\u{4e00}'..='\u{9fff}').contains(&c) { c } else { '-' })
        .collect();
    if safe_old.is_empty() || safe_old != slug {
        return Err(AppError(ApiError::BadRequest("非法视图标识".into())));
    }
    if body.name.trim().is_empty() {
        return Err(AppError(ApiError::BadRequest("视图名不能为空".into())));
    }
    let new_slug: String = body
        .name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || ('\u{4e00}'..='\u{9fff}').contains(&c) { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(40)
        .collect();
    let new_slug = if new_slug.is_empty() {
        return Err(AppError(ApiError::BadRequest("视图名需至少含一个字母或数字".into())));
    } else {
        new_slug
    };
    let dir = repo.root.join(".easyvibe/views");
    let old_path = dir.join(format!("{safe_old}.json"));
    if !old_path.is_file() {
        return Err(AppError(ApiError::NotFound(format!("视图 {slug} 不存在"))));
    }
    let new_path = dir.join(format!("{new_slug}.json"));
    if new_slug != safe_old && new_path.exists() {
        return Err(AppError(ApiError::Conflict(format!("已存在同名视图 {new_slug}，请换一个名字"))));
    }
    let raw = std::fs::read_to_string(&old_path).map_err(|e| ApiError::Internal(format!("读取视图失败: {e}")))?;
    let mut v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| ApiError::Internal(format!("视图解析失败: {e}")))?;
    v["name"] = serde_json::Value::String(body.name.trim().to_string());
    std::fs::write(&new_path, serde_json::to_string_pretty(&v).unwrap_or_else(|_| raw.clone()))
        .map_err(|e| ApiError::Internal(format!("写入视图失败: {e}")))?;
    if new_slug != safe_old {
        let _ = std::fs::remove_file(&old_path);
    }
    Ok(Json(serde_json::json!({ "success": true, "data": { "slug": new_slug } })).into_response())
}

/// 存为视图（F1b 首次消费）：按格式规范 §9 写 .easyvibe/views/<slug>.json（引用式，不存布局）
/// 2026-10-04 审计 P1：同名冲突从"静默另存后缀"改为 409——前端弹覆盖确认（?force=true 覆盖），
/// 数据去向由用户裁决，不再悄悄换名。
pub(crate) async fn save_view(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
    Json(body): Json<SaveViewRequest>,
) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let slug: String = body
        .name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || ('\u{4e00}'..='\u{9fff}').contains(&c) { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(40)
        .collect();
    let slug = if slug.is_empty() {
        format!("view-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0))
    } else {
        slug
    };
    let view_path = repo.root.join(".easyvibe/views").join(format!("{slug}.json"));
    let force = q.get("force").map(|v| v == "true").unwrap_or(false);
    if view_path.exists() && !force {
        return Err(AppError(ApiError::Conflict(format!("已存在同名视图 {slug}"))));
    }
    let view = serde_json::json!({
        "version": "1.0",
        "name": body.name,
        "created_at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string(),
        "source": { "conversation_id": "manual" },
        "nodes": body.nodes.iter().map(|r| serde_json::json!({ "ref": r })).collect::<Vec<_>>(),
        "edges": body.edges,
        "annotations": body.annotations,
    });
    let dir = repo.root.join(".easyvibe/views");
    tokio::fs::create_dir_all(&dir).await.map_err(|e| ApiError::Internal(format!("创建 views 目录失败: {e}")))?;
    let path = dir.join(format!("{slug}.json"));
    easyvibe_map::atomic_write_json(&path, &view).await?;
    Ok((axum::http::StatusCode::CREATED, Json(serde_json::json!({ "success": true, "data": { "path": path.to_string_lossy() } }))).into_response())
}
