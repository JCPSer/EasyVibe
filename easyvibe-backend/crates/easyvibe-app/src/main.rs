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
    pub conversation_repo: Arc<easyvibe_db::SqliteConversationRepository>,
    /// 会话写串行化（审查 🟡1）：append+compact 是 read-modify-write，SQLite 语句原子
    /// 不保证这段复合操作；发送中点"压缩上下文"是 UI 允许的真实并发
    pub chat_lock: Arc<tokio::sync::Mutex<()>>,
    pub executor: Arc<task_exec::TaskExecutor>,
    /// S1-3：harness 单一事实源（插槽内核；恢复默认后热换，chat 与 executor 共用）
    pub harness: Arc<tokio::sync::RwLock<task_exec::Harness>>,
    pub llm_mode: Arc<LlmMode>,
    pub patrol_prompt: Arc<String>,
    /// 子图分析提示词模板（含 <REPO_ROOT>/<MODULE_ID>/<MODULE_JSON> 占位）
    pub submap_prompt: Arc<String>,
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
    /// S2：地图保鲜状态变化（git 有新提交而地图未更新——下游对话/建议/健康分全是假数据自信工作）
    Freshness { repo: String, status: String, latest_commit_at: Option<i64>, commits_since_map: Option<i64> },
    /// 改进#2：agent 过程直播——会话 stdout 行（子图分析/任务执行中的"它在干嘛"）
    SessionOutput { session_id: String, line: String },
}

pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/repos", get(list_repos).post(add_repo))
        .route("/repos/{id}", axum::routing::delete(remove_repo))
        .route("/repos/{id}/map", get(get_map))
        .route("/repos/{id}/freshness", get(get_freshness))
        .route("/repos/{id}/modules/{module_id}/health-history", get(get_health_history))
        .route("/repos/{id}/modules/{module_id}/analyze-submap", axum::routing::post(analyze_submap))
        .route("/repos/{id}/growth", get(get_growth))
        .route("/repos/{id}/progress", get(get_progress))
        .route("/repos/{id}/modules/{module_id}", get(get_submap))
        .route("/repos/{id}/reinduce", axum::routing::post(start_reinduce))
        .route("/repos/{id}/patrol", axum::routing::post(start_patrol))
        .route("/repos/{id}/patrol-runs", get(list_patrol_runs))
        .route("/repos/{id}/health-dashboard", get(get_health_dashboard))
        .route("/repos/{id}/git/status", get(get_git_status))
        .route("/repos/{id}/git/log", get(get_git_log))
        .route("/repos/{id}/git/commit", axum::routing::post(post_git_commit))
        .route("/repos/{id}/git/pull", axum::routing::post(post_git_pull))
        .route("/repos/{id}/git/push", axum::routing::post(post_git_push))
        .route("/repos/{id}/git/discard", axum::routing::post(post_git_discard))
        .route("/repos/{id}/git/commit-message", axum::routing::post(post_git_commit_message))
        .route("/repos/{id}/sessions/{sid}/kill", axum::routing::post(post_session_kill))
        .route("/repos/{id}/tasks/{tid}/kill", axum::routing::post(post_task_kill))
        .route("/repos/{id}/chat", get(get_chat).post(chat))
        .route("/repos/{id}/conversations", get(list_conversations).post(create_conversation))
        .route("/repos/{id}/conversations/{cid}", axum::routing::put(rename_conversation).delete(delete_conversation))
        .route("/repos/{id}/chat/compact", axum::routing::post(compact_chat))
        .route("/repos/{id}/chat/reset", axum::routing::post(reset_chat))
        .route("/repos/{id}/views", get(list_views).post(save_view))
        .route("/repos/{id}/views/{slug}", axum::routing::delete(delete_view))
        .route("/repos/{id}/tasks", get(list_tasks).post(create_task))
        .route("/repos/{id}/tasks/{tid}/decide", axum::routing::post(decide_task))
        .route("/repos/{id}/tasks/{tid}/approvals", get(list_task_approvals))
        .route("/repos/{id}/tasks/{tid}/diff", get(get_task_diff))
        .route("/repos/{id}/suggest", axum::routing::post(suggest))
        .route("/settings", get(list_settings))
        .route("/settings/set", axum::routing::put(put_setting))
        .route("/settings/{scope}/{key}", axum::routing::delete(delete_setting))
        .route("/harness", get(get_harness))
        .route("/diagnostics", get(export_diagnostics))
        .route("/harness/reset", axum::routing::post(reset_harness))
        .with_state(state.clone());

    let router = Router::new()
        .nest("/api", api)
        .route("/ws", get(ws_handler))
        // D5：桌面壳（Tauri v2 WebView）固定源。跨源请求带此 Origin 时放行
        // （CORS 层同时只回这一个源的 Access-Control-Allow-Origin）；
        // evil 页面无法伪造 Origin，Y6 对其它 cross-site 的拦截不受影响。
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(tower_http::cors::AllowOrigin::exact(
                    TAURI_ORIGIN.parse().expect("合法 origin"),
                ))
                .allow_methods([axum::http::Method::GET, axum::http::Method::POST, axum::http::Method::DELETE])
                .allow_headers(tower_http::cors::Any),
        )
        // Y6 清债：跨站请求防护—— evil 页面可对 127.0.0.1 发 simple POST（无 CORS 拦截）。
        // 现代浏览器带 Sec-Fetch-Site 头：cross-site 一律拒（health 放行供连通探测）；
        // 桌面壳 Origin 一律放（WS 握手与 preflight 都带 Origin，无 Sec-Fetch 维度可依赖）。
        .layer(axum::middleware::from_fn(|req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| async move {
            let (parts, body) = req.into_parts();
            let is_health = parts.uri.path() == "/api/health";
            let is_tauri = parts.headers.get("origin").and_then(|v| v.to_str().ok()) == Some(TAURI_ORIGIN);
            let cross_site = parts.headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) == Some("cross-site");
            if cross_site && !is_health && !is_tauri {
                let mut resp = axum::http::Response::new(axum::body::Body::from("cross-site request blocked"));
                *resp.status_mut() = axum::http::StatusCode::FORBIDDEN;
                return Ok::<_, std::convert::Infallible>(resp);
            }
            Ok::<_, std::convert::Infallible>(next.run(axum::http::Request::from_parts(parts, body)).await)
        }))
        .with_state(state);

    // D5：桌面壳同源托管——EASYVIBE_STATIC_DIR 指向渲染器构建产物（dist）时，
    // / 与未命中路径回落到静态资源；前端 fetch('/api/...') 与 /ws 全部同源，
    // 桌面 WebView 直接加载 http://127.0.0.1:{port}，CORS/跨站问题整体消失。
    match std::env::var("EASYVIBE_STATIC_DIR").ok().filter(|d| !d.is_empty()) {
        Some(dir) => {
            use tower_http::services::ServeDir;
            router.fallback_service(ServeDir::new(dir).append_index_html_on_directories(true))
        }
        None => router,
    }
}

/// D5：Tauri v2 生产 WebView 的固定源（WKWebView 自定义协议映射为 http://tauri.localhost）。
/// 后端只信任这一个跨站源；前端在壳内以 http://127.0.0.1:{EASYVIBE_PORT} 直连。
const TAURI_ORIGIN: &str = "http://tauri.localhost";

/// Y7：/api/health 已有 version 字段——前端在 WS 重连（全量重同步点）时比对
/// 首次记录的版本，变化即提示"后端已更新，刷新页面"。此处仅加注释锚点，比对在前端。

async fn health() -> Json<ApiResponse<HealthResponse>> {
    Json(ApiResponse::ok(HealthResponse { status: "ok".into(), version: VERSION.into() }))
}

async fn list_repos(State(st): State<AppState>) -> Json<ApiResponse<Vec<RepoInfo>>> {
    let repos = st
        .map_service
        .repos().await
        .into_iter()
        .map(|r| RepoInfo { id: r.id, name: r.name, root: r.root.to_string_lossy().into_owned() })
        .collect();
    Json(ApiResponse::ok(repos))
}

// ---------- D5 应用内仓库管理：动态注册/注销（desktop-repos 文件由后端独占） ----------

fn data_dir() -> std::path::PathBuf {
    std::env::var("EASYVIBE_DATA_DIR").map(Into::into).unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        std::path::PathBuf::from(format!("{home}/.easyvibe"))
    })
}

fn desktop_repos_file() -> std::path::PathBuf {
    data_dir().join("desktop-repos")
}

/// 读 desktop-repos 持久化文件（每行一个仓库根路径；# 开头为注释）
fn read_desktop_repos() -> Vec<std::path::PathBuf> {
    std::fs::read_to_string(desktop_repos_file())
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(Into::into)
        .collect()
}

/// 全量重写 desktop-repos（注销后保持一致）
fn write_desktop_repos(roots: &[std::path::PathBuf]) {
    let content = roots.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n") + "\n";
    if let Err(e) = std::fs::write(desktop_repos_file(), content) {
        tracing::warn!("desktop-repos 持久化失败: {e}");
    }
}

#[derive(serde::Deserialize)]
struct AddRepoRequest {
    path: String,
}

async fn add_repo(State(st): State<AppState>, Json(body): Json<AddRepoRequest>) -> Result<Response, AppError> {
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
    ));
    info!("[repo-add] 动态注册 {} -> {}", repo.id, repo.root.display());
    Ok(Json(serde_json::json!({ "success": true, "data": { "id": repo.id, "name": repo.name, "root": repo.root.to_string_lossy() } })).into_response())
}

