//! 运行会话排队（需求方案「运行会话气泡 + 单会话排队 v1」§4.1/§5/§8）。
//!
//! 单槽队列（key=repo_id，新排队替换旧排队并广播被替换项）；POST 原子裁决
//! （B4：有活动会话 → 入队；无 → 后端直接代执行）；终态事件驱动 drain（I2：
//! 事件循环只做「判断 + pop + spawn」，执行 IO 不阻塞会话事件广播）+ 12s 清扫器
//! 兜底（B3：事件丢失路径不留死队）。每次队列变更都广播 queue.changed。
use crate::{publish, AppError, AppState, BusEvent};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use easyvibe_api_types::SessionStatus;
use easyvibe_common::ApiError;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// 队列槽位：每仓库同时最多排 1 个（防链式雪崩，需求 §4.3 明确不做多槽）
pub type SessionQueue = Arc<Mutex<HashMap<String, QueuedJob>>>;

/// 排队任务类型（patrol|reinduce|submap；任务槽位不参与排队——任务退回 pending 有自己的机制）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Patrol,
    Reinduce,
    Submap,
}

impl JobKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobKind::Patrol => "patrol",
            JobKind::Reinduce => "reinduce",
            JobKind::Submap => "submap",
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueuedJob {
    pub kind: JobKind,
    /// Submap 必填（模块 id）；其余类型为 None
    pub module_id: Option<String>,
    /// 展示用：「巡检」「归纳」「分析模块 <id>」
    pub label: String,
    pub enqueued_at: DateTime<Utc>,
}

/// 队列变更（每次变更都经 queue.changed 广播，payload 见 ws_handler 的翻译）
#[derive(Debug, Clone)]
pub enum QueueChange {
    Enqueued { job: QueuedJob },
    /// S3：替换时携带被替换项 label，前端 toast「已替换之前的排队：{旧label}」
    Replaced { job: QueuedJob, replaced_label: String },
    Cancelled { job: QueuedJob },
    /// I3：排空携带任务信息与 started 标记
    Drained { job: QueuedJob, started: bool },
    /// B2：执行撞 Conflict（TOCTOU），项放回原槽位（未覆盖期间用户新排队）
    Requeued { job: QueuedJob },
    /// B2：确定性失败，丢弃 + 广播失败原因（不留死信，避免无限重试）
    Failed { job: QueuedJob, error: String },
}

impl QueueChange {
    fn job(&self) -> &QueuedJob {
        match self {
            QueueChange::Enqueued { job }
            | QueueChange::Replaced { job, .. }
            | QueueChange::Cancelled { job }
            | QueueChange::Drained { job, .. }
            | QueueChange::Requeued { job }
            | QueueChange::Failed { job, .. } => job,
        }
    }

    pub fn type_str(&self) -> &'static str {
        match self {
            QueueChange::Enqueued { .. } => "enqueued",
            QueueChange::Replaced { .. } => "replaced",
            QueueChange::Cancelled { .. } => "cancelled",
            QueueChange::Drained { .. } => "drained",
            QueueChange::Requeued { .. } => "requeued",
            QueueChange::Failed { .. } => "failed",
        }
    }

    /// WS payload：repo/type/job + 各类型附加字段（replacedLabel/started/error）
    pub fn to_payload(&self, repo: &str) -> serde_json::Value {
        let job = self.job();
        let mut data = serde_json::json!({
            "repo": repo,
            "type": self.type_str(),
            "job": {
                "kind": job.kind.as_str(),
                "label": job.label,
                "moduleId": job.module_id,
                "enqueuedAt": job.enqueued_at,
            },
        });
        match self {
            QueueChange::Replaced { replaced_label, .. } => data["replacedLabel"] = replaced_label.clone().into(),
            QueueChange::Drained { started, .. } => data["started"] = (*started).into(),
            QueueChange::Failed { error, .. } => data["error"] = error.clone().into(),
            _ => {}
        }
        data
    }
}

