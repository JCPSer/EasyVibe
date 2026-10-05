//! 应用服务编排层（方案 R2 批 B / S7）：把原 handler 内联的 100–200 行业务编排抽成
//! 显式入参的服务函数，handler 退化为「解析入参 → 调 service → 映射错误/响应」。
//!
//! 依赖方向（单向，不引用 routes，避免二次重叠）：
//!   routes/*.rs（HTTP 边界） → service（编排） → state.rs / 各 domain crate
//!
//! 本文件由 `routes/chat.rs`、`routes/task.rs` 的编排原样搬迁而来，**零语义改动**。

use crate::state::*;
use easyvibe_common::ApiError;
use easyvibe_ai_agent::QaClient as _;
use easyvibe_ai_agent::compaction;
use easyvibe_db::{SettingsRepository as _, TaskRepository as _};
use easyvibe_db::{ConversationMessageRow, ConversationRepository as _};
use tracing::info;

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

#[derive(serde::Deserialize)]
pub(crate) struct ChatHttpRequest {
    pub message: String,
    /// 图片附件：dataURL 数组（随消息发给视觉模型；不持久化原文，库中只留占位）
    #[serde(default)]
    pub images: Vec<String>,
    /// D9 @模块：用户显式钉住的模块 id 列表（只作上下文提示，不参与过滤）
    #[serde(default)]
    pub module_refs: Vec<String>,
    /// M4-2 多会话：目标会话 id（缺省=该仓库最近活跃会话）
    #[serde(default)]
    pub conv: Option<String>,
}

/// 入口对话编排（从 `routes/chat.rs::chat` 原样搬迁，零语义改动）。
/// 返回响应体 `data` 字段；HTTP 边界（状态码/响应包装）留在 handler。
pub(crate) async fn chat_once(st: &AppState, id: &str, body: ChatHttpRequest) -> Result<serde_json::Value, ApiError> {
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    if body.message.trim().is_empty() {
        return Err(ApiError::BadRequest("消息不能为空".into()));
    }
    let snap = st.map_service.load_map(&repo).await?;
    // 会话写串行化（审查 🟡1）：持锁覆盖 落库→装配→compact 全段
    let _guard = st.chat_lock.lock().await;
    let conv = resolve_conv(st, id, body.conv.as_deref()).await?;

    // 1) 用户消息落库（会话持久化：历史不再只活在前端 state）
    let persisted = if body.images.is_empty() {
        body.message.clone()
    } else {
        format!("{}\n[图片附件 {} 张（未持久化，重开对话后不可见）]", body.message, body.images.len())
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
            let cfg = resolve_llm(st, id, "chat").await;
            if cfg.api_key.is_empty() {
                return Err(ApiError::BadRequest("未配置 LLM API key（设置面板或 EASYVIBE_LLM_API_KEY）".into()));
            }
            let llm = easyvibe_ai_agent::AnthropicClient::new(&cfg.base_url, &cfg.api_key, &cfg.model);
            // S1-3：user_entry 插槽注入（§9 #4——仅用户入口对话；透明 agent 的空插槽装配永不注入）
            // 注入点 #4：自定义 global 块拼在 skills 之后（团队规则在入口对话同样生效）
            let h = st.harness.read().await;
            let skills = h.user_entry_skills.join("\n\n---\n\n");
            let prefix = if skills.trim().is_empty() {
                String::new()
            } else {
                format!("\n\n## 对话技能（grill-me：需求有歧义时主动用选择题澄清）\n\n{skills}\n\n---\n")
            };
            let prefix = format!("{prefix}{}", crate::task_exec::custom_block(&h.custom.global));
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
    let budget = resolve_adv_i64(st, id, "adv.contextBudget", DEFAULT_CONTEXT_BUDGET).await;
    let threshold = resolve_adv_i64(st, id, "adv.compactThreshold", DEFAULT_COMPACT_THRESHOLD).await;
    let compaction_trace = match maybe_compact(st, id, body.conv.as_deref(), budget, threshold, false).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("[chat] {} auto-compact 失败（不阻断对话）: {e}", id);
            None
        }
    };

    // 累计口径（前端"累计 tokens"与 GET 恢复一致；POST 的 usage 是单次调用增量）
    let usage = {
        let c = st.conversation_repo.get_or_create(id).await?;
        serde_json::json!({ "promptTokens": c.prompt_tokens, "completionTokens": c.completion_tokens })
    };

    Ok(serde_json::json!({
        "reply": answer.reply,
        "refs": answer.refs,
        "clarify": answer.clarify,
        "compaction": compaction_trace,
        "usage": usage,
    }))
}

#[derive(serde::Deserialize)]
pub(crate) struct CreateTaskRequest {
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub modules: Vec<String>,
    #[serde(default)]
    pub acceptance: String,
    #[serde(default)]
    pub source: String, // module / concern / layer / manual
    #[serde(default)]
    pub context: serde_json::Value,
    #[serde(default)]
    pub trust: String, // manual / auto
    /// M4-2：任务←→会话关联（对话升级/工作台聚合）
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// R3 D2：返工来源（驳回→复制为新任务）——后端统一注入驳回理由并回填反链，
    /// 三个前端入口（评审/收件箱/任务页）不必各自拼理由
    #[serde(default)]
    pub origin_task_id: Option<String>,
}