async fn remove_repo(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    if !st.map_service.remove_repo(&id).await {
        return Err(AppError(ApiError::NotFound(format!("仓库 {id} 未挂载"))));
    }
    let remaining: Vec<_> = st.map_service.repos().await.into_iter().map(|r| r.root).collect();
    write_desktop_repos(&remaining);
    info!("[repo-remove] 注销 {}", id);
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

async fn get_map(State(st): State<AppState>, Path(id): Path<String>, headers: axum::http::HeaderMap) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let snap = st.map_service.load_map(&repo).await?;
    // Y1 清债：ETag 条件请求——内容哈希已有，浏览器重连重同步时 If-None-Match 命中即 304
    // （body 不传输；前端零改动，浏览器 HTTP 缓存自动处理）
    let etag = format!("\"{}\"", snap.content_hash);
    if headers.get("if-none-match").and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        let mut resp = axum::http::Response::new(axum::body::Body::empty());
        *resp.status_mut() = axum::http::StatusCode::NOT_MODIFIED;
        resp.headers_mut().insert("etag", etag.parse().unwrap());
        return Ok(resp.into_response());
    }
    let mut resp = Json(snap.json).into_response();
    resp.headers_mut().insert("etag", etag.parse().unwrap());
    resp.headers_mut().insert("cache-control", "private, must-revalidate".parse().unwrap());
    Ok(resp)
}

async fn get_growth(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
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
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    Ok(Json(st.map_service.load_submap(&repo, &module_id).await?).into_response())
}

/// S2：地图保鲜状态（§13.4——stale 地图上的对话/建议/健康分全是假数据自信工作）
async fn get_freshness(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let snap = st.map_service.load_map(&repo).await?;
    let f = freshness::assess(&repo.root, &snap.json);
    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "status": f.status.as_str(),
            "mapGeneratedAt": f.map_generated_at,
            "latestCommitAt": f.latest_commit_at,
            "commitsSinceMap": f.commits_since_map,
        }
    }))
    .into_response())
}

/// S2：模块健康历史（趋势图数据面；module_health_history 自 M2-4 落库以来的第一个消费者）
async fn get_health_history(State(st): State<AppState>, Path((id, module_id)): Path<(String, String)>) -> Result<Response, AppError> {
    if !easyvibe_map::is_valid_id(&module_id) {
        return Err(ApiError::BadRequest(format!("非法模块 id: {module_id}")).into());
    }
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    use easyvibe_db::HealthRepository as _;
    let rows = st.health_repo.list_module_history(&id, &module_id, 20).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": rows })).into_response())
}

/// 归纳完成超时防线（实弹#4：FENJUE 首归纳产物/进度俱齐，但 CLI 不收尾、会话恒 Running，
/// 前端"归纳中"假卡住）：progress.json phase=done 且文件落盘超过 grace 秒 → 判 agent 已交付。
/// 用文件 mtime（系统时间基准）而非 progress.updated_at（RFC3339 带时区，秒级判定会被时区坑）。
fn progress_done_ago_secs(repo_root: &std::path::Path) -> Option<u64> {
    let path = repo_root.join(".easyvibe/map/progress.json");
    let content = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    if v["phase"].as_str() != Some("done") {
        return None;
    }
    let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
    Some(std::time::SystemTime::now().duration_since(modified).ok()?.as_secs())
}

/// S2.5：归纳进度（progress.json 透传——首归纳等待页显示真实阶段/百分比，
/// 不再只转圈；文件缺失（如巡检场景无 progress）返回 done 形状，前端不渲染进度）
async fn get_progress(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let path = repo.root.join(".easyvibe/map/progress.json");
    if !path.exists() {
        return Ok(Json(serde_json::json!({ "success": true, "data": null })).into_response());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| ApiError::Internal(format!("progress 读取失败: {e}")))?;
    let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| ApiError::Internal(format!("progress 解析失败: {e}")))?;
    Ok(Json(serde_json::json!({ "success": true, "data": v })).into_response())
}

/// 子图深入分析（试用反馈"子图加载失败"根因修复——v2.2 归纳不产子图，文件无人生产；
/// 此处把缺口变为能力：透明 agent 扫描模块文件产出子图，落盘 .easyvibe/modules/<id>.json，
/// 与归纳共用写互斥/会话机制。读时拉取无需 watcher，产出后重新展开即见）
async fn analyze_submap(State(st): State<AppState>, Path((id, module_id)): Path<(String, String)>) -> Result<Response, AppError> {
    if !easyvibe_map::is_valid_id(&module_id) {
        return Err(ApiError::BadRequest(format!("非法模块 id: {module_id}")).into());
    }
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let snap = st.map_service.load_map(&repo).await?;
    let module = snap.json["modules"]
        .as_array()
        .and_then(|ms| ms.iter().find(|m| m["id"].as_str() == Some(module_id.as_str())).cloned())
        .ok_or_else(|| ApiError::NotFound(format!("模块 {module_id} 不在主地图中")))?;
    // 子图提示词每次请求重读（产品内置协议迭代快——避免"改了提示词要重启后端"的叠加
    // （本次实弹：路径修正后的提示词因后端未重启而仍用旧版，DeskWar 两次分析空跑）。
    // 读取失败回退启动时装载的副本，绝不阻断
    let template = std::fs::read_to_string(
        std::env::var("EASYVIBE_SUBMAP_PROMPT_PATH").unwrap_or_else(|_| "easyvibe-module-submap-prompt.md".into()),
    )
    .unwrap_or_else(|_| st.submap_prompt.to_string());
    ensure_agent_available(&st)?;
    let prompt = template
        .replace("<REPO_ROOT>", &repo.root.to_string_lossy())
        .replace("<MODULE_ID>", &module_id)
        .replace("<MODULE_JSON>", &serde_json::to_string(&module).unwrap_or_default());
    let session = st
        .session_manager
        .start_induction(&repo.id, &repo.root, &prompt, &st.agent_command, &st.agent_args)
        .await?;
    info!("[submap] 模块 {} 子图分析会话 {} 已启动", module_id, session.session_id);
    Ok((axum::http::StatusCode::ACCEPTED, Json(session)).into_response())
}

/// 触发重新归纳（写路径，M2-3）：spawn 外部 agent 按 v2.2 协议执行，
/// 三通道（progress/growth.log/map.json）由 watcher 自动直播，前端无需轮询
async fn start_reinduce(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    ensure_agent_available(&st)?;
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
                // 实弹#4 防线：进度 100% 落盘超过 90s 但会话仍 Running（agent 已交付未自行退出）
                // → 按成功收尸解除"归纳中"假卡住。产物合法性由 watcher 校验保证（不出残图），
                // 进程资源由 kill_on_drop 在后端生命周期结束时兜底。巡逻无 progress.json，不误触。
                Some(_) => {
                    const GRACE_SECS: u64 = 90;
                    if let Some(ago) = progress_done_ago_secs(&repo2.root) {
                        if ago > GRACE_SECS {
                            tracing::warn!(
                                "[reinduce] 进度 100% 已落盘 {}s 但会话仍未退出——按成功收尸（agent 未自行退出，实弹#4）",
                                ago
                            );
                            let Some(s) = st2.session_manager.status_of(&repo2.id).await else { break };
                            st2.session_manager
                                .note_status(easyvibe_api_types::SessionStatusChanged {
                                    repo: repo2.id.clone(),
                                    session_id: s.session_id.clone(),
                                    status: easyvibe_api_types::SessionStatus::Succeeded,
                                })
                                .await;
                            break;
                        }
                    }
                }
                None => {}
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
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    if *st.llm_mode == LlmMode::Anthropic {
        ensure_agent_available(&st)?;
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
            // run_id 用时间戳独立生成，不复用 session_id——会话计数器在后端重启后归零，
            // 会与历史 patrol_runs 行主键碰撞导致落库失败（实弹：UNIQUE constraint failed）
            let st2 = st.clone();
            let repo2 = repo.clone();
            let run_id = format!("patrol-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
            let run_id_task = run_id.clone();
            let model = st.agent_command.to_string();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let Some(s) = st2.session_manager.status_of(&repo2.id).await else { continue };
                    if matches!(s.status, easyvibe_api_types::SessionStatus::Starting | easyvibe_api_types::SessionStatus::Running) { continue }
                    let result = async {
                        let snap = st2.map_service.load_map(&repo2).await?;
                        st2.patrol_service
                            .record_from_map(&run_id_task, &repo2.id, &model, &snap.json, s.status == easyvibe_api_types::SessionStatus::Succeeded, None)
                            .await
                    }
                    .await;
                    if let Err(e) = result {
                        tracing::warn!("[patrol] 健康历史落库失败: {e}");
                    }
                    break;
                }
            });
            Ok((axum::http::StatusCode::ACCEPTED, Json(serde_json::json!({ "started": true, "sessionId": session.session_id, "runId": run_id, "mode": "agent" }))).into_response())
        }
    }
}

