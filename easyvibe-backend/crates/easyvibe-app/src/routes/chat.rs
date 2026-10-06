//! 资源域：对话/会话/压缩/视图。
//!
//! R2 批 B + c-arch-7 R1：业务编排已抽至 `crate::service::chat`，本文件只做 HTTP 边界
//! （解析入参 → 调 service → 映射错误/响应），零 DB 直连、零组合根仓储句柄直取。

use crate::state::*;
use crate::service::{self, ChatHttpRequest};
use crate::service::chat::{CreateConversationRequest, RenameConversationRequest, RenameViewRequest, SaveViewRequest};
use axum::{
    extract::{Path, State},
    routing::{delete, get, post, put},
    response::{IntoResponse, Response},
    Json, Router,
};

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
    Ok(Json(service::chat::list_conversations(&st, &id).await?).into_response())
}

pub(crate) async fn create_conversation(State(st): State<AppState>, Path(id): Path<String>, Json(body): Json<CreateConversationRequest>) -> Result<Response, AppError> {
    Ok(Json(service::chat::create_conversation(&st, &id, body.title.as_deref()).await?).into_response())
}

pub(crate) async fn rename_conversation(State(st): State<AppState>, Path((id, cid)): Path<(String, String)>, Json(body): Json<RenameConversationRequest>) -> Result<Response, AppError> {
    service::chat::rename_conversation(&st, &id, &cid, &body.title).await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

pub(crate) async fn delete_conversation(State(st): State<AppState>, Path((id, cid)): Path<(String, String)>) -> Result<Response, AppError> {
    service::chat::delete_conversation(&st, &id, &cid).await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

pub(crate) async fn get_chat(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let before = q.get("before").and_then(|v| v.parse::<i64>().ok());
    let limit: i64 = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50);
    let body = service::chat::get_chat(&st, &id, q.get("conv").map(|v| v.as_str()), before, limit).await?;
    Ok(Json(body).into_response())
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
    let body = service::chat::compact_chat(&st, &id, q.get("conv").map(|v| v.as_str())).await?;
    Ok(Json(body).into_response())
}

/// 新对话：清空消息与摘要（会话行保留，token 计数归零）
pub(crate) async fn reset_chat(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    service::chat::reset_chat(&st, &id, q.get("conv").map(|v| v.as_str())).await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

/// 视图列表（F1b 读侧）：.easyvibe/views/*.json 引用式视图，供前端"视图"页签消费
pub(crate) async fn list_views(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    Ok(Json(service::chat::list_views(&st, &id).await?).into_response())
}

/// 删除视图（F1b 读侧闭环）：slug 复用保存时的安全字符集，防线同 save_view
pub(crate) async fn delete_view(State(st): State<AppState>, Path((id, slug)): Path<(String, String)>) -> Result<Response, AppError> {
    service::chat::delete_view(&st, &id, &slug).await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

/// 视图改名（重审 P1：此前只能删了重建——保存冲突还会静默另存新文件造成列表膨胀）。
/// 改名 = 新 slug 写文件 + 删旧文件；slug 冲突 409（同 slug = 纯改名，原地更新 name 字段）。
pub(crate) async fn rename_view(State(st): State<AppState>, Path((id, slug)): Path<(String, String)>, Json(body): Json<RenameViewRequest>) -> Result<Response, AppError> {
    let body = service::chat::rename_view(&st, &id, &slug, &body.name).await?;
    Ok(Json(body).into_response())
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
    let force = q.get("force").map(|v| v == "true").unwrap_or(false);
    let out = service::chat::save_view(&st, &id, body, force).await?;
    Ok((axum::http::StatusCode::CREATED, Json(out)).into_response())
}
