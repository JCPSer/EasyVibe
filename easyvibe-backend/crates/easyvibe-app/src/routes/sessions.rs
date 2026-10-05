//! 资源域：运行会话/用量/健康看板/使用证据埋点。

use crate::state::*;
use crate::session_queue_routes;
use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    response::{IntoResponse, Response},
    Json, Router,
};
use easyvibe_common::{ApiError, ApiResponse};
use tracing::info;

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
    use easyvibe_db::EventRepository as _;
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let name = body["name"].as_str().unwrap_or_default().trim();
    if name.is_empty() || name.len() > 64 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-') {
        return Err(AppError(ApiError::BadRequest("事件名须为 dot.case（字母数字._-，≤64）".into())));
    }
    let payload = body["payload"].as_object().map(|_| body["payload"].to_string()).unwrap_or_else(|| "{}".into());
    st.event_repo.record(&repo.id, name, &payload).await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

/// R3 D1：门控读数——按事件名计数（L3 三道门、Harness 验证指标的秤）
pub(crate) async fn events_summary(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    use easyvibe_db::EventRepository as _;
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let rows = st.event_repo.summary(&repo.id).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": rows })).into_response())
}

pub(crate) async fn get_usage(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<UsageQuery>,
) -> Result<Response, AppError> {
    let days = q.days.unwrap_or(30).clamp(0, 36500);
    let since = if days == 0 {
        "1970-01-01T00:00:00Z".to_string() // 全部
    } else {
        (chrono::Utc::now() - chrono::Duration::days(days.into())).to_rfc3339()
    };
    let (totals, daily, by_kind, by_model, by_module, sessions) = tokio::join!(
        st.agent_session_repo.usage_totals(&id, &since),
        st.agent_session_repo.usage_daily(&id, &since),
        st.agent_session_repo.usage_by_kind(&id, &since),
        st.agent_session_repo.usage_by_model(&id, &since),
        st.agent_session_repo.usage_by_module(&id, &since),
        st.agent_session_repo.list(&id, 50),
    );
    let body = serde_json::json!({
        "success": true,
        "data": {
            "since": since,
            "totals": totals.map_err(AppError::from)?,
            "daily": daily.map_err(AppError::from)?,
            "byKind": by_kind.map_err(AppError::from)?,
            "byModel": by_model.map_err(AppError::from)?,
            "byModule": by_module.map_err(AppError::from)?,
            "sessions": sessions.map_err(AppError::from)?,
        }
    });
    Ok(Json(body).into_response())
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
    let rows = st
        .session_output_repo
        .fetch_after(&sid, q.after_seq.unwrap_or(0), q.limit.unwrap_or(5000).min(5000))
        .await?;
    Ok(Json(serde_json::json!({ "success": true, "data": rows })).into_response())
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
    let rows = st.agent_session_repo.list(&id, 50).await?;
    Ok(Json(ApiResponse::ok(rows)).into_response())
}

pub(crate) async fn list_patrol_runs(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    use easyvibe_db::HealthRepository as _;
    let runs = st.health_repo.list_runs(&id, 20).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": runs })).into_response())
}

/// 巡检历史清理（重审 P1：两表只增不减，自动巡检开启后 module_health_history
/// 行量 = 模块数 × 巡检次数）。DELETE /patrol-runs?keep=N——只留最近 N 次已终态
/// 巡检，running 的永不进删除集。默认 keep=10，上限 200（防误传超大值）。
pub(crate) async fn prune_patrol_runs(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    use easyvibe_db::HealthRepository as _;
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let keep: i64 = q.get("keep").and_then(|v| v.parse().ok()).unwrap_or(10).clamp(0, 200);
    let deleted = st.health_repo.prune_runs(&id, keep).await?;
    info!("[patrol-prune] {} 清理历史巡检 {} 条（保留最近 {} 次）", id, deleted, keep);
    Ok(Json(serde_json::json!({ "success": true, "data": { "deleted": deleted, "keep": keep } })).into_response())
}

/// M4-3 健康看板数据面：近 20 次巡检（含各自模块平均分）+ 最近一次成功巡检的模块明细。
/// 一次聚合查询代替前端 N×M 次 health-history 轮询（N 模块 × M 次巡检）。
pub(crate) async fn get_health_dashboard(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    use easyvibe_db::HealthRepository as _;
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let runs = st.health_repo.list_runs(&id, 20).await?;
    let avgs = st.health_repo.list_run_averages(&id, 20).await?;
    let avg_of: std::collections::HashMap<String, (i64, i64)> = avgs
        .iter()
        .map(|a| (a.run_id.clone(), (a.module_avg, a.module_count)))
        .collect();
    let runs_json: Vec<serde_json::Value> = runs
        .iter()
        .map(|r| {
            let (module_avg, module_count) = avg_of.get(&r.id).copied().unwrap_or((0, 0));
            serde_json::json!({
                "id": r.id,
                "startedAt": r.started_at,
                "finishedAt": r.finished_at,
                "status": r.status,
                "model": r.model,
                "archScore": r.arch_score,
                "moduleAvg": module_avg,
                "moduleCount": module_count,
                // 2026-10-05 问题项新旧对照（仅 succeeded 巡检有值）
                "concernsDiff": r.concerns_diff.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
            })
        })
        .collect();
    let latest_modules = st.health_repo.list_latest_run_modules(&id).await?;
    Ok(Json(serde_json::json!({
        "success": true,
        "data": { "runs": runs_json, "latestModules": latest_modules },
    }))
    .into_response())
}

/// P0 审查后端#1：终止指定会话（归纳/巡检/子图分析/任务执行同一通道）。
/// 已终态返回 409；外部自注册会话（无终止通道）返回 409。
pub(crate) async fn post_session_kill(State(st): State<AppState>, Path((id, sid)): Path<(String, String)>) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    st.session_manager.kill(&sid).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": true })).into_response())
}
