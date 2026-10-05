//! 运行会话排队的**请求/响应面**（需求方案「运行会话气泡 + 单会话排队 v1」§5/§8）。
//!
//! 解环拆分：本模块只承载 HTTP handler、请求校验（`EnqueueBody`/`build_job`）与
//! 错误→HTTP 映射，归 server-api；队列状态机（`QueueState`/`QueuedJob`/`QueueChange`）
//! 已下沉 `easyvibe-event-bus`，执行入口经 `QueueHost`（AppState 实现）回调注入。
//! 因此本文件不出现任何 `crate::start_*_inner` 式反向引用。

use axum::response::IntoResponse;
use chrono::Utc;
use easyvibe_api_types::SessionStatus;
use easyvibe_common::ApiError;
use easyvibe_event_bus::queue::{JobKind, QueueHost, QueuedJob};

use crate::{AppError, AppState};

/// GET /repos/{id}/session-queue（§5）：active 来自 status_of + I1 旁路表，queued 来自槽位
pub(crate) async fn get_session_queue(
    axum::extract::State(st): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<axum::response::Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let active = match st.session_manager.status_of(&id).await {
        Some(s) if matches!(s.status, SessionStatus::Starting | SessionStatus::Running) => {
            let label = st
                .session_manager
                .label_of(&s.session_id)
                .await
                .unwrap_or_else(|| format!("会话 {}", s.session_id));
            serde_json::json!({
                "sessionId": s.session_id,
                "label": label,
                "status": format!("{:?}", s.status).to_lowercase(),
                "startedAt": st.session_manager.started_at_of(&s.session_id).await,
            })
        }
        // 无活动会话（或 active 残留终态）→ null
        _ => serde_json::Value::Null,
    };
    let queued = st.session_queue.peek(&id).await.map(|j| {
        serde_json::json!({
            "kind": j.kind.as_str(),
            "label": j.label,
            "moduleId": j.module_id,
            "enqueuedAt": j.enqueued_at,
        })
    });
    Ok(axum::Json(serde_json::json!({ "success": true, "data": { "active": active, "queued": queued } })).into_response())
}

/// GET /sessions/overview（2026-10-05 全局运行指示）：跨仓库的活动会话 + 排队任务。
/// 单仓库互斥但跨仓库并行合法（用户明示接受并发 agent）——切到 B 发起分析时
/// A 的会话必须全局可见，否则多仓库用户丢失后台任务感知。
pub(crate) async fn get_sessions_overview(
    axum::extract::State(st): axum::extract::State<AppState>,
) -> Result<axum::response::Response, AppError> {
    let mut active = Vec::new();
    for s in st.session_manager.all_active().await {
        let label = st
            .session_manager
            .label_of(&s.session_id)
            .await
            .unwrap_or_else(|| format!("会话 {}", s.session_id));
        active.push(serde_json::json!({
            "sessionId": s.session_id,
            "repo": s.repo,
            "label": label,
            "status": format!("{:?}", s.status).to_lowercase(),
            "startedAt": st.session_manager.started_at_of(&s.session_id).await,
        }));
    }
    let queued: Vec<_> = st
        .session_queue
        .all()
        .await
        .iter()
        .map(|(repo, j)| {
            serde_json::json!({
                "repo": repo,
                "kind": j.kind.as_str(),
                "label": j.label,
                "moduleId": j.module_id,
                "enqueuedAt": j.enqueued_at,
            })
        })
        .collect();
    Ok(axum::Json(serde_json::json!({ "success": true, "data": { "active": active, "queued": queued } })).into_response())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnqueueBody {
    kind: String,
    module_id: Option<String>,
}

/// POST /repos/{id}/session-queue（§8/B4 修订契约）：
/// - 有活动会话 → 入队（已有则替换，响应带被替换项 label）→ `{queued:true, replaced:{label}|null}`
/// - 无活动会话 → 后端直接代执行（经 QueueHost 调同一个 *_inner）→ `{started:true}`
pub(crate) async fn post_session_queue(
    axum::extract::State(st): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::Json(body): axum::Json<EnqueueBody>,
) -> Result<axum::response::Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let job = build_job(&body)?;
    // 原子裁决：临界区内复查活动会话（B4 三段竞态收口——status_of 与入队同锁序，不会被 drain 插队）
    if st.has_active_session(&id).await {
        let replaced = st.session_queue.enqueue_or_replace(&st, &id, job).await;
        return Ok((
            axum::http::StatusCode::ACCEPTED,
            axum::Json(serde_json::json!({
                "success": true,
                "data": { "queued": true, "replaced": replaced.map(|label| serde_json::json!({ "label": label })) },
            })),
        )
            .into_response());
    }
    // 无活动会话：直接代执行（HTTP handler 与 drain 共用同一 QueueHost::run_job，行为不漂移）
    st.run_job(&id, &job).await.map_err(AppError)?;
    Ok((axum::http::StatusCode::ACCEPTED, axum::Json(serde_json::json!({ "success": true, "data": { "started": true } }))).into_response())
}

