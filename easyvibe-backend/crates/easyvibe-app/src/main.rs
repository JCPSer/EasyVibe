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
use easyvibe_ai_agent::QaClient as _;
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
    let session = st
        .session_manager
        .start_induction(&repo.id, &repo.root, &st.prompt_template, &st.agent_command, &st.agent_args)
        .await?;
    Ok((axum::http::StatusCode::ACCEPTED, Json(session)).into_response())
}

/// 触发巡检（M2-4）：Supervisor 直调 LLM，产出新地图原子写回 + 健康历史落库。
/// 写互斥：与归纳共用 SessionManager 的单会话纪律（try_register 拒绝并发地图写）
async fn start_patrol(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    if *st.llm_mode == LlmMode::Anthropic && std::env::var("EASYVIBE_LLM_API_KEY").is_err() {
        return Err(ApiError::BadRequest("未配置 EASYVIBE_LLM_API_KEY".into()).into());
    }
    let run_id = format!("patrol-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
    st.session_manager
        .try_register(SessionStatusChanged {
            repo: repo.id.clone(),
            session_id: run_id.clone(),
            status: easyvibe_api_types::SessionStatus::Running,
        })
        .await?;
    let snap = match st.map_service.load_map(&repo).await {
        Ok(s) => s,
        Err(e) => {
            st.session_manager
                .note_status(SessionStatusChanged {
                    repo: repo.id.clone(),
                    session_id: run_id.clone(),
                    status: easyvibe_api_types::SessionStatus::Failed,
                })
                .await;
            return Err(e.into());
        }
    };

    // 异步执行；状态经 session.statusChanged 上报（session_id = patrol run id）
    let st2 = st.clone();
    let repo2 = repo.clone();
    let run_id_task = run_id.clone();
    tokio::spawn(async move {
        let result: Result<easyvibe_ai_agent::PatrolSummary, ApiError> = match *st2.llm_mode {
            LlmMode::Stub => {
                let llm = easyvibe_ai_agent::StubLlmClient::new();
                st2.patrol_service
                    .run(Some(run_id_task.clone()), &repo2.id, &repo2.root, &snap.json, &st2.patrol_prompt, &st2.schema_path, &llm)
                    .await
            }
            LlmMode::Anthropic => {
                let llm = easyvibe_ai_agent::AnthropicClient::new(
                    &std::env::var("EASYVIBE_LLM_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".into()),
                    &std::env::var("EASYVIBE_LLM_API_KEY").unwrap_or_default(),
                    &std::env::var("EASYVIBE_LLM_MODEL").unwrap_or_else(|_| "claude-sonnet-4-5".into()),
                );
                st2.patrol_service
                    .run(Some(run_id_task.clone()), &repo2.id, &repo2.root, &snap.json, &st2.patrol_prompt, &st2.schema_path, &llm)
                    .await
            }
        };
        let status = match &result {
            Ok(_) => easyvibe_api_types::SessionStatus::Succeeded,
            Err(_) => easyvibe_api_types::SessionStatus::Failed,
        };
        st2.session_manager
            .note_status(SessionStatusChanged { repo: repo2.id, session_id: run_id_task, status })
            .await;
        if let Err(e) = result {
            tracing::warn!("[patrol] 失败: {e}（错误已入 patrol_runs.error）");
        }
    });

    Ok((axum::http::StatusCode::ACCEPTED, Json(serde_json::json!({ "started": true, "runId": run_id }))).into_response())
}

/// 健康历史：巡检运行列表（域 2 的第一个读接口）
async fn list_patrol_runs(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    use easyvibe_db::HealthRepository as _;
    let runs = st.health_repo.list_runs(&id, 20).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": runs })).into_response())
}

// ---------- M2-5：入口对话（F2）+ 存为视图（F1b） ----------

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
            let llm = easyvibe_ai_agent::AnthropicClient::new(
                &std::env::var("EASYVIBE_LLM_BASE_URL").unwrap_or_else(|_| "https://api.anthropic.com".into()),
                &std::env::var("EASYVIBE_LLM_API_KEY").unwrap_or_default(),
                &std::env::var("EASYVIBE_LLM_MODEL").unwrap_or_else(|_| "claude-sonnet-4-5".into()),
            );
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

    // agent 配置：命令/参数/提示词模板均可环境变量覆盖（测试可用 stub 命令）
    let agent_command = std::env::var("EASYVIBE_AGENT_CMD").unwrap_or_else(|_| "claude".into());
    let agent_args: Vec<String> = std::env::var("EASYVIBE_AGENT_ARGS")
        .unwrap_or_else(|_| "-p".into())
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

    let state = AppState {
        map_service,
        session_manager,
        prompt_template: Arc::new(prompt_template),
        agent_command: Arc::new(agent_command),
        agent_args: Arc::new(agent_args),
        patrol_service,
        health_repo,
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
        // 巡检槽位用内存库（测试不落盘）
        let db = easyvibe_db::Database::connect_memory().await.unwrap();
        let health_repo = Arc::new(easyvibe_db::SqliteHealthRepository::new(db.pool().clone()));
        let patrol_service = Arc::new(easyvibe_ai_agent::PatrolService::new(health_repo.clone()));
        AppState {
            map_service: svc,
            session_manager: SessionManager::new(tx),
            prompt_template: Arc::new("test".into()),
            agent_command: Arc::new("true".into()),
            agent_args: Arc::new(vec![]),
            patrol_service,
            health_repo,
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