/// CLI agent 可执行预检：spawn 路径的鉴权由 claude 自身配置（~/.claude/settings.json 的 env
/// 或进程环境变量）负责，与本进程的 EASYVIBE_LLM_API_KEY 无关——旧守卫把"DB 已配 key"
/// 的合法场景误判为未配置（chat 直调走 DB，patrol/reinduce 走 CLI spawn，两条链路配置源不同）。
fn ensure_agent_available(st: &AppState) -> Result<(), ApiError> {
    let cmd = &*st.agent_command;
    let name = cmd.rsplit('/').next().unwrap_or(cmd);
    let on_path = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| d.join(name).is_file()))
        .unwrap_or(false);
    if !on_path && !std::path::Path::new(cmd).is_file() {
        return Err(ApiError::BadRequest(format!("未找到 CLI agent `{cmd}`——请先安装并加入 PATH")));
    }
    Ok(())
}

/// 健康历史：巡检运行列表（域 2 的第一个读接口）
async fn list_patrol_runs(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    use easyvibe_db::HealthRepository as _;
    let runs = st.health_repo.list_runs(&id, 20).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": runs })).into_response())
}

/// M4-3 健康看板数据面：近 20 次巡检（含各自模块平均分）+ 最近一次成功巡检的模块明细。
/// 一次聚合查询代替前端 N×M 次 health-history 轮询（N 模块 × M 次巡检）。
async fn get_health_dashboard(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
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

// ---------- M4-4 Git 工作树 ----------

async fn get_git_status(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let s = git::status(&repo.root).await?;
    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "branch": s.branch, "upstream": s.upstream, "ahead": s.ahead, "behind": s.behind,
            "files": s.files.iter().map(|f| serde_json::json!({
                "status": f.status.to_string(), "path": f.path, "orig": f.orig, "adds": f.adds, "dels": f.dels,
            })).collect::<Vec<_>>(),
        },
    }))
    .into_response())
}

async fn get_git_log(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let limit = q.get("limit").and_then(|l| l.parse::<i64>().ok()).unwrap_or(30).clamp(1, 100);
    let rows = git::log(&repo.root, limit).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": rows })).into_response())
}

async fn post_git_commit(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let message = body["message"].as_str().unwrap_or_default();
    let short = git::commit_all(&repo.root, message).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "shortHash": short } })).into_response())
}

async fn post_git_pull(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    git::pull(&repo.root).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": true })).into_response())
}

async fn post_git_push(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    git::push(&repo.root).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": true })).into_response())
}

async fn post_git_discard(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let path = body["path"].as_str().ok_or_else(|| ApiError::BadRequest("缺少 path".into()))?;
    git::discard(&repo.root, path).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": true })).into_response())
}

/// M4-4 提交把关台：从任务上下文 + 影响面 AI 生成提交说明（Conventional Commits 单行）。
/// footer（EasyVibe-Task: <id>）由后端一并返回，提交时随说明写入，历史可反查任务。
#[derive(serde::Deserialize)]
struct CommitMessageRequest {
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    modules: Vec<String>,
    #[serde(default)]
    diff_stat: String,
}

async fn post_git_commit_message(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CommitMessageRequest>,
) -> Result<Response, AppError> {
    use easyvibe_db::TaskRepository as _;
    let _repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;

    let (task_ctx, footer) = match &body.task_id {
        Some(tid) => {
            let t = st.task_repo.get(tid).await?.ok_or_else(|| ApiError::NotFound(format!("任务 {tid} 不存在")))?;
            let summary = t.result.as_deref().and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok())
                .and_then(|v| v["result"]["summary"].as_str().map(str::to_string));
            let ctx = format!("任务 {}：{}\n需求描述：{}\n执行总结：{}", t.id, t.title, t.description, summary.unwrap_or_else(|| "（无）".into()));
            (ctx, Some(format!("EasyVibe-Task: {}", t.id)))
        }
        None => (String::new(), None),
    };

    let message = match *st.llm_mode {
        LlmMode::Stub => format!("chore({}): EasyVibe 汇总提交", if body.modules.is_empty() { "repo" } else { &body.modules[0] }),
        LlmMode::Anthropic => {
            let cfg = resolve_llm(&st, &id, "chat").await;
            if cfg.api_key.is_empty() {
                return Err(AppError(ApiError::BadRequest("未配置 LLM API key（设置面板或 EASYVIBE_LLM_API_KEY）".into())));
            }
            let system = "你是提交说明撰写助手。根据给定上下文输出一条符合 Conventional Commits 的中文提交说明：仅一行 subject（≤60 字），格式 type(scope): 描述，type 取 fix/feat/refactor/chore/docs 之一，scope 取主要模块名。只输出这一行，不要任何解释、引号或多余内容。";
            let user = format!(
                "改动涉及模块：{}\n任务上下文：\n{}\n变更统计（git diff --stat）：\n{}\n\n提交说明：",
                body.modules.join("、"),
                if task_ctx.is_empty() { "（无关联任务）" } else { &task_ctx },
                body.diff_stat
            );
            let llm = easyvibe_ai_agent::AnthropicClient::new(&cfg.base_url, &cfg.api_key, &cfg.model);
            let out = easyvibe_ai_agent::LlmClient::chat(&llm, easyvibe_ai_agent::ChatRequest { system, user: &user, images: &[] }).await?;
            out.text.trim().lines().next().unwrap_or_default().trim().to_string()
        }
    };
    if message.is_empty() {
        return Err(AppError(ApiError::Internal("LLM 未产出提交说明".into())));
    }
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": message, "footer": footer } })).into_response())
}

/// P0 审查后端#1：终止指定会话（归纳/巡检/子图分析/任务执行同一通道）。
/// 已终态返回 409；外部自注册会话（无终止通道）返回 409。
async fn post_session_kill(State(st): State<AppState>, Path((id, sid)): Path<(String, String)>) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    st.session_manager.kill(&sid).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": true })).into_response())
}

/// 按任务终止：解析任务 → 会话 → kill（任务卡的「终止」按钮走这里）
async fn post_task_kill(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>) -> Result<Response, AppError> {
    use easyvibe_db::TaskRepository as _;
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let task = st.task_repo.get(&tid).await?.ok_or_else(|| ApiError::NotFound(format!("任务 {tid} 不存在")))?;
    let sid = task.session_id.ok_or_else(|| ApiError::BadRequest(format!("任务 {tid} 无关联会话（未开始执行）")))?;
    st.session_manager.kill(&sid).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": true })).into_response())
}

// ---------- M2-5：入口对话（F2）+ 存为视图（F1b） ----------

// ---------- M3-1：配置体系（backend-design §10） ----------

use easyvibe_db::{SettingRow, SettingsRepository as _, TaskRepository as _};

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

/// Y3：一键诊断导出——把"用户报障口头描述"变成"导出一个文件"
/// 最近 200 行日志 + 后端版本 + 各表计数（settings 的加密值剔除）
async fn export_diagnostics(State(st): State<AppState>) -> Result<Response, AppError> {
    let dir = std::env::var("EASYVIBE_DATA_DIR").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        format!("{home}/.easyvibe")
    });
    let log_path = format!("{dir}/logs/easyvibe.log");
    let logs = std::fs::read_to_string(&log_path)
        .map(|t| t.lines().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"))
        .unwrap_or_else(|_| "（日志文件不可读）".into());
    use easyvibe_db::{ApprovalRepository as _, HealthRepository as _, TaskRepository as _};
    let tasks = st.task_repo.list(st.map_service.repos().await.first().map(|r| r.id.as_str()).unwrap_or(""), 100).await.unwrap_or_default();
    let runs = st.health_repo.list_runs(st.map_service.repos().await.first().map(|r| r.id.as_str()).unwrap_or(""), 20).await.unwrap_or_default();
    let aps = tasks.iter().take(10).map(|t| t.id.clone()).collect::<Vec<_>>();
    let body = serde_json::json!({
        "version": VERSION,
        "llm_mode": format!("{:?}", *st.llm_mode),
        "repos": st.map_service.repos().await.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
        "task_counts": {
            "by_status": tasks.iter().fold(serde_json::json!({}), |mut acc, t| {
                let k = t.status.clone();
                let o = acc.as_object_mut().unwrap();
                *o.entry(k).or_insert(serde_json::json!(0)) = serde_json::json!(o.get(&t.status).and_then(|v| v.as_i64()).unwrap_or(0) + 1);
                acc
            }),
        },
        "recent_runs": runs.iter().take(5).map(|r| serde_json::json!({"id": r.id, "status": r.status, "archScore": r.arch_score})).collect::<Vec<_>>(),
        "recent_logs": logs,
    });
    let _ = aps;
    Ok(Json(serde_json::json!({ "success": true, "data": body })).into_response())
}

/// S1-3：harness 状态（manifest + 文件清单）——S3 管理界面的数据面
async fn get_harness(State(st): State<AppState>) -> Result<Response, AppError> {
    let h = st.harness.read().await;
    let mut files: Vec<String> = vec![];
    if let Ok(entries) = std::fs::read_dir(&h.dir) {
        for e in entries.flatten() {
            if e.path().is_file() {
                files.push(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    files.sort();
    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "dir": h.dir.to_string_lossy(),
            "manifest": {
                "id": h.manifest.id, "version": h.manifest.version, "builtin": h.manifest.builtin,
                "routeRules": h.manifest.route_rules,
                "skills": { "userEntry": h.manifest.skills.user_entry, "transparent": h.manifest.skills.transparent },
            },
            "frameworkNeutralized": h.framework_transparent.contains("透明执行模式"),
            "userEntrySkillCount": h.user_entry_skills.len(),
            "files": files,
        }
    }))
    .into_response())
}