fn broadcast_change(st: &AppState, repo: &str, change: QueueChange) {
    publish(&st.event_bus, BusEvent::QueueChanged { repo: repo.to_string(), change });
}

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
    let queued = st.session_queue.lock().await.get(&id).map(|j| {
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
        .lock()
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
/// - 无活动会话 → 后端直接代执行（调同一个 *_inner）→ `{started:true}`（原「无活动返回 409」契约删除）
pub(crate) async fn post_session_queue(
    axum::extract::State(st): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::Json(body): axum::Json<EnqueueBody>,
) -> Result<axum::response::Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let job = build_job(&body)?;
    // 原子裁决：临界区内复查活动会话（B4 三段竞态收口——status_of 与入队同锁序，不会被 drain 插队）
    let active = st.session_manager.status_of(&id).await;
    if matches!(active, Some(ref s) if matches!(s.status, SessionStatus::Starting | SessionStatus::Running)) {
        let replaced = {
            let mut q = st.session_queue.lock().await;
            q.insert(id.clone(), job.clone())
        };
        match &replaced {
            Some(old) => broadcast_change(&st, &id, QueueChange::Replaced { job, replaced_label: old.label.clone() }),
            None => broadcast_change(&st, &id, QueueChange::Enqueued { job }),
        }
        return Ok((
            axum::http::StatusCode::ACCEPTED,
            axum::Json(serde_json::json!({
                "success": true,
                "data": { "queued": true, "replaced": replaced.map(|old| serde_json::json!({ "label": old.label })) },
            })),
        )
            .into_response());
    }
    // 无活动会话：直接代执行（HTTP handler 与 drain 共用同一 inner，行为不漂移）
    run_queued_job(&st, &id, &job).await?;
    Ok((axum::http::StatusCode::ACCEPTED, axum::Json(serde_json::json!({ "success": true, "data": { "started": true } }))).into_response())
}

/// DELETE /repos/{id}/session-queue（§5）：取消排队（204；无排队 404）
pub(crate) async fn delete_session_queue(
    axum::extract::State(st): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<axum::response::Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let removed = st.session_queue.lock().await.remove(&id);
    match removed {
        Some(job) => {
            broadcast_change(&st, &id, QueueChange::Cancelled { job });
            Ok(axum::http::StatusCode::NO_CONTENT.into_response())
        }
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
    Ok(QueuedJob { kind, module_id, label, enqueued_at: Utc::now() })
}

/// 执行一个队列任务：按 kind 分发到与 HTTP handler 相同的 inner（§4.1）
pub(crate) async fn run_queued_job(st: &AppState, repo_id: &str, job: &QueuedJob) -> Result<(), AppError> {
    match job.kind {
        JobKind::Patrol => crate::start_patrol_inner(st.clone(), repo_id.to_string()).await.map(|_| ()),
        JobKind::Reinduce => crate::start_reinduce_inner(st.clone(), repo_id.to_string()).await.map(|_| ()),
        JobKind::Submap => {
            let module_id = job.module_id.clone().unwrap_or_default();
            crate::analyze_submap_inner(st.clone(), repo_id.to_string(), module_id).await.map(|_| ())
        }
    }
}

/// drain 失败分级（B2）：
/// - Conflict（TOCTOU：终态后任务槽等抢注了活动会话）→ 放回原槽位，**不覆盖**期间用户新排的队，等下一触发
/// - 其他确定性失败 → 丢弃 + 广播 queue.changed{type:"failed", error}（不留死信，禁止只留 warn 日志）
pub(crate) async fn handle_drain_failure(st: &AppState, repo_id: &str, job: QueuedJob, err: ApiError) {
    match err {
        ApiError::Conflict(_) => {
            tracing::warn!("[queue] {} 排队任务「{}」执行撞 Conflict（TOCTOU），放回原槽位等下一触发", repo_id, job.label);
            let mut q = st.session_queue.lock().await;
            q.entry(repo_id.to_string()).or_insert(job.clone());
            drop(q);
            broadcast_change(st, repo_id, QueueChange::Requeued { job });
        }
        other => {
            tracing::warn!("[queue] {} 排队任务「{}」启动失败，丢弃: {}", repo_id, job.label, other);
            broadcast_change(st, repo_id, QueueChange::Failed { job, error: other.to_string() });
        }
    }
}

/// drain（§4.1）：「有排队 && 无活动会话」→ pop + tokio::spawn 执行（I2：本函数只做
/// 判断 + pop，IO 不阻塞调用方——事件循环与清扫器共用）。返回 true 表示发生了 drain。
pub(crate) async fn drain_repo_queue(st: &AppState, repo_id: &str) -> bool {
    // 复查活动会话（防御：终态事件后可能已有新会话抢注，如任务槽——活动则跳过等下一终态）
    if let Some(s) = st.session_manager.status_of(repo_id).await {
        if matches!(s.status, SessionStatus::Starting | SessionStatus::Running) {
            return false;
        }
    }
    let job = match st.session_queue.lock().await.remove(repo_id) {
        Some(j) => j,
        None => return false,
    };
    broadcast_change(st, repo_id, QueueChange::Drained { job: job.clone(), started: true });
    let st2 = st.clone();
    let repo = repo_id.to_string();
    tokio::spawn(async move {
        if let Err(e) = run_queued_job(&st2, &repo, &job).await {
            handle_drain_failure(&st2, &repo, job, e.0).await;
        }
    });
    true
}

/// 周期清扫器（B3）：12s 一拍，凡「有排队 && 无活动会话」的仓库触发 drain——
/// 事件驱动降级为「事件加速 + 周期校对」，覆盖 try_send 背压丢弃/任务槽终态/超时 kill/spawn 失败等漏事件路径
pub(crate) async fn run_queue_sweeper(st: AppState) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(12));
    loop {
        tick.tick().await;
        let repos: Vec<String> = st.session_queue.lock().await.keys().cloned().collect();
        for repo in repos {
            drain_repo_queue(&st, &repo).await;
        }
    }
}

// ---------- 内部小件单测（HTTP/drain 集成用例在 main.rs tests，复用 chat_state 工厂） ----------

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

    #[test]
    fn queue_change_payload_carries_type_and_extras() {
        let job = QueuedJob { kind: JobKind::Reinduce, module_id: None, label: "归纳".into(), enqueued_at: Utc::now() };
        let p = QueueChange::Enqueued { job: job.clone() }.to_payload("r1");
        assert_eq!(p["type"], "enqueued");
        assert_eq!(p["job"]["kind"], "reinduce");
        assert!(p.get("started").is_none());
        let p = QueueChange::Drained { job: job.clone(), started: true }.to_payload("r1");
        assert_eq!(p["type"], "drained");
        assert_eq!(p["started"], true);
        let p = QueueChange::Replaced { job: job.clone(), replaced_label: "巡检".into() }.to_payload("r1");
        assert_eq!(p["replacedLabel"], "巡检");
        let p = QueueChange::Failed { job, error: "agent 缺失".into() }.to_payload("r1");
        assert_eq!(p["type"], "failed");
        assert_eq!(p["error"], "agent 缺失");
    }
}
