//! 资源域：任务 CRUD/决策/重试/复审/回看/审批/diff/建议。

use crate::state::*;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use easyvibe_common::ApiError;
use easyvibe_event_bus::{publish, BusEvent};
use easyvibe_ai_agent::SuggestClient as _;
use crate::service::{self, CreateTaskRequest};

/// 按任务终止：解析任务 → 会话 → kill（任务卡的「终止」按钮走这里）
pub(crate) async fn post_task_kill(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    use easyvibe_db::TaskRepository as _;
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let task = st.task_repo.get(&tid).await?.ok_or_else(|| ApiError::NotFound(format!("任务 {tid} 不存在")))?;
    let sid = task.session_id.ok_or_else(|| ApiError::BadRequest(format!("任务 {tid} 无关联会话（未开始执行）")))?;
    st.session_manager.kill(&sid).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": true })).into_response())
}

/// 管理闭环（2026-10-03 现状重审 P0）：删除任务。
/// 有活动会话（running/awaiting_approval 挂起的 spawn）先 best-effort 终止，防孤儿 agent；
/// 级联清 approvals（repo 内）；任务自有归档 development_docs/{tid}.json 一并删除
/// （共享的 MEMORY/INDEX 文档不动）；返工链反链（origin/successor）指到已删任务的行
/// 保留不动——链上其余任务的血缘可读性优先于悬空指针的洁癖。
pub(crate) async fn delete_task(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    use easyvibe_db::TaskRepository as _;
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let task = st.task_repo.get(&tid).await?.ok_or_else(|| ApiError::NotFound(format!("任务 {tid} 不存在")))?;
    if matches!(task.status.as_str(), "running" | "awaiting_approval") {
        if let Some(sid) = &task.session_id {
            // best-effort：会话可能已自然终态（kill 返回 409/404），删除不该被拦
            let _ = st.session_manager.kill(sid).await;
        }
    }
    st.task_repo.delete(&tid).await?;
    // 任务自有归档（diff 全文）随任务删除；读端对缺文件本就返回 diff=null
    let archive = repo.root.join(".easyvibe/development_docs").join(format!("{tid}.json"));
    if archive.is_file() {
        let _ = std::fs::remove_file(&archive);
    }
    // 内存基线清残留（删除后采集永远不会来，留着只是泄漏）
    if let Ok(mut m) = st.executor.baselines.lock() {
        m.remove(&tid);
    }
    publish(&st.event_bus, BusEvent::TaskStatus { repo: id, task_id: tid, status: "deleted".into(), gate: None });
    Ok(Json(serde_json::json!({ "success": true, "data": true })).into_response())
}

/// 就地重试：failed/interrupted → pending 重新入队（见 TaskExecutor::retry 的语义注释）
pub(crate) async fn post_task_retry(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let task = st.executor.retry(&tid).await?;
    publish(&st.event_bus, BusEvent::TaskStatus { repo: id, task_id: tid, status: task.status.clone(), gate: task.gate.clone() });
    Ok(Json(serde_json::json!({ "success": true, "data": { "status": task.status, "gate": task.gate } })).into_response())
}

/// 修改并复审：子 agent 审查打回（rejected）→ 注入审查意见 → 直达实施阶段重跑 →
/// 完成后子 agent 自动复审（见 TaskExecutor::remediate 的语义注释）
pub(crate) async fn post_task_remediate(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let task = st.executor.remediate(&tid).await?;
    publish(&st.event_bus, BusEvent::TaskStatus { repo: id, task_id: tid, status: task.status.clone(), gate: task.gate.clone() });
    Ok(Json(serde_json::json!({ "success": true, "data": { "status": task.status, "gate": task.gate } })).into_response())
}

/// 管道回看·节点重开（方案 §3.1）：body `{gate: "analysis"|"solution"}`——
/// 放回目标评审关（见 TaskExecutor::rewind 的语义注释）；rewind 自身已广播，此处不再双发
pub(crate) async fn post_task_rewind(
    State(st): State<AppState>,
    Path((id, tid)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let target = body["gate"].as_str().ok_or_else(|| ApiError::BadRequest("缺少 gate 字段（analysis/solution）".into()))?;
    let task = st.executor.rewind(&tid, target).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "status": task.status, "gate": task.gate } })).into_response())
}

/// 人工触发子 agent 复审（代码审查节点的审查-修复闭环）：异步执行，结论经
/// result.review + 留痕 + 事件送达（见 TaskExecutor::review_now 的语义注释）
pub(crate) async fn post_task_review(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let task = st.executor.review_now(&tid).await?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(serde_json::json!({ "success": true, "data": { "status": task.status, "gate": task.gate } })),
    )
        .into_response())
}