/// S1-3：恢复默认——现有用户层整体改名备份（.backup-<ts>），出厂底账全量重铺，
/// 装载后热换单一事实源（chat 与 executor 立即生效，无需重启）
async fn reset_harness(State(st): State<AppState>) -> Result<Response, AppError> {
    let dir = task_exec::harness_dir();
    if dir.exists() {
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let backup = dir.with_file_name(format!("harness.backup-{ts}"));
        std::fs::rename(&dir, &backup).map_err(|e| ApiError::Internal(format!("harness 备份失败: {e}")))?;
    }
    task_exec::deploy_builtin_force(&dir)?;
    let fresh = task_exec::load_harness()?;
    let version = fresh.manifest.version.clone();
    *st.harness.write().await = fresh;
    info!("[harness] 已恢复默认 v{}", version);
    Ok(Json(serde_json::json!({ "success": true, "data": { "version": version } })).into_response())
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

/// M4-3：任务完整 diff（按需读取，不随任务列表载荷）——development_docs 归档中的 diffFull；
/// 无归档/无 diff 返回 diff=null（调用方展示"无变更"）
async fn get_task_diff(State(st): State<AppState>, Path((id, tid)): Path<(String, String)>) -> Result<Response, AppError> {
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
async fn suggest(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
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
    /// M4-2：任务←→会话关联（对话升级/工作台聚合）
    #[serde(default)]
    conversation_id: Option<String>,
}

async fn create_task(State(st): State<AppState>, Path(id): Path<String>, Json(body): Json<CreateTaskRequest>) -> Result<Response, AppError> {
    use easyvibe_db::TaskRepository as _;
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
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
        trust: match body.trust.as_str() {
            "auto" => "auto".into(),
            "supervised" => "supervised".into(),
            _ => "manual".into(),
        },
        error: None,
        session_id: None,
        gate: None,
        conversation_id: body.conversation_id,
        prompt_tokens: None,
        completion_tokens: None,
        result: None,
        base_head: None,
        created_at: now.clone(),
        updated_at: now,
    };
    st.task_repo.create(&row).await?;
    // M3-3：入队执行（harness 引擎；并发上限 4，审批门 M3-4 接入）
    st.executor.clone().enqueue_pending(Some(&repo_id)).await;
    Ok((axum::http::StatusCode::CREATED, Json(serde_json::json!({ "success": true, "data": { "id": task_id } }))).into_response())
}

async fn list_tasks(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    use easyvibe_db::TaskRepository as _;
    // M4-2：?conv=<id> 时会话级过滤（工作台影响面/计划进度）
    let tasks = match q.get("conv") {
        Some(cid) => st.task_repo.list_by_conversation(cid).await?,
        None => st.task_repo.list(&id, 50).await?,
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
            "result": t.result.as_deref().and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok()),
            "createdAt": t.created_at, "updatedAt": t.updated_at,
        }))
        .collect();
    Ok(Json(serde_json::json!({ "success": true, "data": items })).into_response())
}

// ---------- M3-5：会话持久化 + auto-compact（backend-design §10a / §11 🟡4/5/6） ----------

use easyvibe_ai_agent::compaction;
use easyvibe_db::{ConversationMessageRow, ConversationRepository as _};

/// 近期窗口原文保留的消息条数（3 轮问答不动，三层策略第 1 层）
const KEEP_RECENT_MESSAGES: usize = 6;
const DEFAULT_CONTEXT_BUDGET: i64 = 256_000; // §10 #2：默认 256K，高级设置可调
const DEFAULT_COMPACT_THRESHOLD: i64 = 80;   // §10a：触发 80% → 压到 40%

/// 高级设置解析：仓库行覆盖全局行（同 resolve_llm 的两级哲学）
async fn resolve_adv_i64(st: &AppState, repo_id: &str, key: &str, default: i64) -> i64 {
    for scope in [repo_id, "global"] {
        if let Ok(Some(row)) = st.settings_repo.get(scope, key).await {
            if let Ok(v) = serde_json::from_str::<i64>(&row.value) { return v; }
            if let Ok(v) = serde_json::from_str::<f64>(&row.value) { return v as i64; }
        }
    }
    default
}

/// 未压缩消息折叠为 (q, a) 对（容错奇数/乱序；系统消息不参与）
fn fold_pairs(messages: &[ConversationMessageRow]) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut pending_q: Option<String> = None;
    for m in messages {
        match m.role.as_str() {
            "user" => pending_q = Some(m.content.clone()),
            "assistant" => {
                if let Some(q) = pending_q.take() {
                    pairs.push((q, m.content.clone()));
                }
            }
            _ => {}
        }
    }
    pairs
}

/// 压缩执行（auto 与手动共用）：返回留痕消息（"上下文已压缩：82%→34%"）。
/// 存储分离（§11 🟡5）：水位前消息标 compacted（原文保留可回放），运行态只剩摘要+窗口；
/// 摘要必带会话状态（§11 🟡6）：compact_stub/compact_with_llm 的结构化段落保证。
/// 调用方约定：auto 路径（chat 第 5 步）必须吞错降级——压缩绝不可打断已成功的对话（审查 🔴）；
/// 手动路径（compact_chat）传播错误，那里没有已落库的回答可损失。
async fn maybe_compact(st: &AppState, repo_id: &str, conv_id: Option<&str>, budget: i64, threshold: i64, force: bool) -> Result<Option<String>, ApiError> {
    // M4-2：压缩按会话（缺省=该仓库最近活跃会话）
    let conv = resolve_conv(st, repo_id, conv_id).await?;
    let fresh = st.conversation_repo.list_uncompacted(&conv.id).await?;
    // 触发口径 = 真实装配口径：未压缩窗口 + 既有摘要（摘要自身增长也会再触发，护栏闭环）
    let total: i64 = fresh.iter().map(|m| m.tokens).sum::<i64>()
        + conv.summary.as_deref().map(easyvibe_ai_agent::estimate_tokens).unwrap_or(0);
    if !force && !compaction::needs_compaction(total, budget, threshold) {
        return Ok(None);
    }
    let Some(wm) = compaction::compaction_watermark(&fresh, KEEP_RECENT_MESSAGES) else {
        return Ok(None); // 不足一个窗口不压
    };
    let old: Vec<ConversationMessageRow> = fresh.iter().filter(|m| m.id <= wm).cloned().collect();
    if old.is_empty() {
        return Ok(None);
    }
    let result = match *st.llm_mode {
        LlmMode::Stub => compaction::compact_stub(conv.summary.as_deref(), &old, budget, total),
        LlmMode::Anthropic => {
            let cfg = resolve_llm(st, repo_id, "chat").await;
            if cfg.api_key.is_empty() {
                // 无 key 退化 stub（诚实标注），压缩不可用不该打断对话
                compaction::compact_stub(conv.summary.as_deref(), &old, budget, total)
            } else {
                let llm = easyvibe_ai_agent::AnthropicClient::new(&cfg.base_url, &cfg.api_key, &cfg.model);
                compaction::compact_with_llm(&llm, conv.summary.as_deref(), &old, budget, total).await?
            }
        }
    };
    st.conversation_repo
        .apply_compaction(&conv.id, result.before_id, &result.summary, result.prompt_tokens, result.completion_tokens)
        .await?;
    let trace = format!("上下文已压缩：{}%→{}%", result.before_pct, result.after_pct);
    st.conversation_repo
        .append_message(&conv.id, "system", &trace, easyvibe_ai_agent::estimate_tokens(&trace))
        .await?;
    info!("[chat] {} 压缩 {}%→{}%（水位 {}）", repo_id, result.before_pct, result.after_pct, result.before_id);
    Ok(Some(trace))
}

/// 恢复对话（R1 分页：?before=<id>&limit=50——切换页签/重启/刷新均恢复；
/// hasMore 为真时前端给"加载更早"入口，不再全表读）

/// M4-2 多会话：解析目标会话（缺省=该仓库最近活跃的会话）
async fn resolve_conv(st: &AppState, repo: &str, conv: Option<&str>) -> Result<easyvibe_db::ConversationRow, ApiError> {
    match conv {
        Some(cid) => {
            let rows = st.conversation_repo.list_by_repo(repo).await?;
            rows.into_iter().find(|c| c.id == cid).ok_or_else(|| ApiError::NotFound(format!("会话 {cid} 不存在于仓库 {repo}")))
        }
        None => st.conversation_repo.get_or_create(repo).await,
    }
}

/// 会话摘要（AionUI TConversationRuntimeSummary 精简版）：state + pending 审批数 + 消息数
async fn conversation_summary(st: &AppState, c: &easyvibe_db::ConversationRow) -> serde_json::Value {
    let tasks = st.task_repo.list_by_conversation(&c.id).await.unwrap_or_default();
    let pending = tasks.iter().filter(|t| t.status == "awaiting_approval").count();
    let running = tasks.iter().filter(|t| t.status == "running" || t.status == "pending").count();
    let msg_count = st.conversation_repo.count_messages(&c.id).await.unwrap_or(0);
    serde_json::json!({
        "id": c.id, "title": c.title, "repo": c.repo,
        "createdAt": c.created_at, "updatedAt": c.updated_at,
        "messageCount": msg_count,
        "usage": { "promptTokens": c.prompt_tokens, "completionTokens": c.completion_tokens },
        "runtime": {
            "state": if running > 0 { "running" } else if pending > 0 { "waiting_confirmation" } else { "idle" },
            "pendingConfirmations": pending,
            "runningTasks": running,
        },
    })
}

