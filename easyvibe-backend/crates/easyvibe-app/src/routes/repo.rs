//! 资源域：仓库注册/注销与健康（含 git 子资源路由注册）。

use crate::state::*;
use crate::pipeline::spawn_repo_pipeline;
use crate::git;
use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    response::{IntoResponse, Response},
    Json, Router,
};
use easyvibe_api_types::{HealthResponse, RepoInfo};
use easyvibe_common::{ApiError, ApiResponse};
use easyvibe_map::repo_from_root;
use tracing::info;
use crate::VERSION;
use crate::state::{read_desktop_repos, write_desktop_repos};

/// 本域路由（R1 自注册）：仓库注册/注销/健康 + git 子资源。
/// git handler 留 `crate::git`（仅注册不搬实现，见方案 §2.1.3(d) 决策 A）。
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route("/repos", get(list_repos).post(add_repo))
        .route("/repos/{id}", delete(remove_repo))
        .route("/repos/{id}/git/status", get(git::get_git_status))
        .route("/repos/{id}/git/log", get(git::get_git_log))
        .route("/repos/{id}/git/commit", get(git::get_git_commit))
        .route("/repos/{id}/git/commit", post(git::post_git_commit))
        .route("/repos/{id}/git/pull", post(git::post_git_pull))
        .route("/repos/{id}/git/push", post(git::post_git_push))
        .route("/repos/{id}/git/discard", post(git::post_git_discard))
        .route("/repos/{id}/git/commit-message", post(git::post_git_commit_message))
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

// ---------- D5 应用内仓库管理：动态注册/注销（desktop-repos 文件由后端独占） ----------

#[derive(serde::Deserialize)]
pub(crate) struct AddRepoRequest {
    path: String,
}

pub(crate) async fn add_repo(State(st): State<AppState>, Json(body): Json<AddRepoRequest>) -> Result<Response, AppError> {
    let root = std::path::PathBuf::from(body.path.trim());
    if !root.is_dir() {
        return Err(AppError(ApiError::BadRequest(format!("目录不存在或不可读: {}", root.display()))));
    }
    let repo = repo_from_root(&root);
    st.map_service.add_repo(repo.clone()).await.map_err(AppError)?;
    // 持久化 + 启动该仓库的 watcher 管线（自动归纳由管线内决定）
    let mut roots = read_desktop_repos();
    if !roots.iter().any(|p| p == &root) {
        roots.push(root.clone());
        write_desktop_repos(&roots);
    }
    tokio::spawn(spawn_repo_pipeline(
        repo.clone(),
        st.map_service.clone(),
        st.event_bus.clone(),
        st.session_manager.clone(),
        (*st.prompt_template).clone(),
        (*st.agent_command).clone(),
        (*st.agent_args).clone(),
        st.harness.clone(),
    ));
    info!("[repo-add] 动态注册 {} -> {}", repo.id, repo.root.display());
    Ok(Json(serde_json::json!({ "success": true, "data": { "id": repo.id, "name": repo.name, "root": repo.root.to_string_lossy() } })).into_response())
}

pub(crate) async fn remove_repo(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未挂载")))?;
    // 重审 P1：注销前杀活动会话——此前 running 的归纳/任务 agent 成孤儿，
    // 仓库写互斥被占死，进程只能等超时或后端退出（kill_on_drop）才释放
    if let Some(s) = st.session_manager.status_of(&id).await {
        if matches!(s.status, easyvibe_api_types::SessionStatus::Starting | easyvibe_api_types::SessionStatus::Running) {
            let _ = st.session_manager.kill(&s.session_id).await;
        }
    }
    // 任务会话兜底（active 槽位只记最后一个注册者；running/awaiting 的任务逐个点杀，
    // 已终态会话 kill 返回 409 属预期，忽略）
    use easyvibe_db::TaskRepository as _;
    if let Ok(tasks) = st.task_repo.list(&id, 500).await {
        for t in tasks.into_iter().filter(|t| matches!(t.status.as_str(), "running" | "awaiting_approval")) {
            if let Some(sid) = t.session_id {
                let _ = st.session_manager.kill(&sid).await;
            }
        }
    }
    // S1：顺带清该仓库队列项（kill 产生的终态事件会对已注销仓库触发 drain 空跑；
    // 广播 cancelled 让前端气泡即时清态）。解环后由 QueueState::cancel 完成「移除 + 广播」。
    let _ = st.session_queue.cancel(&st, &id).await;
    // 数据清除（?wipe=true）：抹掉该仓库在本地库的全部痕迹
    // （任务/审批/会话/消息/巡检历史/事件/仓库级设置）——默认保留，用户显式选择才清
    let wiped = if q.get("wipe").map(|v| v == "true").unwrap_or(false) {
        easyvibe_db::wipe_repo(&st.pool, &id).await?
    } else {
        0
    };
    // 先取根再注销（注销后 find_repo 即查不到）
    let root = st.map_service.find_repo(&id).await.map(|r| r.root);
    if !st.map_service.remove_repo(&id).await {
        return Err(AppError(ApiError::NotFound(format!("仓库 {id} 未挂载"))));
    }
    // 重审 P2 实弹 bug：remaining 从"本进程已挂载"算——dev 后端（env 只有 FENJUE）
    // 注销 FENJUE 会写出空文件，把 App 后端的 hover-client 行一并抹掉（桌面端"仓库消失"）
    // 持久化文件是跨进程共享事实源，必须以文件为基准：读文件 → 删被注销的根 → 写回
    let mut roots = read_desktop_repos();
    if let Some(root) = root {
        roots.retain(|p| p != &root);
    }
    write_desktop_repos(&roots);
    info!("[repo-remove] 注销 {}（数据清除 {} 行，文件剩 {} 个仓库）", id, wiped, roots.len());
    Ok(Json(serde_json::json!({ "success": true, "data": { "wiped": wiped } })).into_response())
}