// ---------- M2-5：入口对话（F2）+ 存为视图（F1b） ----------

// ---------- M3-1：配置体系（backend-design §10） ----------


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
    let task = st.executor.decide(&tid, &body.decision, body.note.as_deref(), body.gate.as_deref()).await?;
    publish(&st.event_bus, BusEvent::TaskStatus {
        repo: id,
        task_id: tid,
        status: task.status.clone(),
        gate: task.gate.clone(),
    });
    Ok(Json(serde_json::json!({ "success": true, "data": { "status": task.status, "gate": task.gate } })).into_response())
}

pub(crate) async fn list_task_approvals(State(st): State<AppState>, Path((_, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    use easyvibe_db::ApprovalRepository as _;
    let aps = st.approval_repo.list_by_task(&tid).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": aps })).into_response())
}

/// M4-3：任务完整 diff（按需读取，不随任务列表载荷）——development_docs 归档中的 diffFull；
/// 无归档/无 diff 返回 diff=null（调用方展示"无变更"）
pub(crate) async fn get_task_diff(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let path = repo.root.join(".easyvibe/development_docs").join(format!("{tid}.json"));
    if !path.exists() {
        return Ok(Json(serde_json::json!({ "success": true, "data": { "diff": null, "diffStat": null } })).into_response());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| ApiError::Internal(format!("归档读取失败: {e}")))?;
    let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| ApiError::Internal(format!("归档解析失败: {e}")))?;
    Ok(Json(serde_json::json!({
        "success": true,
        "data": { "diff": v["diffFull"], "diffStat": v["diffStat"] }
    }))
    .into_response())
}

/// 智能优化建议：AI 主动发现优化机会（Stub=确定性派生；LLM=地图注入生成），
/// 每条建议可一键转修复任务（前端组装 TaskDraft）
pub(crate) async fn suggest(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let snap = st.map_service.load_map(&repo).await?;
    let suggestions: Vec<easyvibe_ai_agent::Suggestion> = match *st.llm_mode {
        LlmMode::Stub => easyvibe_ai_agent::StubSuggestClient.suggest(&snap.json).await?,
        LlmMode::Anthropic => {
            let cfg = resolve_llm(&st, &id, "chat").await;
            if cfg.api_key.is_empty() {
                return Err(AppError(ApiError::BadRequest("未配置 LLM API key（设置面板或 EASYVIBE_LLM_API_KEY）".into())));
            }
            let llm = easyvibe_ai_agent::AnthropicClient::new(&cfg.base_url, &cfg.api_key, &cfg.model);
            easyvibe_ai_agent::LlmSuggestClient::new(llm).suggest(&snap.json).await?
        }
    };
    let items: Vec<serde_json::Value> = suggestions
        .into_iter()
        .map(|sg| serde_json::json!({ "title": sg.title, "description": sg.description, "modules": sg.modules, "priority": sg.priority, "rationale": sg.rationale }))
        .collect();
    Ok(Json(serde_json::json!({ "success": true, "data": items })).into_response())
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
    use easyvibe_db::TaskRepository as _;
    // M4-2：?conv=<id> 时会话级过滤（工作台影响面/计划进度）
    // 重审 P1：?limit= 可调（默认 50 是"任务一多旧任务消失"的失控感来源），上限 500 防全表拉取
    let limit: i64 = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50).clamp(1, 500);
    let tasks = match q.get("conv") {
        Some(cid) => st.task_repo.list_by_conversation(cid).await?,
        None => st.task_repo.list(&id, limit).await?,
    };
    let items: Vec<serde_json::Value> = tasks
        .into_iter()
        .map(|t| serde_json::json!({
            "id": t.id, "title": t.title, "description": t.description,
            "modules": serde_json::from_str::<serde_json::Value>(&t.modules).unwrap_or_default(),
            "acceptance": t.acceptance, "source": t.source,
            "status": t.status, "trust": t.trust, "error": t.error,
            "gate": t.gate, "sessionId": t.session_id,
            "conversationId": t.conversation_id,
            "originTaskId": t.origin_task_id, "successorTaskId": t.successor_task_id,
            "result": t.result.as_deref().and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok()),
            "createdAt": t.created_at, "updatedAt": t.updated_at,
        }))
        .collect();
    Ok(Json(serde_json::json!({ "success": true, "data": items })).into_response())
}

// ---------- M3-5：会话持久化 + auto-compact（backend-design §10a / §11 🟡4/5/6） ----------