async fn list_conversations(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let convs = st.conversation_repo.list_by_repo(&id).await?;
    let mut items = Vec::new();
    for c in &convs {
        items.push(conversation_summary(&st, c).await);
    }
    Ok(Json(serde_json::json!({ "success": true, "data": items })).into_response())
}

#[derive(serde::Deserialize)]
struct CreateConversationRequest {
    #[serde(default)]
    title: Option<String>,
}

async fn create_conversation(State(st): State<AppState>, Path(id): Path<String>, Json(body): Json<CreateConversationRequest>) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let cid = format!("chat:{id}:{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
    let conv = st.conversation_repo.create(&cid, &id, body.title.as_deref()).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": conversation_summary(&st, &conv).await })).into_response())
}

#[derive(serde::Deserialize)]
struct RenameConversationRequest {
    title: String,
}

async fn rename_conversation(State(st): State<AppState>, Path((id, cid)): Path<(String, String)>, Json(body): Json<RenameConversationRequest>) -> Result<Response, AppError> {
    if body.title.trim().is_empty() {
        return Err(AppError(ApiError::BadRequest("会话名不能为空".into())));
    }
    resolve_conv(&st, &id, Some(&cid)).await?;
    st.conversation_repo.rename(&cid, body.title.trim()).await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

async fn delete_conversation(State(st): State<AppState>, Path((id, cid)): Path<(String, String)>) -> Result<Response, AppError> {
    resolve_conv(&st, &id, Some(&cid)).await?;
    let convs = st.conversation_repo.list_by_repo(&id).await?;
    if convs.len() <= 1 {
        return Err(AppError(ApiError::BadRequest("每个仓库至少保留一个会话".into())));
    }
    st.conversation_repo.delete(&cid).await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

async fn get_chat(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    // M4-2 多会话：?conv=<id> 选择会话（缺省=最近活跃）
    let conv = resolve_conv(&st, &id, q.get("conv").map(|v| v.as_str())).await?;
    let before = q.get("before").and_then(|v| v.parse::<i64>().ok());
    let limit: i64 = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50).clamp(1, 200);
    let messages = st.conversation_repo.list_messages(&conv.id, before, limit).await?;
    let has_more = messages.len() as i64 == limit;
    // 内联审批数据源：该会话关联任务中等待审批的门（AionUI 内联审批卡模式）
    let pending_approvals: Vec<serde_json::Value> = st.task_repo.list_by_conversation(&conv.id).await.unwrap_or_default()
        .into_iter()
        .filter(|t| t.status == "awaiting_approval")
        .map(|t| serde_json::json!({ "taskId": t.id, "title": t.title, "gate": t.gate }))
        .collect();
    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "conversation": { "id": conv.id, "title": conv.title },
            "summary": conv.summary,
            "messages": messages,
            "hasMore": has_more,
            "pendingApprovals": pending_approvals,
            "usage": { "promptTokens": conv.prompt_tokens, "completionTokens": conv.completion_tokens },
        }
    }))
    .into_response())
}

#[derive(serde::Deserialize)]
struct ChatHttpRequest {
    message: String,
    /// 图片附件：dataURL 数组（随消息发给视觉模型；不持久化原文，库中只留占位）
    #[serde(default)]
    images: Vec<String>,
    /// D9 @模块：用户显式钉住的模块 id 列表（只作上下文提示，不参与过滤）
    #[serde(default)]
    module_refs: Vec<String>,
    /// M4-2 多会话：目标会话 id（缺省=该仓库最近活跃会话）
    #[serde(default)]
    conv: Option<String>,
}

/// 入口对话（M2-5 + M3-5 持久化）：服务端是会话事实源——
/// 用户消息落库 → 运行态上下文=摘要+未压缩窗口 → 问答 → 回答落库+token 记账 → auto-compact 检查
async fn chat(State(st): State<AppState>, Path(id): Path<String>, Json(body): Json<ChatHttpRequest>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    if body.message.trim().is_empty() {
        return Err(AppError(ApiError::BadRequest("消息不能为空".into())));
    }
    let snap = st.map_service.load_map(&repo).await?;
    // 会话写串行化（审查 🟡1）：持锁覆盖 落库→装配→compact 全段
    let _guard = st.chat_lock.lock().await;
    let conv = resolve_conv(&st, &id, body.conv.as_deref()).await?;

    // 1) 用户消息落库（会话持久化：历史不再只活在前端 state）
    let persisted = if body.images.is_empty() {
        body.message.clone()
    } else {
        format!("{}
[图片附件 {} 张（未持久化，重开对话后不可见）]", body.message, body.images.len())
    };
    st.conversation_repo
        .append_message(&conv.id, "user", &persisted, easyvibe_ai_agent::estimate_tokens(&persisted))
        .await?;

    // 2) 运行态上下文：未压缩消息（近期窗口原文 + 此前由摘要代表）
    let fresh = st.conversation_repo.list_uncompacted(&conv.id).await?;
    let pairs = fold_pairs(&fresh);

    // 3) 问答（槽位配置 M3-1）
    // D9 @模块：把命中的模块概要拼成聚焦块前置给 LLM（原文落库，聚焦块只在本次调用的消息头）
    let snap_json: &serde_json::Value = &snap.json;
    let focus_block = {
        let mods = snap_json.get("modules").and_then(|m| m.as_array()).cloned().unwrap_or_default();
        let known: Vec<String> = body
            .module_refs
            .iter()
            .filter(|id| mods.iter().any(|m| m.get("id").and_then(|v| v.as_str()) == Some(id.as_str())))
            .cloned()
            .collect();
        if known.is_empty() {
            String::new()
        } else {
            let lines: Vec<String> = known
                .iter()
                .filter_map(|id| mods.iter().find(|m| m.get("id").and_then(|v| v.as_str()) == Some(id.as_str())))
                .map(|m| {
                    let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let resp = m.get("responsibility").and_then(|v| v.as_str()).unwrap_or("");
                    let score = m.get("health").and_then(|h| h.get("score")).and_then(|v| v.as_i64()).unwrap_or(-1);
                    format!("- {name}（{id}）：{resp}　健康分 {score}")
                })
                .collect();
            format!("【本轮聚焦模块】（用户显式 @ 引用，回答请优先围绕这些模块展开）\n{}\n\n", lines.join("\n"))
        }
    };
    let llm_message = format!("{focus_block}{}", body.message);
    let answer: easyvibe_ai_agent::QaAnswer = match *st.llm_mode {
        LlmMode::Stub => easyvibe_ai_agent::StubQaClient::new().ask(&snap.json, &llm_message, &pairs, &body.images).await?,
        LlmMode::Anthropic => {
            let cfg = resolve_llm(&st, &id, "chat").await;
            if cfg.api_key.is_empty() {
                return Err(AppError(ApiError::BadRequest("未配置 LLM API key（设置面板或 EASYVIBE_LLM_API_KEY）".into())));
            }
            let llm = easyvibe_ai_agent::AnthropicClient::new(&cfg.base_url, &cfg.api_key, &cfg.model);
            // S1-3：user_entry 插槽注入（§9 #4——仅用户入口对话；透明 agent 的空插槽装配永不注入）
            let skills = st.harness.read().await.user_entry_skills.join("\n\n---\n\n");
            let prefix = if skills.trim().is_empty() {
                String::new()
            } else {
                format!("\n\n## 对话技能（grill-me：需求有歧义时主动用选择题澄清）\n\n{skills}\n\n---\n")
            };
            easyvibe_ai_agent::LlmQaClient::new_with_prefix(llm, &prefix).ask(&snap.json, &llm_message, &pairs, &body.images).await?
        }
    };

    // 4) 回答落库 + token 用量累计（§10 #4）
    st.conversation_repo
        .append_message(&conv.id, "assistant", &answer.reply, easyvibe_ai_agent::estimate_tokens(&answer.reply))
        .await?;
    st.conversation_repo
        .add_tokens(&conv.id, answer.prompt_tokens as i64, answer.completion_tokens as i64)
        .await?;

    // 5) auto-compact（§10a：80% 触发，全自动不打断）——压缩失败降级不传播（审查 🔴：
    //    回答已落库，绝不能让第 5 步把本轮问答变成 500）
    let budget = resolve_adv_i64(&st, &id, "adv.contextBudget", DEFAULT_CONTEXT_BUDGET).await;
    let threshold = resolve_adv_i64(&st, &id, "adv.compactThreshold", DEFAULT_COMPACT_THRESHOLD).await;
    let compaction_trace = match maybe_compact(&st, &id, body.conv.as_deref(), budget, threshold, false).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("[chat] {} auto-compact 失败（不阻断对话）: {e}", id);
            None
        }
    };

    // 累计口径（前端"累计 tokens"与 GET 恢复一致；POST 的 usage 是单次调用增量）
    let usage = {
        let c = st.conversation_repo.get_or_create(&id).await?;
        serde_json::json!({ "promptTokens": c.prompt_tokens, "completionTokens": c.completion_tokens })
    };

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "reply": answer.reply,
            "refs": answer.refs,
            "clarify": answer.clarify,
            "compaction": compaction_trace,
            "usage": usage,
        }
    }))
    .into_response())
}

/// 手动压缩（§10a：对话界面"压缩上下文"按钮；自动阈值兜底之外的主动手段）
async fn compact_chat(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let _guard = st.chat_lock.lock().await;
    let budget = resolve_adv_i64(&st, &id, "adv.contextBudget", DEFAULT_CONTEXT_BUDGET).await;
    let trace = maybe_compact(&st, &id, q.get("conv").map(|v| v.as_str()), budget, 0, true).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "compacted": trace.is_some(), "trace": trace } })).into_response())
}

