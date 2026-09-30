//! 组装层：二进制入口，REST 路由 + WS 事件推送。
use axum::{
    extract::{Path, State, WebSocketUpgrade},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use easyvibe_api_types::{HealthResponse, MapChanged, MapInvalid, RepoInfo, SessionStatusChanged, WsMessage};
use easyvibe_common::{events as ev, ApiError, ApiResponse, ErrorResponse};
use easyvibe_map::{repo_from_root, spawn_map_watcher, MapService};
use easyvibe_session::SessionManager;
use easyvibe_ai_agent::{QaClient as _, SuggestClient as _};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::info;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct AppState {
    pub map_service: Arc<MapService>,
    pub session_manager: Arc<SessionManager>,
    /// v2.2 提示词模板（含 <REPO_ROOT> 占位）
    pub prompt_template: Arc<String>,
    /// agent CLI 命令与参数（如 claude + ["-p"]）
    pub agent_command: Arc<String>,
    pub agent_args: Arc<Vec<String>>,
    // M2-4：巡检槽位 + 域 2 健康历史
    pub patrol_service: Arc<easyvibe_ai_agent::PatrolService<easyvibe_db::SqliteHealthRepository>>,
    pub health_repo: Arc<easyvibe_db::SqliteHealthRepository>,
    pub settings_repo: Arc<easyvibe_db::SqliteSettingsRepository>,
    pub cipher: Arc<easyvibe_common::SecretCipher>,
    pub task_repo: Arc<easyvibe_db::SqliteTaskRepository>,
    pub approval_repo: Arc<easyvibe_db::SqliteApprovalRepository>,
    pub executor: Arc<task_exec::TaskExecutor>,
    pub llm_mode: Arc<LlmMode>,
    pub patrol_prompt: Arc<String>,
    pub schema_path: Arc<String>,
    /// 后端 → 前端事件总线（broadcast；WS handler 订阅）
    pub event_bus: broadcast::Sender<BusEvent>,
}

/// LLM 客户端来源：stub（零成本验证）或 anthropic（真实 API）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmMode {
    Stub,
    Anthropic,
}

/// 总线事件（内部枚举，发送时翻译为 WsMessage）
#[derive(Debug, Clone)]
pub enum BusEvent {
    MapChanged(MapChanged),
    MapInvalid(MapInvalid),
    Growth { repo: String, event: Value },
    Progress { repo: String, progress: Value },
    SessionStatus(SessionStatusChanged),
    TaskStatus { repo: String, task_id: String, status: String, gate: Option<String> },
}

pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/repos", get(list_repos))
        .route("/repos/{id}/map", get(get_map))
        .route("/repos/{id}/growth", get(get_growth))
        .route("/repos/{id}/modules/{module_id}", get(get_submap))
        .route("/repos/{id}/reinduce", axum::routing::post(start_reinduce))
        .route("/repos/{id}/patrol", axum::routing::post(start_patrol))
        .route("/repos/{id}/patrol-runs", get(list_patrol_runs))
        .route("/repos/{id}/chat", axum::routing::post(chat))
        .route("/repos/{id}/views", axum::routing::post(save_view))
        .route("/repos/{id}/tasks", get(list_tasks).post(create_task))
        .route("/repos/{id}/tasks/{tid}/decide", axum::routing::post(decide_task))
        .route("/repos/{id}/tasks/{tid}/approvals", get(list_task_approvals))
        .route("/repos/{id}/suggest", axum::routing::post(suggest))
        .route("/settings", get(list_settings))
        .route("/settings/set", axum::routing::put(put_setting))
        .route("/settings/{scope}/{key}", axum::routing::delete(delete_setting))
        .with_state(state.clone());

    Router::new()
        .nest("/api", api)
        .route("/ws", get(ws_handler))
        .with_state(state)
}

async fn health() -> Json<ApiResponse<HealthResponse>> {
    Json(ApiResponse::ok(HealthResponse { status: "ok".into(), version: VERSION.into() }))
}

async fn list_repos(State(st): State<AppState>) -> Json<ApiResponse<Vec<RepoInfo>>> {
    let repos = st
        .map_service
        .repos()
        .into_iter()
        .map(|r| RepoInfo { id: r.id, name: r.name, root: r.root.to_string_lossy().into_owned() })
        .collect();
    Json(ApiResponse::ok(repos))
}

async fn get_map(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let snap = st.map_service.load_map(&repo).await?;
    Ok(Json(snap.json).into_response())
}

async fn get_growth(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    // 原样返回 growth.log 事件数组（与文件行一致，前端状态机统一消费）
    Ok(Json(st.map_service.load_growth(&repo).await?).into_response())
}

