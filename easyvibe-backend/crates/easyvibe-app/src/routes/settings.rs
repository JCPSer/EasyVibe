//! 资源域：设置/密钥/harness 管理与诊断导出。

use crate::state::*;
use axum::{
    extract::{Path, State},
    routing::{delete, get, post, put},
    response::{IntoResponse, Response},
    Json, Router,
};
use easyvibe_common::ApiError;
use tracing::info;
use crate::task_exec;

/// 本域路由（R1 自注册）：设置/密钥/harness 管理与诊断导出。
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/settings", get(list_settings))
        .route("/settings/set", put(put_setting))
        .route("/settings/{scope}/{key}", delete(delete_setting))
        .route("/harness", get(get_harness))
        .route("/harness/custom/files", get(list_custom_files))
        .route("/harness/custom/file", get(get_custom_file).put(put_custom_file).delete(delete_custom_file))
        .route("/harness/custom/toggle", put(toggle_custom_file))
        .route("/harness/custom/template", get(get_custom_template))
        .route("/harness/custom/generate", post(generate_custom))
        .route("/diagnostics", get(export_diagnostics))
}

pub(crate) async fn list_settings(State(st): State<AppState>, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> Result<Response, AppError> {
    let scope = q.get("scope").cloned().unwrap_or_else(|| "global".into());
    Ok(Json(crate::service::settings::list_settings(&st, &scope).await?).into_response())
}

#[derive(serde::Deserialize)]
pub(crate) struct PutSettingRequest {
    scope: String,
    key: String,
    value: serde_json::Value,
}

pub(crate) async fn put_setting(State(st): State<AppState>, Json(body): Json<PutSettingRequest>) -> Result<Response, AppError> {
    crate::service::settings::put_setting(&st, &body.scope, &body.key, body.value).await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

pub(crate) async fn delete_setting(State(st): State<AppState>, Path((scope, key)): Path<(String, String)>) -> Result<Response, AppError> {
    crate::service::settings::delete_setting(&st, &scope, &key).await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

/// Y3：一键诊断导出——把"用户报障口头描述"变成"导出一个文件"
/// 最近 200 行日志 + 后端版本 + 各表计数（settings 的加密值剔除）
pub(crate) async fn export_diagnostics(State(st): State<AppState>) -> Result<Response, AppError> {
    Ok(Json(crate::service::settings::export_diagnostics(&st).await?).into_response())
}

/// S1-3：harness 状态——两栏改造（方案 v2 §5）后只暴露版本状态（about 页数据面），
/// 出厂文件清单不再下发（出厂层不对用户展示）
pub(crate) async fn get_harness(State(st): State<AppState>) -> Result<Response, AppError> {
    let h = st.harness.read().await;
    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "dir": h.dir.to_string_lossy(),
            "manifest": {
                "id": h.manifest.id, "version": h.manifest.version, "builtin": h.manifest.builtin,
            },
            "frameworkNeutralized": h.framework_transparent.contains("透明执行模式"),
            "userEntrySkillCount": h.user_entry_skills.len(),
        }
    }))
    .into_response())
}

// ---------- 自定义层端点（方案 v2 §5——出厂层不对用户展示，custom 层是唯一可写面） ----------

/// 槽位存在性/大小/启用状态清单
pub(crate) async fn list_custom_files() -> Result<Response, AppError> {
    let dir = task_exec::harness_custom_dir();
    let state = task_exec::read_custom_state(&dir);
    let slots: Vec<serde_json::Value> = task_exec::CUSTOM_SLOTS
        .iter()
        .map(|(file, key)| {
            let p = dir.join(file);
            let (exists, size, mtime) = std::fs::metadata(&p)
                .map(|m| (true, m.len(), m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0)))
                .unwrap_or((false, 0, 0));
            serde_json::json!({
                "path": file, "slot": key, "exists": exists, "size": size, "mtimeMs": mtime,
                "enabled": state[*key].as_bool().unwrap_or(true),
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "success": true, "data": { "slots": slots } })).into_response())
}

/// custom 文件读取：canonicalize 越界防线（与出厂版同纪律），钳制在 harness-custom/ 内
pub(crate) async fn get_custom_file(
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let rel = q.get("path").cloned().unwrap_or_default();
    let full = custom_slot_path(&rel)?;
    let canonical = full.canonicalize().map_err(|_| AppError(ApiError::NotFound("文件不存在".into())))?;
    let dir_canon = task_exec::harness_custom_dir().canonicalize().map_err(|e| ApiError::Internal(format!("harness-custom 目录不可读: {e}")))?;
    if !canonical.starts_with(&dir_canon) || !canonical.is_file() {
        return Err(AppError(ApiError::NotFound("文件不存在（路径越界或非文件）".into())));
    }
    let content = std::fs::read_to_string(&canonical).map_err(|e| ApiError::Internal(format!("文件读取失败: {e}")))?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "path": rel, "content": content } })).into_response())
}