/// 新对话：清空消息与摘要（会话行保留，token 计数归零）
async fn reset_chat(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let _guard = st.chat_lock.lock().await;
    let conv = resolve_conv(&st, &id, q.get("conv").map(|v| v.as_str())).await?;
    st.conversation_repo.reset(&conv.id).await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
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

/// 视图列表（F1b 读侧）：.easyvibe/views/*.json 引用式视图，供前端"视图"页签消费
async fn list_views(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let dir = repo.root.join(".easyvibe/views");
    let mut items: Vec<serde_json::Value> = vec![];
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else { continue };
            items.push(serde_json::json!({
                "slug": path.file_stem().and_then(|s| s.to_str()).unwrap_or(""),
                "name": v["name"],
                "createdAt": v["created_at"],
                "nodes": v["nodes"].as_array().map(|a| a.len()).unwrap_or(0),
                "view": v,
            }));
        }
    }
    items.sort_by(|a, b| b["createdAt"].as_str().cmp(&a["createdAt"].as_str()));
    Ok(Json(serde_json::json!({ "success": true, "data": items })).into_response())
}

/// 删除视图（F1b 读侧闭环）：slug 复用保存时的安全字符集，防线同 save_view
async fn delete_view(State(st): State<AppState>, Path((id, slug)): Path<(String, String)>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let safe: String = slug
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || ('\u{4e00}'..='\u{9fff}').contains(&c) { c } else { '-' })
        .collect();
    if safe.is_empty() || safe != slug {
        return Err(AppError(ApiError::BadRequest("非法视图标识".into())));
    }
    let path = repo.root.join(".easyvibe/views").join(format!("{safe}.json"));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| ApiError::Internal(format!("删除视图失败: {e}")))?;
    }
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