/// DELETE /repos/{id}/session-queue（§5）：取消排队（204；无排队 404）
pub(crate) async fn delete_session_queue(
    axum::extract::State(st): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<axum::response::Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    match st.session_queue.cancel(&st, &id).await {
        Some(_) => Ok(axum::http::StatusCode::NO_CONTENT.into_response()),
        None => Err(AppError(ApiError::NotFound("无排队任务".into()))),
    }
}

fn build_job(body: &EnqueueBody) -> Result<QueuedJob, AppError> {
    let kind = match body.kind.as_str() {
        "patrol" => JobKind::Patrol,
        "reinduce" => JobKind::Reinduce,
        "submap" => JobKind::Submap,
        other => return Err(AppError(ApiError::BadRequest(format!("未知排队类型: {other}（仅支持 patrol|reinduce|submap）")))),
    };
    let module_id = match kind {
        JobKind::Submap => {
            let mid = body.module_id.as_deref().filter(|s| !s.is_empty())
                .ok_or_else(|| AppError(ApiError::BadRequest("submap 排队必须带 moduleId".into())))?;
            if !easyvibe_map::is_valid_id(mid) {
                return Err(AppError(ApiError::BadRequest(format!("非法模块 id: {mid}"))));
            }
            Some(mid.to_string())
        }
        _ => None,
    };
    let label = match kind {
        JobKind::Patrol => "巡检".to_string(),
        JobKind::Reinduce => "归纳".to_string(),
        JobKind::Submap => format!("分析模块 {}", module_id.as_deref().unwrap_or("?")),
    };
    Ok(QueuedJob { kind, module_id, label, enqueued_at: Utc::now(), force_full: false })
}

// ---------- 请求面小件单测（HTTP/drain 集成用例在 main.rs tests，复用 chat_state 工厂） ----------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_job_validates_kind_and_module_id() {
        let ok = |r: Result<QueuedJob, AppError>| r.map_err(|e| e.0.to_string()).unwrap();
        let body = EnqueueBody { kind: "patrol".into(), module_id: None };
        let job = ok(build_job(&body));
        assert_eq!(job.kind, JobKind::Patrol);
        assert_eq!(job.label, "巡检");
        assert!(job.module_id.is_none());
        // submap 必须带合法 moduleId
        assert!(build_job(&EnqueueBody { kind: "submap".into(), module_id: None }).is_err());
        assert!(build_job(&EnqueueBody { kind: "submap".into(), module_id: Some("../etc".into()) }).is_err());
        let job = ok(build_job(&EnqueueBody { kind: "submap".into(), module_id: Some("exam-core".into()) }));
        assert_eq!(job.label, "分析模块 exam-core");
        // 未知类型 400
        assert!(build_job(&EnqueueBody { kind: "task".into(), module_id: None }).is_err());
    }
}
