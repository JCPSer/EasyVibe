//! 资源域：运行会话/用量/健康看板/使用证据埋点。
//!
//! c-arch-7 R1：业务编排与仓储调用已下沉 `crate::service::sessions`，本文件只做 HTTP 边界
//! （Path/Query 解析 → 调 service → 映射响应），零 DB 直连、零组合根仓储句柄直取。

use crate::state::*;
use crate::session_queue_routes;
use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    response::{IntoResponse, Response},
    Json, Router,
};

/// 本域路由（R1 自注册）：运行会话/队列/用量/健康看板/使用证据埋点/巡检历史。
/// session-queue 与 sessions/overview 的 handler 留 `crate::session_queue_routes`（仅注册）。
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/repos/{id}/session-queue",
            get(session_queue_routes::get_session_queue)
                .post(session_queue_routes::post_session_queue)
                .delete(session_queue_routes::delete_session_queue),
        )
        .route("/repos/{id}/patrol-runs", get(list_patrol_runs).delete(prune_patrol_runs))
        .route("/sessions/overview", get(session_queue_routes::get_sessions_overview))
        .route("/repos/{id}/agent-sessions", get(list_agent_sessions))
        .route("/repos/{id}/usage", get(get_usage))
        .route("/repos/{id}/health-dashboard", get(get_health_dashboard))
        .route("/repos/{id}/events", post(ingest_event))
        .route("/repos/{id}/events/summary", get(events_summary))
        .route("/repos/{id}/sessions/{sid}/kill", post(post_session_kill))
        .route("/repos/{id}/sessions/{sid}/output", get(get_session_output))
}

/// 健康历史：巡检运行列表（域 2 的第一个读接口）
/// R3 D1：前端交互埋点入库（dot.case 事件名 + JSON 计数维度；服务端事件由总线持久化任务代写）
pub(crate) async fn ingest_event(State(st): State<AppState>, Path(id): Path<String>, Json(body): Json<serde_json::Value>) -> Result<Response, AppError> {
    Ok(Json(crate::service::sessions::ingest_event(&st, &id, body).await?).into_response())
}

/// R3 D1：门控读数——按事件名计数（L3 三道门、Harness 验证指标的秤）
pub(crate) async fn events_summary(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    Ok(Json(crate::service::sessions::events_summary(&st, &id).await?).into_response())
}

pub(crate) async fn get_usage(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<UsageQuery>,
) -> Result<Response, AppError> {
    Ok(Json(crate::service::sessions::get_usage(&st, &id, q.days).await?).into_response())
}

#[derive(serde::Deserialize)]
pub(crate) struct UsageQuery {
    days: Option<i64>,
}

/// M2：会话输出回放/补拉——afterSeq 之后的行（升序，上限 5000）
pub(crate) async fn get_session_output(
    State(st): State<AppState>,
    Path((_repo, sid)): Path<(String, String)>,
    Query(q): Query<OutputQuery>,
) -> Result<Response, AppError> {
    Ok(Json(crate::service::sessions::get_session_output(&st, &sid, q.after_seq, q.limit).await?).into_response())
}

#[derive(serde::Deserialize)]
pub(crate) struct OutputQuery {
    after_seq: Option<i64>,
    limit: Option<i64>,
}

/// M1/U1：会话历史（运行页历史回放 / 用量页统计的数据源；M2 输出落盘前的元数据层）
pub(crate) async fn list_agent_sessions(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    Ok(Json(crate::service::sessions::list_agent_sessions(&st, &id).await?).into_response())
}

pub(crate) async fn list_patrol_runs(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    Ok(Json(crate::service::sessions::list_patrol_runs(&st, &id).await?).into_response())
}

/// 巡检历史清理（重审 P1：两表只增不减，自动巡检开启后 module_health_history
/// 行量 = 模块数 × 巡检次数）。DELETE /patrol-runs?keep=N——只留最近 N 次已终态
/// 巡检，running 的永不进删除集。默认 keep=10，上限 200（防误传超大值）。
pub(crate) async fn prune_patrol_runs(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let keep: i64 = q.get("keep").and_then(|v| v.parse().ok()).unwrap_or(10);
    Ok(Json(crate::service::sessions::prune_patrol_runs(&st, &id, keep).await?).into_response())
}

/// M4-3 健康看板数据面：近 20 次巡检（含各自模块平均分）+ 最近一次成功巡检的模块明细。
/// 一次聚合查询代替前端 N×M 次 health-history 轮询（N 模块 × M 次巡检）。
pub(crate) async fn get_health_dashboard(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    Ok(Json(crate::service::sessions::get_health_dashboard(&st, &id).await?).into_response())
}

/// P0 审查后端#1：终止指定会话（归纳/巡检/子图分析/任务执行同一通道）。
/// 已终态返回 409；外部自注册会话（无终止通道）返回 409。
pub(crate) async fn post_session_kill(State(st): State<AppState>, Path((id, sid)): Path<(String, String)>) -> Result<Response, AppError> {
    crate::service::sessions::kill_session(&st, &id, &sid).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": true })).into_response())
}
