//! 资源域：对话/会话/压缩/视图。
//!
//! R2 批 B：业务编排已抽至 `crate::service`，本文件只做 HTTP 边界
//! （解析入参 → 调 service → 映射错误/响应）。

use crate::state::*;
use crate::service::{self, resolve_adv_i64, resolve_conv, conversation_summary, maybe_compact, ChatHttpRequest, DEFAULT_CONTEXT_BUDGET};
use axum::{
    extract::{Path, State},
    routing::{delete, get, post, put},
    response::{IntoResponse, Response},
    Json, Router,
};
use easyvibe_common::ApiError;
use easyvibe_db::{ConversationRepository as _, TaskRepository as _};

/// 本域路由（R1 自注册）：对话/会话/压缩/视图。
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/repos/{id}/chat", get(get_chat).post(chat))
        .route("/repos/{id}/conversations", get(list_conversations).post(create_conversation))
        .route("/repos/{id}/conversations/{cid}", put(rename_conversation).delete(delete_conversation))
        .route("/repos/{id}/chat/compact", post(compact_chat))
        .route("/repos/{id}/chat/reset", post(reset_chat))
        .route("/repos/{id}/views", get(list_views).post(save_view))
        .route("/repos/{id}/views/{slug}", delete(delete_view).put(rename_view))
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

/// 入口对话（M2-5 + M3-5 持久化）：HTTP 边界；编排在 `service::chat_once`。
pub(crate) async fn chat(State(st): State<AppState>, Path(id): Path<String>, Json(body): Json<ChatHttpRequest>) -> Result<Response, AppError> {
    let data = service::chat_once(&st, &id, body).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": data })).into_response())
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