/// custom 文件写入：新槽默认启用（写入 state.json）；写后热装载（与出厂 put 同纪律）
pub(crate) async fn put_custom_file(
    State(st): State<AppState>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let rel = body.get("path").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let content = body.get("content").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let full = custom_slot_path(&rel)?;
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ApiError::Internal(format!("目录创建失败: {e}")))?;
    }
    std::fs::write(&full, content).map_err(|e| ApiError::Internal(format!("文件写入失败: {e}")))?;
    // 新槽默认启用（state 缺失条目 = 启用；显式写 true 让状态可见）
    let dir = task_exec::harness_custom_dir();
    let mut state = task_exec::read_custom_state(&dir);
    if let (Some(obj), Some(key)) = (state.as_object_mut(), slot_key_of(&rel)) {
        obj.entry(key.to_string()).or_insert(serde_json::Value::Bool(true));
    }
    task_exec::write_custom_state(&dir, &state)?;
    let fresh = task_exec::load_harness()?;
    *st.harness.write().await = fresh;
    info!("[harness] 自定义槽 {} 已保存并热装载", rel);
    Ok(Json(serde_json::json!({ "success": true, "data": { "path": rel } })).into_response())
}

/// custom 文件删除：关闭该槽补充（state.json 同步清条目），热装载
pub(crate) async fn delete_custom_file(
    State(st): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let rel = q.get("path").cloned().unwrap_or_default();
    let full = custom_slot_path(&rel)?;
    if full.exists() {
        let canonical = full.canonicalize().map_err(|_| AppError(ApiError::NotFound("文件不存在".into())))?;
        let dir_canon = task_exec::harness_custom_dir().canonicalize().map_err(|e| ApiError::Internal(format!("harness-custom 目录不可读: {e}")))?;
        if !canonical.starts_with(&dir_canon) || !canonical.is_file() {
            return Err(AppError(ApiError::NotFound("文件不存在（路径越界或非文件）".into())));
        }
        std::fs::remove_file(&canonical).map_err(|e| ApiError::Internal(format!("文件删除失败: {e}")))?;
    }
    let dir = task_exec::harness_custom_dir();
    let mut state = task_exec::read_custom_state(&dir);
    if let (Some(obj), Some(key)) = (state.as_object_mut(), slot_key_of(&rel)) {
        obj.remove(key);
    }
    task_exec::write_custom_state(&dir, &state)?;
    let fresh = task_exec::load_harness()?;
    *st.harness.write().await = fresh;
    info!("[harness] 自定义槽 {} 已删除并热装载", rel);
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

/// 槽位开关（方案 v2 §2：停用 ≠ 删除——文件保留，仅停止注入）；写 state.json 并热装载
pub(crate) async fn toggle_custom_file(
    State(st): State<AppState>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let rel = body.get("path").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let enabled = body.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
    let key = slot_key_of(&rel).ok_or_else(|| AppError(ApiError::BadRequest(format!("非自定义槽位: {rel}"))))?;
    let dir = task_exec::harness_custom_dir();
    let mut state = task_exec::read_custom_state(&dir);
    state[key] = serde_json::Value::Bool(enabled);
    task_exec::write_custom_state(&dir, &state)?;
    let fresh = task_exec::load_harness()?;
    *st.harness.write().await = fresh;
    info!("[harness] 自定义槽 {} 开关 -> {}", rel, enabled);
    Ok(Json(serde_json::json!({ "success": true, "data": { "path": rel, "enabled": enabled } })).into_response())
}

/// 槽位键校验：只收 CUSTOM_SLOTS 白名单文件名，拒绝一切路径形态
fn slot_key_of(rel: &str) -> Option<&'static str> {
    task_exec::CUSTOM_SLOTS.iter().find(|(f, _)| f == &rel).map(|(_, k)| *k)
}

/// custom 槽路径：白名单文件名直接拼目录（无相对路径游戏空间）
fn custom_slot_path(rel: &str) -> Result<std::path::PathBuf, AppError> {
    if slot_key_of(rel).is_none() {
        return Err(AppError(ApiError::BadRequest(format!("非自定义槽位（仅允许 {:?}）", task_exec::CUSTOM_SLOTS.iter().map(|(f, _)| f).collect::<Vec<_>>()))));
    }
    Ok(task_exec::harness_custom_dir().join(rel))
}

/// 示例模板（方案 v2 §6：创建槽时预填进编辑器；不落盘、不保存不生效）
const TEMPLATE_GLOBAL: &str = r#"# 团队通用补充规则（global.md）
# 追加在出厂规则之后注入所有 agent 上下文；与出厂规则冲突时以本文为准。

## 代码审查追加维度
1. 安全：SQL 注入、鉴权遗漏、敏感信息硬编码必须逐项排查。
2. 质量：所有公开函数必须有文档注释；编译器/linter 警告一律修复，不允许静默放行。

