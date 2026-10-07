//! 资源域：仓库注册/注销与健康（含 git 子资源路由注册）。
//!
//! 边界纪律（守卫 R5/R7）：本文件只做入参解析 + 调 `crate::service` + 状态码/响应映射；
//! git 领域规则在 `easyvibe-git`，编排在 `crate::service::git`，本文件零领域实现。

use crate::state::*;
use crate::service::git as git_service;
use crate::service::repo as repo_service;
use axum::{
    extract::{Path, Query, State},
    routing::{delete, get, post},
    response::{IntoResponse, Response},
    Json, Router,
};
use easyvibe_api_types::{HealthResponse, RepoInfo};
use easyvibe_common::{ApiError, ApiResponse};
use crate::VERSION;

/// 本域路由（R1 自注册）：仓库注册/注销/健康 + git 子资源。
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/repos", get(list_repos).post(add_repo))
        .route("/repos/{id}", delete(remove_repo))
        .route("/repos/{id}/git/status", get(get_git_status))
        .route("/repos/{id}/git/log", get(get_git_log))
        .route("/repos/{id}/git/commit", get(get_git_commit))
        .route("/repos/{id}/git/commit", post(post_git_commit))
        .route("/repos/{id}/git/diff", get(get_git_diff))
        .route("/repos/{id}/git/pull", post(post_git_pull))
        .route("/repos/{id}/git/push", post(post_git_push))
        .route("/repos/{id}/git/discard", post(post_git_discard))
        .route("/repos/{id}/git/commit-message", post(post_git_commit_message))
}

pub(crate) async fn health() -> Json<ApiResponse<HealthResponse>> {
    Json(ApiResponse::ok(HealthResponse { status: "ok".into(), version: VERSION.into() }))
}

pub(crate) async fn list_repos(State(st): State<AppState>) -> Json<ApiResponse<Vec<RepoInfo>>> {
    let repos = st
        .map_service
        .repos().await
        .into_iter()
        .map(|r| RepoInfo { id: r.id, name: r.name, root: r.root.to_string_lossy().into_owned() })
        .collect();
    Json(ApiResponse::ok(repos))
}

// ---------- D5 应用内仓库管理：动态注册/注销（编排在 crate::service::repo） ----------

#[derive(serde::Deserialize)]
pub(crate) struct AddRepoRequest {
    path: String,
}

pub(crate) async fn add_repo(State(st): State<AppState>, Json(body): Json<AddRepoRequest>) -> Result<Response, AppError> {
    let data = repo_service::add_repo(&st, &body.path).await?;
    Ok(Json(ApiResponse::ok(data)).into_response())
}

pub(crate) async fn remove_repo(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let wipe = q.get("wipe").map(|v| v == "true").unwrap_or(false);
    let data = repo_service::remove_repo(&st, &id, wipe).await?;
    Ok(Json(ApiResponse::ok(data)).into_response())
}

// ---------- git 子资源：入参解析 → crate::service::git → 响应映射 ----------

pub(crate) async fn get_git_status(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let data = git_service::git_status(&st, &id).await?;
    Ok(Json(ApiResponse::ok(data)).into_response())
}

pub(crate) async fn get_git_log(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let limit = q.get("limit").and_then(|l| l.parse::<i64>().ok()).unwrap_or(30).clamp(1, 100);
    let rows = git_service::git_log(&st, &id, limit).await?;
    Ok(Json(ApiResponse::ok(rows)).into_response())
}

pub(crate) async fn get_git_commit(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let hash = q.get("hash").ok_or_else(|| ApiError::BadRequest("缺少 hash 参数".into()))?;
    let detail = git_service::git_commit_detail(&st, &id, hash).await?;
    Ok(Json(ApiResponse::ok(detail)).into_response())
}

/// 单文件差异（diff 抽屉）：?file=<relpath> 必填，&staged=true 时对比暂存区 vs HEAD。
pub(crate) async fn get_git_diff(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let file = q.get("file").ok_or_else(|| ApiError::BadRequest("缺少 file 参数".into()))?;
    let staged = q.get("staged").map(|v| v == "true").unwrap_or(false);
    let data = git_service::git_diff(&st, &id, file, staged).await?;
    Ok(Json(ApiResponse::ok(data)).into_response())
}

pub(crate) async fn post_git_commit(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let message = body["message"].as_str().unwrap_or_default();
    let data = git_service::git_commit(&st, &id, message).await?;
    Ok(Json(ApiResponse::ok(data)).into_response())
}

pub(crate) async fn post_git_pull(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    git_service::git_pull(&st, &id).await?;
    Ok(Json(ApiResponse::ok(true)).into_response())
}

pub(crate) async fn post_git_push(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    git_service::git_push(&st, &id).await?;
    Ok(Json(ApiResponse::ok(true)).into_response())
}

pub(crate) async fn post_git_discard(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let path = body["path"].as_str().ok_or_else(|| ApiError::BadRequest("缺少 path".into()))?;
    // 重审 P2：全部撤销（path="*"）与逐文件撤销同一端点，语义由 path 值区分（见 service::git）。
    git_service::git_discard(&st, &id, path).await?;
    Ok(Json(ApiResponse::ok(true)).into_response())
}

pub(crate) async fn post_git_commit_message(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<git_service::CommitMessageRequest>,
) -> Result<Response, AppError> {
    let data = git_service::git_commit_message(&st, &id, body).await?;
    Ok(Json(ApiResponse::ok(data)).into_response())
}