async fn get_submap(
    State(st): State<AppState>,
    Path((id, module_id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    // 路径遍历防线：module_id 必须满足 Schema 的 id 字符集（审查 🔴1）
    if !easyvibe_map::is_valid_id(&module_id) {
        return Err(ApiError::BadRequest(format!("非法模块 id: {module_id}")).into());
    }
    let repo = st.map_service.find_repo(&id).ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    Ok(Json(st.map_service.load_submap(&repo, &module_id).await?).into_response())
}

/// 触发重新归纳（写路径，M2-3）：spawn 外部 agent 按 v2.2 协议执行，
/// 三通道（progress/growth.log/map.json）由 watcher 自动直播，前端无需轮询
async fn start_reinduce(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    // 实弹验证发现：agent 可能"成功退出但什么都没写"（如模型能力不足只输出分析），
    // 因此记录会话前的地图哈希，终态后比对——未变化则告警（会话仍算成功：重归纳产出相同内容合法）。
    let hash_before = st.map_service.load_map(&repo).await.ok().map(|s| s.content_hash);
    let session = st
        .session_manager
        .start_induction(&repo.id, &repo.root, &st.prompt_template, &st.agent_command, &st.agent_args)
        .await?;

    // 终态后产物核验
    let st2 = st.clone();
    let repo2 = repo.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            match st2.session_manager.status_of(&repo2.id).await {
                Some(s) if !matches!(s.status, easyvibe_api_types::SessionStatus::Starting | easyvibe_api_types::SessionStatus::Running) => {
                    if let Some(before) = hash_before {
                        if let Ok(snap) = st2.map_service.load_map(&repo2).await {
                            if snap.content_hash == before {
                                tracing::warn!(
                                    "[reinduce] 会话 {} 终态 {:?} 但 map.json 未变化——agent 可能未执行归纳（模型能力/提示词遵从？）",
                                    s.session_id, s.status
                                );
                            }
                        }
                    }
                    break;
                }
                _ => {}
            }
        }
    });

    Ok((axum::http::StatusCode::ACCEPTED, Json(session)).into_response())
}

/// 触发巡检（M2-4，实弹 #2 修订）：两条执行路径——
/// - Stub 模式：ai-agent PatrolService 零成本确定性巡检（测试/demo）
/// - 真实模式：**session spawn**（与归纳同路径）。实弹发现直调无 tools 声明的 API
///   只会得到模型的 tool_call 幻觉（DSML 伪调用），真实巡检需要 Bash 核查
///   （wc/git/grep），是工具型任务，必须由带工具的 agent 执行。
/// 两条路径共用写互斥（try_register），终态后健康历史落域 2。
async fn start_patrol(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    if *st.llm_mode == LlmMode::Anthropic && std::env::var("EASYVIBE_LLM_API_KEY").is_err() {
        return Err(ApiError::BadRequest("未配置 EASYVIBE_LLM_API_KEY".into()).into());
    }
    let run_id = format!("patrol-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));

    match *st.llm_mode {
        LlmMode::Stub => {
            st.session_manager
                .try_register(SessionStatusChanged { repo: repo.id.clone(), session_id: run_id.clone(), status: easyvibe_api_types::SessionStatus::Running })
                .await?;
            let snap = match st.map_service.load_map(&repo).await {
                Ok(s) => s,
                Err(e) => {
                    st.session_manager.note_status(SessionStatusChanged { repo: repo.id.clone(), session_id: run_id.clone(), status: easyvibe_api_types::SessionStatus::Failed }).await;
                    return Err(e.into());
                }
            };
            let st2 = st.clone();
            let repo2 = repo.clone();
            let run_id_task = run_id.clone();
            tokio::spawn(async move {
                let llm = easyvibe_ai_agent::StubLlmClient::new();
                let result = st2
                    .patrol_service
                    .run(Some(run_id_task.clone()), &repo2.id, &repo2.root, &snap.json, &st2.patrol_prompt, &st2.schema_path, &llm)
                    .await;
                let status = match &result {
                    Ok(_) => easyvibe_api_types::SessionStatus::Succeeded,
                    Err(_) => easyvibe_api_types::SessionStatus::Failed,
                };
                st2.session_manager.note_status(SessionStatusChanged { repo: repo2.id, session_id: run_id_task, status }).await;
                if let Err(e) = result {
                    tracing::warn!("[patrol] 失败: {e}");
                }
            });
            Ok((axum::http::StatusCode::ACCEPTED, Json(serde_json::json!({ "started": true, "runId": run_id, "mode": "stub" }))).into_response())
        }
        LlmMode::Anthropic => {
            // 真实巡检 = 工具型执行：spawn 带工具的 CLI agent，prompt 要求原子写回 map.json
            let session = st
                .session_manager
                .start_induction(&repo.id, &repo.root, &st.patrol_prompt, &st.agent_command, &st.agent_args)
                .await?;
            // 终态后：解析产物地图，健康历史落域 2（succeeded 但产物缺 health 也算失败记录）
            let st2 = st.clone();
            let repo2 = repo.clone();
            let session_id = session.session_id.clone();
            let model = st.agent_command.to_string();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let Some(s) = st2.session_manager.status_of(&repo2.id).await else { continue };
                    if matches!(s.status, easyvibe_api_types::SessionStatus::Starting | easyvibe_api_types::SessionStatus::Running) { continue }
                    let result = async {
                        let snap = st2.map_service.load_map(&repo2).await?;
                        st2.patrol_service
                            .record_from_map(&session_id, &repo2.id, &model, &snap.json, s.status == easyvibe_api_types::SessionStatus::Succeeded, None)
                            .await
                    }
                    .await;
                    if let Err(e) = result {
                        tracing::warn!("[patrol] 健康历史落库失败: {e}");
                    }
                    break;
                }
            });
            Ok((axum::http::StatusCode::ACCEPTED, Json(serde_json::json!({ "started": true, "sessionId": session.session_id, "mode": "agent" }))).into_response())
        }
    }
}