/// 存为视图（F1b 首次消费）：按格式规范 §9 写 .easyvibe/views/<slug>.json（引用式，不存布局）
async fn save_view(State(st): State<AppState>, Path(id): Path<String>, Json(body): Json<SaveViewRequest>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
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
    // 试用深挖#D：同名视图静默覆盖旧图（数据丢失）——冲突时追加短后缀
    let view_path = repo.root.join(".easyvibe/views").join(format!("{slug}.json"));
    let slug = if view_path.exists() {
        let suffix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() % 100000).unwrap_or(0);
        format!("{slug}-{suffix}")
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
                BusEvent::Freshness { repo, status, latest_commit_at, commits_since_map } => WsMessage {
                    name: "freshness.changed".into(),
                    data: serde_json::json!({ "repo": repo, "status": status, "latestCommitAt": latest_commit_at, "commitsSinceMap": commits_since_map }),
                },
                BusEvent::SessionOutput { session_id, line } => WsMessage {
                    name: "session.output".into(),
                    data: serde_json::json!({ "sessionId": session_id, "line": line }),
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
mod freshness;
mod git;

/// D5：单仓库 watcher 管线——自动归纳（无合法地图时）+ map/growth/progress 三 watcher。
/// 启动挂载与 POST /api/repos 动态注册共用（新仓库热生效，无需重启壳/后端）。
async fn spawn_repo_pipeline(
    r: easyvibe_map::Repo,
    map_service: Arc<MapService>,
    event_bus: broadcast::Sender<BusEvent>,
    session_manager: Arc<easyvibe_session::SessionManager>,
    prompt_template: String,
    agent_command: String,
    agent_args: Vec<String>,
) {
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
    let bus = event_bus;
    let repo_id = r.id.clone();
    tokio::spawn(async move {
        while prx.changed().await.is_ok() {
            let progress = prx.borrow().clone();
            if progress.is_null() { continue; }
            let _ = bus.send(BusEvent::Progress { repo: repo_id.clone(), progress });
        }
    });
}

#[tokio::main]
async fn main() {
    // Y3 清债：日志落盘（排障不再只靠终端）——数据目录 logs/ 按天滚动，双写 stderr
    let _log_guard = {
        let dir = std::env::var("EASYVIBE_DATA_DIR").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            format!("{home}/.easyvibe")
        });
        let dir = format!("{dir}/logs");
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| eprintln!("日志目录创建失败 {dir}: {e}"));
        let appender = tracing_appender::rolling::daily(&dir, "easyvibe.log");
        let (nb, guard) = tracing_appender::non_blocking(appender);
        // 双写：文件（按天滚动，排障事实源）+ stderr（D5-2 桌面壳 pipe_child_logs
        // 转发 sidecar 日志用——壳日志与后端日志汇流到一处，只看一个流）
        use tracing_subscriber::prelude::*;
        let file_layer = tracing_subscriber::fmt::layer().with_writer(nb).with_ansi(false);
        let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr).with_ansi(false);
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new("info"))
            .with(file_layer)
            .with(stderr_layer)
            .init();
        guard
    };

    // 仓库注册：EASYVIBE_REPO 环境变量（开发期手段）+ ~/.easyvibe/desktop-repos 持久化文件
    //（D5：文件归后端独占——应用内添加/注销都改写它，桌面壳不再代读）。
    // 合并去重、跳过不存在目录；两者皆空 = 零仓库起步（前端引导添加）。
    let mut repo_roots: Vec<std::path::PathBuf> = std::env::var("EASYVIBE_REPO")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(Into::into)
        .collect();
    for p in read_desktop_repos() {
        if !repo_roots.contains(&p) {
            repo_roots.push(p);
        }
    }
    let repo_roots: Vec<_> = repo_roots.into_iter().filter(|p| p.is_dir()).collect();
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
    // 改进#2：agent 输出 → 事件总线（过程直播）
    {
        let mut rx = session_manager.subscribe_output();
        let bus = event_bus.clone();
        tokio::spawn(async move {
            while let Ok(o) = rx.recv().await {
                let _ = bus.send(BusEvent::SessionOutput { session_id: o.session_id, line: o.line });
            }
        });
    }

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
    // D5：抽成 spawn_repo_pipeline——启动挂载与 POST /api/repos 动态注册共用同一条管线
    for r in repos {
        tokio::spawn(spawn_repo_pipeline(
            r,
            map_service.clone(),
            event_bus.clone(),
            session_manager.clone(),
            prompt_template.clone(),
            agent_command.clone(),
            agent_args.clone(),
        ));
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
    let conversation_repo = Arc::new(easyvibe_db::SqliteConversationRepository::new(database.pool().clone()));

    // Y4 清债：主密钥走 KeyProvider 抽象（当前=文件源；二期换系统钥匙串只换实现）
    let cipher = easyvibe_common::SecretCipher::from_provider(&easyvibe_common::FileKeyProvider::new(&data_dir))
        .expect("主密钥装载失败");

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
    let submap_prompt = std::fs::read_to_string(
        std::env::var("EASYVIBE_SUBMAP_PROMPT_PATH").unwrap_or_else(|_| "easyvibe-module-submap-prompt.md".into()),
    )
    .expect("子图分析提示词不可读（用 EASYVIBE_SUBMAP_PROMPT_PATH 指定）");

    // M3-3/S1-3：harness 插槽内核装载（manifest 驱动 + 出厂底账补齐）+ 任务执行引擎 + pending 恢复
    let harness = Arc::new(tokio::sync::RwLock::new(task_exec::load_harness().expect("harness 装载失败")));
    info!(
        "harness: {} v{}（user_entry 技能 {} 个）",
        harness.read().await.dir.display(),
        harness.read().await.manifest.version,
        harness.read().await.user_entry_skills.len()
    );
    let executor = task_exec::TaskExecutor::new(
        task_repo.clone(),
        approval_repo.clone(),
        session_manager.clone(),
        map_service.clone(),
        harness.clone(),
        Arc::new(agent_command.clone()),
        Arc::new(agent_args.clone()),
        std::env::var("EASYVIBE_MAX_PARALLEL").ok().and_then(|v| v.parse().ok()).unwrap_or(4),
        settings_repo.clone(),
    );
    // M3-5（§11 🟡4）：重启会杀掉 spawn 的 agent（kill_on_drop）——running 任务先标记
    // interrupted（awaiting_approval 等用户决策的任务不受影响）；pending 任务照常重新入队
    match task_repo.interrupt_running().await {
        Ok(n) if n > 0 => tracing::warn!("[startup] {} 个 running 任务标记 interrupted（后端重启）", n),
        Ok(_) => {}
        Err(e) => tracing::warn!("[startup] interrupted 标记失败: {e}"),
    }
    executor.enqueue_pending(None).await;

    // S2：定时落后度检查——地图保鲜状态变化推 freshness.changed（默认 30 分钟，adv.freshnessCheckMinutes 可调）。
    // 只检查不自动重归纳：git 漂移通知用户，是否花 agent 成本重归纳由用户决定（一键巡检/重归纳在头部）
    {
        let bus = event_bus.clone();
        let maps = map_service.clone();
        let settings = settings_repo.clone();
        tokio::spawn(async move {
            let mut last: std::collections::HashMap<String, String> = Default::default();
            loop {
                for repo in maps.repos().await {
                    let Ok(snap) = maps.load_map(&repo).await else { continue };
                    let f = freshness::assess(&repo.root, &snap.json);
                    let status = f.status.as_str().to_string();
                    let changed = last.get(&repo.id).map(|p| p != &status).unwrap_or(true);
                    if changed || status != "fresh" {
                        let _ = bus.send(BusEvent::Freshness {
                            repo: repo.id.clone(),
                            status: status.clone(),
                            latest_commit_at: f.latest_commit_at,
                            commits_since_map: f.commits_since_map,
                        });
                    }
                    last.insert(repo.id.clone(), status);
                }
                let mut mins: u64 = 30;
                if let Ok(Some(row)) = settings.get("global", "adv.freshnessCheckMinutes").await {
                    if let Ok(v) = serde_json::from_str::<i64>(&row.value) {
                        mins = (v.max(1)) as u64;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(mins * 60)).await;
            }
        });
    }

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
        conversation_repo,
        chat_lock: Arc::new(tokio::sync::Mutex::new(())),
        executor,
        harness: harness.clone(),
        llm_mode: Arc::new(llm_mode),
        patrol_prompt: Arc::new(patrol_prompt),
        submap_prompt: Arc::new(submap_prompt),
        schema_path: Arc::new(schema_path),
        event_bus,
    };
    let app = build_router(state.clone());
    // 定时巡检（默认关：adv.autoPatrolEnabled=true 开启，间隔 adv.autoPatrolHours 默认 24h）——
    // 健康保鲜不靠用户想起；成本可控（间隔可调/随时关），无活动会话才触发（写互斥天然排队）
    {
        let st_for_patrol = state.clone();
        let settings = st_for_patrol.settings_repo.clone();
        let maps = st_for_patrol.map_service.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                let enabled = settings.get("global", "adv.autoPatrolEnabled").await.ok().flatten()
                    .and_then(|r| serde_json::from_str::<bool>(&r.value).ok()).unwrap_or(false);
                if !enabled { continue }
                let hours: i64 = settings.get("global", "adv.autoPatrolHours").await.ok().flatten()
                    .and_then(|r| serde_json::from_str::<i64>(&r.value).ok()).unwrap_or(24).max(1);
                for repo in maps.repos().await {
                    use easyvibe_db::HealthRepository as _;
                    let fresh_enough = st_for_patrol.health_repo.list_runs(&repo.id, 1).await.ok()
                        .and_then(|runs| runs.first().and_then(|r| r.started_at.parse::<i64>().ok()))
                        .map(|t| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64 - t < hours * 3600)
                        .unwrap_or(false);
                    if fresh_enough { continue }
                    info!("[auto-patrol] {} 距上次巡检超 {}h，自动触发", repo.id, hours);
                    let st2 = st_for_patrol.clone();
                    let repo_id = repo.id.clone();
                    tokio::spawn(async move {
                        if let Err(e) = start_patrol(axum::extract::State(st2), axum::extract::Path(repo_id.clone())).await {
                            tracing::warn!("[auto-patrol] {} 触发失败: {:?}", repo_id, e.0);
                        }
                    });
                }
            }
        });
    }

    // D5：端口可由桌面壳覆盖（开发 7101 / 桌面壳 7151，避免双开冲突）
    let port: u16 = std::env::var("EASYVIBE_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(7101);
    let addr = format!("127.0.0.1:{port}");
    info!("EasyVibe backend listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap_or_else(|e| panic!("绑定 {addr} 失败: {e}"));
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    /// 样例地图（与 ai-agent 测试同构：validate_minimum 可通过，stub 问答可命中）
    const SAMPLE_MAP: &str = r#"{
      "version": "1.0",
      "meta": {"repo": "demo", "generated_at": "t", "generator": "g/test"},
      "layers": [{"id": "application", "name": "应用服务层", "order": 0, "description": "d"}],
      "modules": [{
        "id": "exam-core", "name": "考试与评测核心", "layer": "application",
        "responsibility": "考试会话编排、答题流程与评测提交管线",
        "files": ["lib/**"], "key_entries": [], "dependencies": [],
        "health": {"score": 64, "coupling": "high", "complexity": "high", "churn": "medium",
                   "decay_flags": [], "review_note": "n", "concerns": []}
      }],
      "edges": [],
      "health": {"score": 58, "coupling": "high", "complexity": "high", "churn": "high",
                 "decay_flags": [], "review_note": "r", "concerns": []}
    }"#;

    async fn test_state() -> AppState {
        test_state_with(MapService::new(vec![])).await
    }

    async fn test_state_with(svc: std::sync::Arc<MapService>) -> AppState {
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
        let conversation_repo = Arc::new(easyvibe_db::SqliteConversationRepository::new(db.pool().clone()));
        let harness = Arc::new(tokio::sync::RwLock::new(task_exec::Harness {
            dir: std::env::temp_dir(),
            manifest: task_exec::HarnessManifest {
                id: "stub".into(), version: "0".into(), builtin: false,
                route_rules: vec![], skills: task_exec::HarnessSkills::default(), transparent_neutralize: vec![],
            },
            framework_transparent: "框架".into(),
            user_entry_skills: vec![],
        }));
        let executor = task_exec::TaskExecutor::new(
            task_repo.clone(),
            approval_repo.clone(),
            session_manager.clone(),
            svc.clone(),
            harness.clone(),
            Arc::new("true".into()),
            Arc::new(vec![]),
            4,
            settings_repo.clone(),
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
            conversation_repo,
            chat_lock: Arc::new(tokio::sync::Mutex::new(())),
            executor,
            harness,
            llm_mode: Arc::new(LlmMode::Stub),
            patrol_prompt: Arc::new("test".into()),
            submap_prompt: Arc::new("test".into()),
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
    async fn map_etag_conditional_304() {
        // Y1 清债回归：ETag 命中即 304（重连重同步不再全量传输）
        let (state, repo) = chat_state("etag").await;
        let app = build_router(state);
        let get = |if_none: Option<&str>| {
            let mut req = axum::http::Request::get(format!("/api/repos/{repo}/map"))
                .body(axum::body::Body::empty()).unwrap();
            if let Some(v) = if_none {
                req.headers_mut().insert("if-none-match", v.parse().unwrap());
            }
            let app = app.clone();
            async move {
                let resp = app.oneshot(req).await.unwrap();
                (resp.status(), resp.headers().get("etag").and_then(|h| h.to_str().ok()).unwrap_or("").to_string())
            }
        };
        let (status, etag) = get(None).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(etag.starts_with('"') && etag.ends_with('"'), "ETag 应为引号包裹的哈希: {etag}");
        let (status2, _) = get(Some(&etag)).await;
        assert_eq!(status2, axum::http::StatusCode::NOT_MODIFIED, "If-None-Match 命中应 304");
    }

    #[tokio::test]
    async fn unknown_repo_404() {
        let app = build_router(test_state().await);
        let resp = app.oneshot(axum::http::Request::get("/api/repos/nope/map").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }

    /// 带样例地图的测试仓库（chat E2E 用）
    async fn chat_state(tag: &str) -> (AppState, String) {
        let dir = std::env::temp_dir().join(format!("ev-chat-test-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".easyvibe/map")).unwrap();
        std::fs::write(dir.join(".easyvibe/map/map.json"), SAMPLE_MAP).unwrap();
        let repo = repo_from_root(&dir);
        let repo_id = repo.id.clone();
        (test_state_with(MapService::new(vec![repo])).await, repo_id)
    }

    async fn post_chat(app: &axum::Router, repo: &str, message: &str) -> serde_json::Value {
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::post(format!("/api/repos/{repo}/chat"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::json!({ "message": message }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "chat POST 应成功");
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()
    }

    #[tokio::test]
    async fn progress_done_ago_gates_on_phase() {
        let dir = std::env::temp_dir().join("ev-progress-done-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".easyvibe/map")).unwrap();
        // 无文件 → None
        assert!(progress_done_ago_secs(&dir).is_none());
        // phase != done → None（归纳中不得误判）
        std::fs::write(dir.join(".easyvibe/map/progress.json"), r#"{"phase":"inducting"}"#).unwrap();
        assert!(progress_done_ago_secs(&dir).is_none());
        // phase=done → Some（文件刚写，ago 很小）
        std::fs::write(dir.join(".easyvibe/map/progress.json"), r#"{"phase":"done"}"#).unwrap();
        let ago = progress_done_ago_secs(&dir).expect("done 应有秒数");
        assert!(ago < 5, "刚落盘的文件 ago 应极小: {ago}");
    }

    #[tokio::test]
    async fn chat_persists_and_restores_from_db() {
        let (state, repo) = chat_state("persist").await;
        let app = build_router(state);
        let r = post_chat(&app, &repo, "谁负责评测提交？").await;
        let reply = r["data"]["reply"].as_str().unwrap_or_default();
        assert!(reply.contains("考试与评测核心"), "stub 应答应基于地图, reply={reply}");
        assert!(r["data"]["refs"].as_array().unwrap().iter().any(|x| x == "exam-core"));

        // 第二轮后再恢复：完整历史从库读（M3-5：不再只活在前端 state）
        post_chat(&app, &repo, "健康度多少分？").await;
        let resp = app
            .clone()
            .oneshot(axum::http::Request::get(format!("/api/repos/{repo}/chat")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let data = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"].clone();
        let messages = data["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4, "两轮对话 = user+assistant × 2");
        assert_eq!(messages[0]["role"], "user");
        assert!(messages[0]["content"].as_str().unwrap().contains("评测提交"));
        // token 用量已记账（stub 估算为正；POST 返回累计口径）
        assert!(data["usage"]["promptTokens"].as_i64().unwrap() > 0, "记账链路应产生正用量");
    }

    #[tokio::test]
    async fn chat_module_refs_injected_but_original_persisted() {
        let (state, repo) = chat_state("mrefs").await;
        let app = build_router(state);
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::post(format!("/api/repos/{repo}/chat"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::json!({ "message": "它健康吗？", "module_refs": ["exam-core", "ghost-module"] }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "带 module_refs 的 chat 应成功");
        // 持久化的是用户原文：聚焦块与未知 id 都不得进库（D9：聚焦只活在本轮 LLM 调用）
        let resp = app
            .clone()
            .oneshot(axum::http::Request::get(format!("/api/repos/{repo}/chat")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let messages = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"]["messages"]
            .as_array()
            .unwrap()
            .clone();
        let last_user = messages.iter().rev().find(|m| m["role"] == "user").unwrap();
        let content = last_user["content"].as_str().unwrap();
        assert_eq!(content, "它健康吗？");
        assert!(!content.contains("聚焦模块"), "聚焦块不得落库");
        assert!(!content.contains("ghost-module"), "未知模块 id 不得落库");
    }

    #[tokio::test]
    async fn conversations_multi_create_chat_scope() {
        let (state, repo) = chat_state("multiconv").await;
        let app = build_router(state);
        let get = |path: String| {
            let app = app.clone();
            async move {
                app.oneshot(axum::http::Request::get(path).body(axum::body::Body::empty()).unwrap()).await.unwrap()
            }
        };
        // 初始：每仓库一个默认会话
        let resp = get(format!("/api/repos/{repo}/conversations")).await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let list = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        assert_eq!(list["data"].as_array().unwrap().len(), 0, "会话懒创建：初始应为 0");

        // 新建第二个会话
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::post(format!("/api/repos/{repo}/conversations"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"title":"重构专项"}"#.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let created = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        let cid = created["data"]["id"].as_str().unwrap().to_string();
        assert_eq!(created["data"]["title"], "重构专项");
        assert_eq!(created["data"]["runtime"]["state"], "idle");

        // 向新会话发消息：不影响默认会话
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::post(format!("/api/repos/{repo}/chat"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::json!({ "message": "只在新会话里", "conv": cid }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let resp = get(format!("/api/repos/{repo}/chat?conv={cid}")).await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let d = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        assert_eq!(d["data"]["messages"].as_array().unwrap().len(), 2, "新会话应有问答两条");
        assert_eq!(d["data"]["conversation"]["title"], "重构专项");

        // 缺省会话 = 最近活跃（M4-2 语义）：不带 conv 的 /chat 应回到刚聊过的会话
        let resp = get(format!("/api/repos/{repo}/chat")).await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let d = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        assert_eq!(d["data"]["conversation"]["id"], cid, "缺省应回落到最近活跃的会话");
        assert_eq!(d["data"]["messages"].as_array().unwrap().len(), 2);

        // 重命名生效
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::put(format!("/api/repos/{repo}/conversations/{cid}"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"title":"改名了"}"#.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let resp = get(format!("/api/repos/{repo}/conversations")).await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let list = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        // 缺省回落不新建会话：列表仍只有手工创建的那一个（懒创建纪律）
        assert_eq!(list["data"].as_array().unwrap().len(), 1);
        assert!(list["data"].as_array().unwrap().iter().any(|c| c["title"] == "改名了"));
    }

    #[tokio::test]
    async fn y6_blocks_cross_site_but_allows_tauri_origin() {
        let (state, repo) = chat_state("y6").await;
        let app = build_router(state);
        // evil 页面：cross-site 且无白名单 Origin → 403
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::get("/api/repos")
                    .header("sec-fetch-site", "cross-site")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::FORBIDDEN, "跨站请求应被 Y6 拦截");
        // 桌面壳：Origin 白名单 → 放行（evil 页面无法伪造 Origin）
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::get("/api/repos")
                    .header("sec-fetch-site", "cross-site")
                    .header("origin", TAURI_ORIGIN)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "桌面壳 Origin 应豁免 Y6");
        let _ = repo;
    }

    #[tokio::test]
    async fn chat_manual_compact_leaves_trace_and_keeps_window() {
        let (state, repo) = chat_state("compact").await;
        // 让会话超过近期窗口（8 轮 = 16 条 > KEEP_RECENT 6）
        let app = build_router(state);
        for i in 0..8 {
            post_chat(&app, &repo, &format!("第 {i} 个问题：模块职责是什么？")).await;
        }
        // 手动压缩（force，绕过阈值）
        let resp = app
            .clone()
            .oneshot(axum::http::Request::post(format!("/api/repos/{repo}/chat/compact")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let data = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"].clone();
        assert_eq!(data["compacted"], true);
        let trace = data["trace"].as_str().unwrap().to_string();
        assert!(trace.contains("上下文已压缩"), "留痕消息: {trace}");

        // 恢复：系统留痕消息在列；水位前消息标 compacted（原文保留可回放）；近期窗口未压缩
        let resp = app
            .clone()
            .oneshot(axum::http::Request::get(format!("/api/repos/{repo}/chat")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let data = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"].clone();
        let messages = data["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 17, "16 条对话 + 1 条系统留痕");
        assert_eq!(messages[16]["role"], "system");
        assert!(messages[16]["content"].as_str().unwrap().contains("上下文已压缩"));
        assert!(messages.iter().take(10).all(|m| m["compacted"] == true), "水位前 10 条已折叠进摘要");
        assert!(messages[10..16].iter().all(|m| m["compacted"] == false), "近期窗口原文保留");
        // 摘要非空且 stub 诚实标注
        let summary = data["summary"].as_str().unwrap().to_string();
        assert!(summary.contains("stub"), "stub 模式压缩应诚实标注: {summary}");
    }

    #[tokio::test]
    async fn views_roundtrip_create_list_delete() {
        let (state, repo) = chat_state("views-crud").await;
        let dir = std::env::temp_dir().join("ev-chat-test-views-crud/.easyvibe/views");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("我的视图.json"),
            r#"{"version":"1.0","name":"我的视图","created_at":"2026-09-30","nodes":[{"ref":"module:m1"}],"edges":[],"annotations":[]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("second.json"),
            r#"{"version":"1.0","name":"second","created_at":"2026-09-29","nodes":[],"edges":[],"annotations":[]}"#,
        )
        .unwrap();
        let app = build_router(state);
        let get = |app: axum::Router, path: &str| {
            let path = path.to_string();
            async move {
                let resp = app.oneshot(axum::http::Request::get(path).body(axum::body::Body::empty()).unwrap()).await.unwrap();
                let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
                serde_json::from_slice::<serde_json::Value>(&body).unwrap()
            }
        };
        // 列表：按 createdAt 倒序，中文 slug 可读
        let d = get(app.clone(), &format!("/api/repos/{repo}/views")).await;
        let items = d["data"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["slug"], "我的视图", "倒序：新者在前");
        assert_eq!(items[0]["nodes"], 1);
        // 删除：确认语义——删后列表减一
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::delete(format!("/api/repos/{repo}/views/{}", urlencoding_encode("second")))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let d = get(app.clone(), &format!("/api/repos/{repo}/views")).await;
        assert_eq!(d["data"].as_array().unwrap().len(), 1);
        // 非法 slug（路径遍历企图）→ 400
        let resp = app
            .oneshot(
                axum::http::Request::delete(format!("/api/repos/{repo}/views/..%2F..%2Fetc"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    fn urlencoding_encode(s: &str) -> String {
        s.bytes().map(|b| format!("%{:02X}", b)).collect()
    }

    #[tokio::test]
    async fn task_diff_endpoint_reads_archive_on_demand() {
        let (state, repo) = chat_state("diff-endpoint").await;
        // 手工造归档（正常路径由终态采集写入）：diff 全文只进归档，tasks.result 不带
        let dir = std::env::temp_dir().join("ev-chat-test-diff-endpoint/.easyvibe/development_docs");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("task-x.json"),
            r#"{"taskId":"task-x","diffFull":"+added line","diffStat":" a.txt | 1 +","archivedPath":null}"#,
        )
        .unwrap();
        let app = build_router(state);
        let get = |path: String| {
            let app = app.clone();
            async move {
                let resp = app.oneshot(axum::http::Request::get(path).body(axum::body::Body::empty()).unwrap()).await.unwrap();
                let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
                serde_json::from_slice::<serde_json::Value>(&body).unwrap()
            }
        };
        let d = get(format!("/api/repos/{repo}/tasks/task-x/diff")).await;
        assert_eq!(d["data"]["diff"], "+added line");
        assert_eq!(d["data"]["diffStat"], " a.txt | 1 +");
        // 无归档 → diff null（不 404：调用方区分"无变更"）
        let d = get(format!("/api/repos/{repo}/tasks/task-nope/diff")).await;
        assert!(d["data"]["diff"].is_null());
    }

    #[tokio::test]
    async fn chat_reset_starts_fresh_conversation() {
        let (state, repo) = chat_state("reset").await;
        let app = build_router(state);
        post_chat(&app, &repo, "一个问题").await;
        let resp = app
            .clone()
            .oneshot(axum::http::Request::post(format!("/api/repos/{repo}/chat/reset")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let resp = app
            .oneshot(axum::http::Request::get(format!("/api/repos/{repo}/chat")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let data = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"].clone();
        assert!(data["messages"].as_array().unwrap().is_empty());
        assert!(data["summary"].is_null());
    }
}