## 通用工作约定
3. 产物中的统计/验证结论必须用工具核实，禁止估算。
4. 方案必须包含「性能影响」与「回滚方式」两节。

（以上是示例模板——按需删减，保存后才会生效）
"#;

const TEMPLATE_DEVELOPMENT: &str = r#"# 团队开发流程补充规则（rule_development.md）
# 追加在出厂 rule_development.md 之后；仅对 development 流程（需求/方案/实施/审查）生效。

## 代码审查追加维度
1. 安全：SQL 注入、鉴权遗漏、敏感信息硬编码必须逐项排查。
2. 质量：所有公开函数必须有文档注释；编译器/linter 警告一律修复。

## 方案设计追加要求
3. 方案文档必须包含「性能影响」与「回滚方式」两节。
4. 涉及数据库变更必须给出迁移脚本与回滚脚本。

（以上是示例模板——按需删减，保存后才会生效）
"#;

pub(crate) async fn get_custom_template(
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let slot = q.get("slot").cloned().unwrap_or_default();
    let content = match slot.as_str() {
        "global" => TEMPLATE_GLOBAL,
        "development" => TEMPLATE_DEVELOPMENT,
        _ => return Err(AppError(ApiError::BadRequest(format!("未知槽位: {slot}")))),
    };
    Ok(Json(serde_json::json!({ "success": true, "data": { "slot": slot, "content": content } })).into_response())
}

/// AI 辅助生成（方案 v2 §2/§5）：自然语言需求 → LLM 起草 markdown 填入编辑器。
/// 纪律与示例模板一致：生成不落盘、不保存不生效。LLM 不可用 → 503 友好文案。
pub(crate) async fn generate_custom(
    State(st): State<AppState>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let slot = body.get("slot").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let description = body.get("description").and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();
    if description.is_empty() {
        return Err(AppError(ApiError::BadRequest("请描述你的团队规则需求".into())));
    }
    let (slot_name, where_text, factory_rule) = match slot.as_str() {
        "global" => ("global.md", "全部 agent 上下文（任务流水线、归纳、巡检、入口对话）", "inject-prompt.md（透明执行框架）"),
        "analysis" => ("rule_analysis.md", "需求分析阶段（阶段 1 需求矩阵 agent）", "rule_development.md（开发流程规则）"),
        "design" => ("rule_design.md", "方案设计阶段（阶段 2 方案设计 agent）", "rule_development.md（开发流程规则）"),
        "implement" => ("rule_implement.md", "代码开发阶段（阶段 3 实施 agent）", "rule_development.md（开发流程规则）"),
        "review" => ("rule_review.md", "代码审查（独立审查 agent 与阶段产物初审）", "rule_development.md / rule_bugfix.md（审查依据规则）"),
        _ => return Err(AppError(ApiError::BadRequest(format!("未知槽位: {slot}")))),
    };
    let cfg = resolve_llm(&st, "", "chat").await;
    if !matches!(*st.llm_mode, crate::state::LlmMode::Anthropic) || cfg.api_key.is_empty() {
        return Err(AppError(ApiError::Internal("LLM 未配置（设置面板或 EASYVIBE_LLM_API_KEY）——AI 生成不可用".into())));
    }
    let system = "你是 EasyVibe 的 harness 自定义规则起草助手。只输出 Markdown 正文，不要输出解释、前言或代码围栏。";
    let user = format!(
        "用户想为自己的团队定制 EasyVibe harness 自定义补充规则（槽位 {slot_name}）。\n\
         该槽将追加在出厂规则（{factory_rule}）之后注入：{where_text}。\n\
         补充规则与出厂规则冲突时以补充为准——只写「追加的要求」，不要重复出厂已有的流程规定。\n\
         用户需求描述：\n{description}\n\n\
         请起草一份简洁、可执行的 Markdown 补充规则（条款化，5-10 条为宜）。",
    );
    let llm = easyvibe_ai_agent::AnthropicClient::new(&cfg.base_url, &cfg.api_key, &cfg.model);
    let outcome = easyvibe_ai_agent::LlmClient::chat(
        &llm,
        easyvibe_ai_agent::ChatRequest { system, user: &user, images: &[] },
    )
    .await
    .map_err(|e| AppError(ApiError::Internal(format!("LLM 生成失败: {e}"))))?;
    let text = outcome.text.trim().to_string();
    if text.is_empty() {
        return Err(AppError(ApiError::Internal("LLM 返回为空，请重试或改用示例模板".into())));
    }
    info!("[harness] AI 生成槽 {} 草稿（{} tokens 出文）", slot_name, outcome.completion_tokens);
    Ok(Json(serde_json::json!({ "success": true, "data": { "slot": slot, "content": text } })).into_response())
}

// ---------- M3-2：指哪打哪——任务创建（上下文已组织好随表单提交；执行引擎 M3-3 接入） ----------