/// 健康历史：巡检运行列表（域 2 的第一个读接口）
async fn list_patrol_runs(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    use easyvibe_db::HealthRepository as _;
    let runs = st.health_repo.list_runs(&id, 20).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": runs })).into_response())
}

// ---------- M2-5：入口对话（F2）+ 存为视图（F1b） ----------

// ---------- M3-1：配置体系（backend-design §10） ----------

use easyvibe_db::{SettingRow, SettingsRepository as _};

/// 生效配置解析：仓库行覆盖全局行；无设置时回退环境变量（开发期手段）。
pub struct ResolvedLlm {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

async fn service_of(st: &AppState, scope: &str, slot: &str) -> (String, Option<serde_json::Value>, Option<String>) {
    let binding = st.settings_repo.get(scope, &format!("slot.{slot}")).await.ok().flatten()
        .and_then(|r| serde_json::from_str::<String>(&r.value).ok())
        .unwrap_or_else(|| "default".into());
    let base = st.settings_repo.get(scope, &format!("llm.service.{binding}")).await.ok().flatten()
        .and_then(|r| serde_json::from_str::<serde_json::Value>(&r.value).ok());
    let key = st.settings_repo.get(scope, &format!("llm.service.{binding}.apiKey")).await.ok().flatten()
        .and_then(|r| if r.encrypted { st.cipher.decrypt(&r.value).ok() } else { Some(r.value) })
        .and_then(|v| serde_json::from_str::<String>(&v).ok());
    (binding, base, key)
}

/// 生效配置解析：仓库行覆盖全局行；无设置时回退环境变量（开发期手段）。
pub async fn resolve_llm(st: &AppState, repo_id: &str, slot: &str) -> ResolvedLlm {
    // 仓库级优先，全局兜底；key 在仓库级没有时回落全局
    let (mut binding, mut base, mut key) = service_of(st, repo_id, slot).await;
    if base.is_none() {
        let g = service_of(st, "global", slot).await;
        binding = g.0;
        base = g.1;
        if key.is_none() { key = g.2; }
    }
    let _ = binding;
    let obj = base.unwrap_or_default();
    ResolvedLlm {
        base_url: obj.get("baseUrl").and_then(|v| v.as_str()).map(Into::into)
            .or_else(|| std::env::var("EASYVIBE_LLM_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.anthropic.com".into()),
        api_key: key.or_else(|| std::env::var("EASYVIBE_LLM_API_KEY").ok()).unwrap_or_default(),
        model: obj.get("model").and_then(|v| v.as_str()).map(Into::into)
            .or_else(|| std::env::var("EASYVIBE_LLM_MODEL").ok())
            .unwrap_or_else(|| "claude-sonnet-4-5".into()),
    }
}

/// 敏感 key 规则：以 .apiKey / apiKey 结尾自动加密 at rest
fn is_sensitive_key(key: &str) -> bool {
    key.ends_with(".apiKey") || key.ends_with("apiKey")
}

async fn list_settings(State(st): State<AppState>, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> Result<Response, AppError> {
    let scope = q.get("scope").cloned().unwrap_or_else(|| "global".into());
    let rows = st.settings_repo.list(&scope).await?;
    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            let value = if r.encrypted {
                // 本地单机：解密返回供 UI 编辑（网络传输仅限 127.0.0.1）
                st.cipher.decrypt(&r.value).unwrap_or_default()
            } else {
                r.value
            };
            serde_json::json!({
                "key": r.key,
                "value": serde_json::from_str::<serde_json::Value>(&value).unwrap_or(serde_json::Value::String(value)),
                "encrypted": r.encrypted,
                "updatedAt": r.updated_at,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "success": true, "data": items })).into_response())
}

#[derive(serde::Deserialize)]
struct PutSettingRequest {
    scope: String,
    key: String,
    value: serde_json::Value,
}

async fn put_setting(State(st): State<AppState>, Json(body): Json<PutSettingRequest>) -> Result<Response, AppError> {
    if body.scope.is_empty() || body.key.is_empty() || body.key.contains('/') || body.key.contains("..") {
        return Err(AppError(ApiError::BadRequest("非法 scope/key".into())));
    }
    let sensitive = is_sensitive_key(&body.key);
    let raw = serde_json::to_string(&body.value).map_err(|e| AppError(ApiError::Internal(e.to_string())))?;
    let (value, encrypted) = if sensitive {
        (st.cipher.encrypt(&raw)?, true)
    } else {
        (raw, false)
    };
    st.settings_repo
        .set(&SettingRow {
            scope: body.scope,
            key: body.key,
            value,
            encrypted,
            updated_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string(),
        })
        .await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

async fn delete_setting(State(st): State<AppState>, Path((scope, key)): Path<(String, String)>) -> Result<Response, AppError> {
    st.settings_repo.delete(&scope, &key).await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

// ---------- M3-2：指哪打哪——任务创建（上下文已组织好随表单提交；执行引擎 M3-3 接入） ----------

/// 审批决策（M3-4）：approved/rejected 按当前关卡推进或终止；发射 task.statusChanged
#[derive(serde::Deserialize)]
struct DecideRequest {
    decision: String, // approved / rejected
    #[serde(default)]
    note: Option<String>,
}

async fn decide_task(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>, Json(body): Json<DecideRequest>) -> Result<Response, AppError> {
    use easyvibe_db::TaskRepository as _;
    let task = st.executor.decide(&tid, &body.decision, body.note.as_deref()).await?;
    let _ = st.event_bus.send(BusEvent::TaskStatus {
        repo: id,
        task_id: tid,
        status: task.status.clone(),
        gate: task.gate.clone(),
    });
    Ok(Json(serde_json::json!({ "success": true, "data": { "status": task.status, "gate": task.gate } })).into_response())
}

async fn list_task_approvals(State(st): State<AppState>, Path((_, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    use easyvibe_db::ApprovalRepository as _;
    let aps = st.approval_repo.list_by_task(&tid).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": aps })).into_response())
}

/// 智能优化建议：AI 主动发现优化机会（Stub=确定性派生；LLM=地图注入生成），
/// 每条建议可一键转修复任务（前端组装 TaskDraft）
async fn suggest(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
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

#[derive(serde::Deserialize)]
struct CreateTaskRequest {
    title: String,
    description: String,
    #[serde(default)]
    modules: Vec<String>,
    #[serde(default)]
    acceptance: String,
    #[serde(default)]
    source: String, // module / concern / layer / manual
    #[serde(default)]
    context: serde_json::Value,
    #[serde(default)]
    trust: String, // manual / auto
}

async fn create_task(State(st): State<AppState>, Path(id): Path<String>, Json(body): Json<CreateTaskRequest>) -> Result<Response, AppError> {
    use easyvibe_db::TaskRepository as _;
    let repo = st.map_service.find_repo(&id).ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    if body.description.trim().is_empty() {
        return Err(AppError(ApiError::BadRequest("需求描述不能为空".into())));
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0).to_string();
    let task_id = format!("task-{}", &now);
    let repo_id = repo.id.clone();
    let row = easyvibe_db::TaskRow {
        id: task_id.clone(),
        repo: repo_id.clone(),
        title: body.title,
        description: body.description,
        modules: serde_json::to_string(&body.modules).unwrap_or_else(|_| "[]".into()),
        acceptance: body.acceptance,
        source: if body.source.is_empty() { "manual".into() } else { body.source },
        context: serde_json::to_string(&body.context).unwrap_or_else(|_| "{}".into()),
        status: "pending".into(), // M3-3：harness 执行引擎接走
        trust: if body.trust == "auto" { "auto".into() } else { "manual".into() },
        error: None,
        session_id: None,
        gate: None,
        created_at: now.clone(),
        updated_at: now,
    };
    st.task_repo.create(&row).await?;
    // M3-3：入队执行（harness 引擎；并发上限 4，审批门 M3-4 接入）
    st.executor.clone().enqueue_pending(Some(&repo_id)).await;
    Ok((axum::http::StatusCode::CREATED, Json(serde_json::json!({ "success": true, "data": { "id": task_id } }))).into_response())
}

async fn list_tasks(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    use easyvibe_db::TaskRepository as _;
    let tasks = st.task_repo.list(&id, 50).await?;
    let items: Vec<serde_json::Value> = tasks
        .into_iter()
        .map(|t| serde_json::json!({
            "id": t.id, "title": t.title, "description": t.description,
            "modules": serde_json::from_str::<serde_json::Value>(&t.modules).unwrap_or_default(),
            "acceptance": t.acceptance, "source": t.source,
            "status": t.status, "trust": t.trust, "error": t.error,
            "gate": t.gate, "sessionId": t.session_id,
            "createdAt": t.created_at, "updatedAt": t.updated_at,
        }))
        .collect();
    Ok(Json(serde_json::json!({ "success": true, "data": items })).into_response())
}

#[derive(serde::Deserialize)]
struct ChatHttpRequest {
    message: String,
    #[serde(default)]
    history: Vec<ChatTurn>,
}

#[derive(serde::Deserialize, Clone)]
struct ChatTurn {
    role: String, // user / assistant
    content: String,
}

/// 入口对话：基于语义地图问答（M2-5 范围：地图级回答；代码级追问留待 M3 工具能力）
async fn chat(State(st): State<AppState>, Path(id): Path<String>, Json(body): Json<ChatHttpRequest>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let snap = st.map_service.load_map(&repo).await?;
    // 历史折叠为 (q, a) 对（容错奇数/乱序）
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut pending_q: Option<String> = None;
    for t in &body.history {
        match t.role.as_str() {
            "user" => pending_q = Some(t.content.clone()),
            "assistant" => {
                if let Some(q) = pending_q.take() {
                    pairs.push((q, t.content.clone()));
                }
            }
            _ => {}
        }
    }
    let answer: easyvibe_ai_agent::QaAnswer = match *st.llm_mode {
        LlmMode::Stub => easyvibe_ai_agent::StubQaClient::new().ask(&snap.json, &body.message, &pairs).await?,
        LlmMode::Anthropic => {
            // M3-1：槽位配置解析（settings 库优先，env 兜底）
            let cfg = resolve_llm(&st, &id, "chat").await;
            if cfg.api_key.is_empty() {
                return Err(AppError(ApiError::BadRequest("未配置 LLM API key（设置面板或 EASYVIBE_LLM_API_KEY）".into())));
            }
            let llm = easyvibe_ai_agent::AnthropicClient::new(&cfg.base_url, &cfg.api_key, &cfg.model);
            easyvibe_ai_agent::LlmQaClient::new(llm).ask(&snap.json, &body.message, &pairs).await?
        }
    };
    Ok(Json(serde_json::json!({
        "success": true,
        "data": { "reply": answer.reply, "refs": answer.refs }
    }))
    .into_response())
}

#[derive(serde::Deserialize)]
struct SaveViewRequest {
    name: String,
    #[serde(default)]
    nodes: Vec<String>, // ["module:exam-core", ...]
    #[serde(default)]
    edges: Vec<serde_json::Value>,
    #[serde(default)]
    annotations: Vec<serde_json::Value>,
}

/// 存为视图（F1b 首次消费）：按格式规范 §9 写 .easyvibe/views/<slug>.json（引用式，不存布局）
async fn save_view(State(st): State<AppState>, Path(id): Path<String>, Json(body): Json<SaveViewRequest>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let slug: String = body
        .name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || ('\u{4e00}'..='\u{9fff}').contains(&c) { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(40)
        .collect();
    let slug = if slug.is_empty() {
        format!("view-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0))
    } else {
        slug
    };
    let view = serde_json::json!({
        "version": "1.0",
        "name": body.name,
        "created_at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string(),
        "source": { "conversation_id": "manual" },
        "nodes": body.nodes.iter().map(|r| serde_json::json!({ "ref": r })).collect::<Vec<_>>(),
        "edges": body.edges,
        "annotations": body.annotations,
    });
    let dir = repo.root.join(".easyvibe/views");
    tokio::fs::create_dir_all(&dir).await.map_err(|e| ApiError::Internal(format!("创建 views 目录失败: {e}")))?;
    let path = dir.join(format!("{slug}.json"));
    easyvibe_map::atomic_write_json(&path, &view).await?;
    Ok((axum::http::StatusCode::CREATED, Json(serde_json::json!({ "success": true, "data": { "path": path.to_string_lossy() } }))).into_response())
}

/// WS：订阅事件总线，向前端推送 domain.camelCase 事件
async fn ws_handler(State(st): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |mut socket| async move {
        use axum::extract::ws::Message;
        let mut rx = st.event_bus.subscribe();
        // Lagged（广播滞后）不致命：跳过丢失的批次继续收；Closed 才退出
        loop {
            let event = match rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!("[ws] 滞后，丢弃 {skipped} 个事件（客户端应经 REST 重同步）");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let msg: WsMessage<Value> = match event {
                BusEvent::MapChanged(d) => WsMessage { name: ev::MAP_CHANGED.into(), data: serde_json::to_value(d).unwrap_or_default() },
                BusEvent::MapInvalid(d) => WsMessage { name: ev::MAP_INVALID.into(), data: serde_json::to_value(d).unwrap_or_default() },
                BusEvent::Growth { repo, event } => WsMessage {
                    name: ev::GROWTH_EVENT.into(),
                    data: serde_json::json!({ "repo": repo, "event": event }),
                },
                BusEvent::Progress { repo, progress } => WsMessage {
                    name: ev::PROGRESS_UPDATED.into(),
                    data: serde_json::json!({ "repo": repo, "progress": progress }),
                },
                BusEvent::SessionStatus(s) => WsMessage {
                    name: ev::SESSION_STATUS_CHANGED.into(),
                    data: serde_json::to_value(s).unwrap_or_default(),
                },
                BusEvent::TaskStatus { repo, task_id, status, gate } => WsMessage {
                    name: "task.statusChanged".into(),
                    data: serde_json::json!({ "repo": repo, "taskId": task_id, "status": status, "gate": gate }),
                },
            };
            if let Ok(text) = serde_json::to_string(&msg) {
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break; // 前端断开
                }
            }
        }
    })
}

/// ApiError 的新类型包装（绕过孤儿规则；common 层不依赖 axum）
struct AppError(ApiError);

impl From<ApiError> for AppError {
    fn from(e: ApiError) -> Self {
        AppError(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let err = self.0;
        let (status, code) = match &err {
            ApiError::BadRequest(_) => (axum::http::StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            ApiError::NotFound(_) => (axum::http::StatusCode::NOT_FOUND, "NOT_FOUND"),
            ApiError::Conflict(_) => (axum::http::StatusCode::CONFLICT, "CONFLICT"),
            ApiError::MapInvalid(_) => (axum::http::StatusCode::UNPROCESSABLE_ENTITY, "MAP_INVALID"),
            ApiError::Internal(_) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };
        let body = Json(ErrorResponse { success: false, error: err.to_string(), code: code.into() });
        (status, body).into_response()
    }
}

mod task_exec;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();

    // 仓库注册：从 EASYVIBE_REPO 环境变量读取（逗号分隔的多工作区预留），M2-1 先支持一个
    let repo_roots: Vec<std::path::PathBuf> = std::env::var("EASYVIBE_REPO")
        .expect("请设置 EASYVIBE_REPO 指向代码仓库根目录（可逗号分隔多个）")
        .split(',')
        .map(|s| s.trim().into())
        .collect();
    let repos: Vec<_> = repo_roots.iter().map(|p| repo_from_root(p)).collect();
    for r in &repos {
        info!("注册仓库 {} -> {}", r.id, r.root.display());
    }

    let map_service = MapService::new(repos.clone());
    // 预热缓存（不出残图：加载失败仅告警，不阻断启动）
    for r in &repos {
        if let Err(e) = map_service.load_map(r).await {
            tracing::warn!("预热 {} 失败: {e}", r.id);
        }
    }

    let (event_bus, _) = broadcast::channel(256);

    // session 管理：会话事件翻译进总线
    let (session_tx, mut session_rx) = tokio::sync::mpsc::channel::<SessionStatusChanged>(64);
    let bus = event_bus.clone();
    tokio::spawn(async move {
        while let Some(s) = session_rx.recv().await {
            let _ = bus.send(BusEvent::SessionStatus(s));
        }
    });
    let session_manager = SessionManager::new(session_tx);

    // agent CLI 配置：命令/参数/提示词模板均可环境变量覆盖（测试可用 stub 命令）。
    // 默认 -p --bare --dangerously-skip-permissions：bare 跳过宿主 hooks（防 grill-me 类
    // 钩子把无人值守任务带偏成访谈模式）；skip-permissions 授予 Bash 等工具（实弹验证发现
    // headless 下 Bash 默认被拒，agent 只能"分析后成功退出"什么都不写）。
    let agent_command = std::env::var("EASYVIBE_AGENT_CMD").unwrap_or_else(|_| "claude".into());
    let agent_args: Vec<String> = std::env::var("EASYVIBE_AGENT_ARGS")
        .unwrap_or_else(|_| "-p --bare --dangerously-skip-permissions".into())
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    let prompt_path = std::env::var("EASYVIBE_PROMPT_PATH")
        .unwrap_or_else(|_| "easyvibe-map-prompt-v2.2.md".into());
    let prompt_template = std::fs::read_to_string(&prompt_path)
        .unwrap_or_else(|e| panic!("提示词模板不可读 {prompt_path}: {e}（用 EASYVIBE_PROMPT_PATH 指定）"));
    info!("agent={} args={:?} prompt={}", agent_command, agent_args, prompt_path);

    // 每个仓库一个地图 watcher，变更翻译为总线事件
    for r in repos {
        // F1 打开仓库自动初始化（后端侧）：无合法地图的仓库启动即触发归纳，
        // 产物经 watcher 推送，前端 map.changed 后自动渲染
        if map_service.cached(&r.id).await.is_none() {
            info!("[auto-init] {} 无合法地图，自动触发归纳", r.id);
            if let Err(e) = session_manager
                .start_induction(&r.id, &r.root, &prompt_template, &agent_command, &agent_args)
                .await
            {
                tracing::warn!("[auto-init] {} 触发失败: {e}", r.id);
            }
        }
        let mut rx = spawn_map_watcher(map_service.clone(), r.clone());
        let bus = event_bus.clone();
        let repo_id = r.id.clone();
        tokio::spawn(async move {
            while rx.changed().await.is_ok() {
                let event = match rx.borrow().clone() {
                    Ok(snap) => {
                        let version = snap.json.get("version").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                        BusEvent::MapChanged(MapChanged { repo: repo_id.clone(), version })
                    }
                    Err(e) => BusEvent::MapInvalid(MapInvalid { repo: repo_id.clone(), error: e }),
                };
                let _ = bus.send(event);
            }
        });

        // growth.log 增量 → growth.event
        let mut grx = easyvibe_map::spawn_growth_watcher(r.clone());
        let bus = event_bus.clone();
        let repo_id = r.id.clone();
        tokio::spawn(async move {
            while grx.changed().await.is_ok() {
                let batch = grx.borrow().clone();
                for event in batch {
                    let _ = bus.send(BusEvent::Growth { repo: repo_id.clone(), event });
                }
            }
        });

        // progress.json → progress.updated
        let mut prx = easyvibe_map::spawn_progress_watcher(r.clone());
        let bus = event_bus.clone();
        let repo_id = r.id.clone();
        tokio::spawn(async move {
            while prx.changed().await.is_ok() {
                let progress = prx.borrow().clone();
                if progress.is_null() { continue; }
                let _ = bus.send(BusEvent::Progress { repo: repo_id.clone(), progress });
            }
        });
    }

    // M2-4：域 2 SQLite + 巡检槽位
    let data_dir = std::env::var("EASYVIBE_DATA_DIR").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        format!("{home}/.easyvibe")
    });
    std::fs::create_dir_all(&data_dir).unwrap_or_else(|e| panic!("数据目录不可写 {data_dir}: {e}"));
    let database = easyvibe_db::Database::connect_file(&format!("{data_dir}/easyvibe.db"))
        .await
        .expect("SQLite 初始化失败");
    info!("域 2 状态库: {data_dir}/easyvibe.db");
    let health_repo = Arc::new(easyvibe_db::SqliteHealthRepository::new(database.pool().clone()));
    let patrol_service = Arc::new(easyvibe_ai_agent::PatrolService::new(health_repo.clone()));
    let settings_repo = Arc::new(easyvibe_db::SqliteSettingsRepository::new(database.pool().clone()));
    let task_repo = Arc::new(easyvibe_db::SqliteTaskRepository::new(database.pool().clone()));
    let approval_repo = Arc::new(easyvibe_db::SqliteApprovalRepository::new(database.pool().clone()));

    // 主密钥：EASYVIBE_MASTER_KEY（64 位十六进制）优先，否则数据目录 .master_key（0600，首次生成）
    let cipher = {
        let from_env = std::env::var("EASYVIBE_MASTER_KEY").ok().and_then(|hex| easyvibe_common::SecretCipher::from_hex_key(&hex).ok());
        from_env.unwrap_or_else(|| {
            let path = format!("{data_dir}/.master_key");
            let hex = std::fs::read_to_string(&path).unwrap_or_else(|_| {
                let mut bytes = [0u8; 32];
                getrandom::getrandom(&mut bytes).expect("主密钥生成失败");
                let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                std::fs::write(&path, &hex).expect("写主密钥文件失败");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt as _;
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                }
                hex
            });
            easyvibe_common::SecretCipher::from_hex_key(hex.trim()).expect("主密钥文件损坏")
        })
    };

    let llm_mode = if std::env::var("EASYVIBE_LLM_MODE").map(|v| v == "stub").unwrap_or(false) {
        LlmMode::Stub
    } else {
        LlmMode::Anthropic
    };
    let patrol_prompt = std::fs::read_to_string(
        std::env::var("EASYVIBE_PATROL_PROMPT_PATH").unwrap_or_else(|_| "easyvibe-map-patrol-prompt.md".into()),
    )
    .expect("巡检提示词模板不可读（用 EASYVIBE_PATROL_PROMPT_PATH 指定）");
    let schema_path = std::env::var("EASYVIBE_SCHEMA_PATH").unwrap_or_else(|_| "easyvibe-map-schema-v1.json".into());

    // M3-3：harness 装载（路径换姓）+ 任务执行引擎 + pending 恢复
    let workspace_ref = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../reference");
    let (_harness_dir, framework) = task_exec::load_harness(&workspace_ref).expect("harness 装载失败");
    info!("harness: {}", _harness_dir.display());
    let executor = task_exec::TaskExecutor::new(
        task_repo.clone(),
        approval_repo.clone(),
        session_manager.clone(),
        map_service.clone(),
        Arc::new(framework),
        Arc::new(agent_command.clone()),
        Arc::new(agent_args.clone()),
    );
    executor.enqueue_pending(None).await;

    let state = AppState {
        map_service,
        session_manager,
        prompt_template: Arc::new(prompt_template),
        agent_command: Arc::new(agent_command),
        agent_args: Arc::new(agent_args),
        patrol_service,
        health_repo,
        settings_repo,
        cipher: Arc::new(cipher),
        task_repo,
        approval_repo,
        executor,
        llm_mode: Arc::new(llm_mode),
        patrol_prompt: Arc::new(patrol_prompt),
        schema_path: Arc::new(schema_path),
        event_bus,
    };
    let app = build_router(state);
    let addr = "127.0.0.1:7101";
    info!("EasyVibe backend listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("绑定 7101 失败");
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    async fn test_state() -> AppState {
        let svc = MapService::new(vec![]);
        let (bus, _) = broadcast::channel(8);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let session_manager = SessionManager::new(tx);
        // 巡检槽位用内存库（测试不落盘）
        let db = easyvibe_db::Database::connect_memory().await.unwrap();
        let health_repo = Arc::new(easyvibe_db::SqliteHealthRepository::new(db.pool().clone()));
        let patrol_service = Arc::new(easyvibe_ai_agent::PatrolService::new(health_repo.clone()));
        let settings_repo = Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone()));
        let task_repo = Arc::new(easyvibe_db::SqliteTaskRepository::new(db.pool().clone()));
        let approval_repo = Arc::new(easyvibe_db::SqliteApprovalRepository::new(db.pool().clone()));
        let executor = task_exec::TaskExecutor::new(
            task_repo.clone(),
            approval_repo.clone(),
            session_manager.clone(),
            svc.clone(),
            Arc::new("框架".into()),
            Arc::new("true".into()),
            Arc::new(vec![]),
        );
        let cipher = easyvibe_common::SecretCipher::from_hex_key(&"ab".repeat(32)).unwrap();
        AppState {
            map_service: svc,
            session_manager,
            prompt_template: Arc::new("test".into()),
            agent_command: Arc::new("true".into()),
            agent_args: Arc::new(vec![]),
            patrol_service,
            health_repo,
            settings_repo,
            cipher: Arc::new(cipher),
            task_repo,
            approval_repo,
            executor,
            llm_mode: Arc::new(LlmMode::Stub),
            patrol_prompt: Arc::new("test".into()),
            schema_path: Arc::new("schema.json".into()),
            event_bus: bus,
        }
    }

    #[tokio::test]
    async fn health_ok() {
        let app = build_router(test_state().await);
        let resp = app.oneshot(axum::http::Request::get("/api/health").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_repo_404() {
        let app = build_router(test_state().await);
        let resp = app.oneshot(axum::http::Request::get("/api/repos/nope/map").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }
}