/// 影响面合约展开：声明模块 → 其 files glob 边界（地图缺失/模块未命中时静默不约束——
/// 合约是增强护栏不是准入门槛，与"提示不拦截"哲学一致）
pub(crate) async fn task_ctx_with_contract(
    st: &AppState,
    repo: &easyvibe_map::Repo,
    modules: &[String],
    mut ctx: serde_json::Value,
) -> serde_json::Value {
    if modules.is_empty() {
        return ctx;
    }
    let Ok(snap) = st.map_service.load_map(repo).await else { return ctx };
    let mut patterns: Vec<String> = vec![];
    if let Some(mods) = snap.json["modules"].as_array() {
        for mid in modules {
            if let Some(m) = mods.iter().find(|m| m["id"].as_str() == Some(mid.as_str())) {
                if let Some(files) = m["files"].as_array() {
                    patterns.extend(files.iter().filter_map(|f| f.as_str().map(str::to_string)));
                }
            }
        }
    }
    if !patterns.is_empty() {
        ctx["contract"] = serde_json::json!({ "modules": modules, "patterns": patterns });
    }
    ctx
}

/// 建任务编排（从 `routes/task.rs::create_task` 原样搬迁，零语义改动）。
/// 返回新任务 id；HTTP 边界（201/响应包装）留在 handler。
pub(crate) async fn create_task(st: &AppState, id: &str, body: CreateTaskRequest) -> Result<String, ApiError> {
    use easyvibe_db::{ApprovalRepository as _, EventRepository as _, TaskRepository as _};
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    if body.description.trim().is_empty() {
        return Err(ApiError::BadRequest("需求描述不能为空".into()));
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0).to_string();
    let task_id = format!("task-{}", &now);
    let repo_id = repo.id.clone();
    // R3 D2：返工来源——三入口统一在后端注入驳回理由（此前只有评审页注入，记忆强度取决于入口），
    // 并回填原任务的 successor 反链（返工率聚合的命脉）。
    // 前端惯例放在 context.origin_task_id（TaskDraft.context 透传），兼容顶层 origin_task_id
    let origin_input = body.origin_task_id.or_else(|| {
        body.context
            .get("origin_task_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    });
    let mut description = body.description;
    let mut origin_task_id = None;
    if let Some(origin_id) = origin_input.as_deref() {
        if let Some(origin) = st.task_repo.get(origin_id).await? {
            origin_task_id = Some(origin.id.clone());
            let reason = st
                .approval_repo
                .list_by_task(&origin.id)
                .await
                .ok()
                .map(|aps| {
                    aps.into_iter()
                        .filter(|a| a.decision == "rejected" && a.note.as_deref().map(str::trim).is_some_and(|n| !n.is_empty()))
                        .map(|a| a.note.unwrap())
                        .collect::<Vec<_>>()
                        .join("；")
                })
                .filter(|r| !r.is_empty());
            if let Some(r) = reason {
                description = format!("{description}\n\n—— 返工自 {}（驳回理由：{}）", origin.id, r);
            } else {
                description = format!("{description}\n\n—— 返工自 {}", origin.id);
            }
        }
    }
    let contract_ctx = task_ctx_with_contract(st, &repo, &body.modules, body.context.clone()).await;
    let has_contract = contract_ctx.get("contract").is_some();
    let row = easyvibe_db::TaskRow {
        id: task_id.clone(),
        repo: repo_id.clone(),
        title: body.title,
        description,
        modules: serde_json::to_string(&body.modules).unwrap_or_else(|_| "[]".into()),
        acceptance: body.acceptance,
        source: if body.source.is_empty() { "manual".into() } else { body.source },
        // 影响面合约（战略审查第一 P0）：声明的模块在创建时展开为 glob 边界写入 context——
        // 终态采集对全部变更文件做确定性越界校验（task_exec::collect_task_result），零 LLM。
        context: serde_json::to_string(&contract_ctx).unwrap_or_else(|_| "{}".into()),
        status: "pending".into(), // M3-3：harness 执行引擎接走
        trust: match body.trust.as_str() {
            "auto" => "auto".into(),
            "supervised" => "supervised".into(),
            _ => "manual".into(),
        },
        error: None,
        session_id: None,
        gate: None,
        conversation_id: body.conversation_id,
        prompt_tokens: None,
        completion_tokens: None,
        result: None,
        base_head: None,
        created_at: now.clone(),
        updated_at: now,
        origin_task_id,
        successor_task_id: None,
    };
    st.task_repo.create(&row).await?;
    // 返工链反链回填（原任务 → 本任务）
    if let Some(origin_id) = row.origin_task_id.as_deref() {
        let _ = st.task_repo.set_successor(origin_id, &task_id).await;
    }
    // R3 D1：影响模块填写率/合约声明率——L3 门①的原始计数
    let _ = st
        .event_repo
        .record(&repo_id, "task.created", &serde_json::json!({ "trust": row.trust, "modules": body.modules.len(), "hasContract": has_contract, "rework": row.origin_task_id.is_some() }).to_string())
        .await;
    // M3-3：入队执行（harness 引擎；并发上限 4，审批门 M3-4 接入）
    st.executor.clone().enqueue_pending(Some(&repo_id)).await;
    Ok(task_id)
}
