//! 资源域：任务 CRUD/决策/重试/复审/回看/审批/diff/建议。
//!
//! c-arch-7 R1：业务编排与仓储调用已下沉 `crate::service::task`，本文件只做 HTTP 边界
//! （解析入参 → 调 service → 映射状态码/响应），零 DB 直连、零组合根仓储句柄直取。

use crate::state::*;
use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    response::{IntoResponse, Response},
    Json, Router,
};
use easyvibe_common::{ApiError, ApiResponse};
use crate::service::{self, CreateTaskRequest};

/// 本域路由（R1 自注册）：任务 CRUD/决策/重试/复审/回看/审批/diff/建议。
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/repos/{id}/tasks", get(list_tasks).post(create_task))
        .route("/repos/{id}/tasks/{tid}", delete(delete_task))
        .route("/repos/{id}/tasks/{tid}/decide", post(decide_task))
        .route("/repos/{id}/tasks/{tid}/retry", post(post_task_retry))
        .route("/repos/{id}/tasks/{tid}/remediate", post(post_task_remediate))
        .route("/repos/{id}/tasks/{tid}/rewind", post(post_task_rewind))
        .route("/repos/{id}/tasks/{tid}/review", post(post_task_review))
        .route("/repos/{id}/tasks/{tid}/approvals", get(list_task_approvals))
        .route("/repos/{id}/tasks/{tid}/diff", get(get_task_diff))
        .route("/repos/{id}/tasks/{tid}/kill", post(post_task_kill))
        .route("/repos/{id}/suggest", post(suggest))
}

/// 按任务终止：会话 kill（任务卡「终止」按钮走这里）。
pub(crate) async fn post_task_kill(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    service::task::kill_task(&st, &id, &tid).await?;
    Ok(Json(ApiResponse::ok(true)).into_response())
}

/// 管理闭环（2026-10-03 现状重审 P0）：删除任务（级联清理由 service 编排）。
pub(crate) async fn delete_task(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    service::task::delete_task(&st, &id, &tid).await?;
    Ok(Json(ApiResponse::ok(true)).into_response())
}

/// 就地重试：failed/interrupted → pending 重新入队（见 TaskExecutor::retry 的语义注释）
pub(crate) async fn post_task_retry(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    let out = service::task::retry_task(&st, &id, &tid).await?;
    Ok(Json(ApiResponse::ok(out)).into_response())
}

/// 修改并复审：子 agent 审查打回（rejected）→ 注入审查意见 → 直达实施阶段重跑 →
/// 完成后子 agent 自动复审（见 TaskExecutor::remediate 的语义注释）
pub(crate) async fn post_task_remediate(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    let out = service::task::remediate_task(&st, &id, &tid).await?;
    Ok(Json(ApiResponse::ok(out)).into_response())
}

/// 管道回看·节点重开（方案 §3.1）：body `{gate: "analysis"|"solution"}`——
/// 放回目标评审关（见 TaskExecutor::rewind 的语义注释）；rewind 自身已广播，此处不再双发
pub(crate) async fn post_task_rewind(
    State(st): State<AppState>,
    Path((id, tid)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let target = body["gate"].as_str().ok_or_else(|| ApiError::BadRequest("缺少 gate 字段（analysis/solution）".into()))?;
    let out = service::task::rewind_task(&st, &id, &tid, target).await?;
    Ok(Json(ApiResponse::ok(out)).into_response())
}

/// 人工触发子 agent 复审（代码审查节点的审查-修复闭环）：异步执行，结论经
/// result.review + 留痕 + 事件送达（见 TaskExecutor::review_now 的语义注释）
pub(crate) async fn post_task_review(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    let out = service::task::review_task(&st, &id, &tid).await?;
    Ok((axum::http::StatusCode::ACCEPTED, Json(ApiResponse::ok(out))).into_response())
}

/// 审批决策（M3-4）：approved/rejected 按当前关卡推进或终止；发射 task.statusChanged
#[derive(serde::Deserialize)]
pub(crate) struct DecideRequest {
    decision: String, // approved / rejected
    #[serde(default)]
    note: Option<String>,
    /// N27：用户所见关卡（防双击穿透——任务已推进后，针对旧关卡的重复 decide 必须 409）。
    /// 缺省回退服务端当前关卡（兼容旧客户端）。
    #[serde(default)]
    gate: Option<String>,
}

pub(crate) async fn decide_task(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>, Json(body): Json<DecideRequest>) -> Result<Response, AppError> {
    let out = service::task::decide_task(&st, &id, &tid, &body.decision, body.note.as_deref(), body.gate.as_deref()).await?;
    Ok(Json(ApiResponse::ok(out)).into_response())
}

pub(crate) async fn list_task_approvals(State(st): State<AppState>, Path((_, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    Ok(Json(service::task::list_task_approvals(&st, &tid).await?).into_response())
}

/// M4-3：任务完整 diff（按需读取，不随任务列表载荷）——development_docs 归档中的 diffFull；
/// 无归档/无 diff 返回 diff=null（调用方展示"无变更"）
pub(crate) async fn get_task_diff(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    Ok(Json(service::task::task_diff(&st, &id, &tid).await?).into_response())
}

/// 智能优化建议：AI 主动发现优化机会（Stub=确定性派生；LLM=地图注入生成），
/// 每条建议可一键转修复任务（前端组装 TaskDraft）
pub(crate) async fn suggest(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    Ok(Json(service::task::suggest_tasks(&st, &id).await?).into_response())
}

/// 建任务（M3-3 入队）：HTTP 边界；编排在 `service::create_task`。
pub(crate) async fn create_task(State(st): State<AppState>, Path(id): Path<String>, Json(body): Json<CreateTaskRequest>) -> Result<Response, AppError> {
    let task_id = service::create_task(&st, &id, body).await?;
    Ok((axum::http::StatusCode::CREATED, Json(serde_json::json!({ "success": true, "data": { "id": task_id } }))).into_response())
}

pub(crate) async fn list_tasks(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    // M4-2：?conv=<id> 时会话级过滤（工作台影响面/计划进度）
    // 重审 P1：?limit= 可调（默认 50 是"任务一多旧任务消失"的失控感来源），上限 500 防全表拉取
    let limit: i64 = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50);
    let body = service::task::list_tasks(&st, &id, q.get("conv").map(|v| v.as_str()), limit).await?;
    Ok(Json(body).into_response())
}

// ---------- M3-5：会话持久化 + auto-compact（backend-design §10a / §11 🟡4/5/6） ----------
