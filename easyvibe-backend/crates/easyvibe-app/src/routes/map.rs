//! 资源域：地图/子图/成长/保鲜/进度/归纳/巡检（HTTP 边界）。
//!
//! 方案 R2：业务编排已下沉 `crate::service::map`，本文件只做入参解析、
//! ETag/304 头、状态码与响应包装、`AppError` 映射。

use crate::state::*;
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, post},
    response::{IntoResponse, Response},
    Json, Router,
};

/// 本域路由（R1 自注册）：地图读取/子图/成长/保鲜/进度/归纳/巡检。
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/repos/{id}/map", get(get_map))
        .route("/repos/{id}/freshness", get(get_freshness))
        .route("/repos/{id}/modules/{module_id}/health-history", get(get_health_history))
        .route("/repos/{id}/modules/{module_id}/analyze-submap", post(analyze_submap))
        .route("/repos/{id}/growth", get(get_growth))
        .route("/repos/{id}/progress", get(get_progress))
        .route("/repos/{id}/modules/{module_id}", get(get_submap))
        .route("/repos/{id}/reinduce", post(start_reinduce))
        .route("/repos/{id}/patrol", post(start_patrol))
}

pub(crate) async fn get_map(State(st): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Result<Response, AppError> {
    // Y1 清债：ETag 条件请求——内容哈希已有，浏览器重连重同步时 If-None-Match 命中即 304
    // （body 不传输；前端零改动，浏览器 HTTP 缓存自动处理）
    let (body, etag) = crate::service::map::load_map_with_etag(&st, &id).await?;
    if headers.get("if-none-match").and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        let mut resp = axum::http::Response::new(axum::body::Body::empty());
        *resp.status_mut() = axum::http::StatusCode::NOT_MODIFIED;
        resp.headers_mut().insert("etag", etag.parse().unwrap());
        return Ok(resp.into_response());
    }
    let mut resp = Json(body).into_response();
    resp.headers_mut().insert("etag", etag.parse().unwrap());
    resp.headers_mut().insert("cache-control", "private, must-revalidate".parse().unwrap());
    Ok(resp)
}

pub(crate) async fn get_growth(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    // 原样返回 growth.log 事件数组（与文件行一致，前端状态机统一消费）
    Ok(Json(crate::service::map::load_growth(&st, &id).await?).into_response())
}

pub(crate) async fn get_submap(
    State(st): State<AppState>,
    Path((id, module_id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    Ok(Json(crate::service::map::load_submap(&st, &id, &module_id).await?).into_response())
}

/// S2：地图保鲜状态（§13.4——stale 地图上的对话/建议/健康分全是假数据自信工作）
pub(crate) async fn get_freshness(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let data = crate::service::map::assess_freshness(&st, &id).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": data })).into_response())
}

/// S2：模块健康历史（趋势图数据面；module_health_history 自 M2-4 落库以来的第一个消费者）
pub(crate) async fn get_health_history(State(st): State<AppState>, Path((id, module_id)): Path<(String, String)>) -> Result<Response, AppError> {
    let data = crate::service::map::list_health_history(&st, &id, &module_id).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": data })).into_response())
}

/// S2.5：归纳进度（progress.json 透传——首归纳等待页显示真实阶段/百分比，
/// 不再只转圈；文件缺失（如巡检场景无 progress）返回 null 形状，前端不渲染进度）
pub(crate) async fn get_progress(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let data = crate::service::map::load_progress(&st, &id).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": data })).into_response())
}

/// 子图深入分析（试用反馈"子图加载失败"根因修复——v2.2 归纳不产子图，文件无人生产；
/// 此处把缺口变为能力：透明 agent 扫描模块文件产出子图，落盘 .easyvibe/modules/<id>.json，
/// 与归纳共用写互斥/会话机制。读时拉取无需 watcher，产出后重新展开即见）
pub(crate) async fn analyze_submap(State(st): State<AppState>, Path((id, module_id)): Path<(String, String)>) -> Result<Response, AppError> {
    let session = crate::service::map::analyze_submap(&st, &id, &module_id).await?;
    Ok((axum::http::StatusCode::ACCEPTED, Json(session)).into_response())
}

/// 触发重新归纳（写路径，M2-3）：spawn 外部 agent 按 v2.2 协议执行，
/// 三通道（progress/growth.log/map.json）由 watcher 自动直播，前端无需轮询
pub(crate) async fn start_reinduce(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let session = crate::service::map::start_reinduce(&st, &id, false).await?;
    Ok((axum::http::StatusCode::ACCEPTED, Json(session)).into_response())
}

/// 触发巡检（M2-4，实弹 #2 修订）：两条执行路径（Stub / Anthropic）编排在 service。
pub(crate) async fn start_patrol(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let body = crate::service::map::start_patrol(&st, &id).await?;
    Ok((axum::http::StatusCode::ACCEPTED, Json(body)).into_response())
}
