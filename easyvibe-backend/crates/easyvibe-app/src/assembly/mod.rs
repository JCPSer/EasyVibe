//! 装配格（c-arch-10 R2）：启动装配 + 静态产物托管。
//!
//! 与 `server-api` 同为 `application` 层（order 3），属**同层单向**关系：
//! `assembly → server-api`（装配读取 `AppState` / `build_router` / `service::*`），**不得反向**
//! （`server-api` 内不得出现 `crate::assembly::`）。同层边不产生 direction_violation。
//!
//! 本格自 `bootstrap.rs`（启动装配，纯搬运）与 `router.rs` 静态回落段（外提）拆出——
//! 使 `server-api` 由「四职责合驻」收敛为「路由边界 + 业务编排」两件事。

mod bridges;
mod logging;
mod schedulers;
pub(crate) mod static_host;

use crate::router::build_router;
use crate::state::{data_dir, AppState, LlmMode};
use crate::task_exec;
use easyvibe_api_types::SessionStatusChanged;
use easyvibe_event_bus::BusEvent;
use easyvibe_map::MapService;
use easyvibe_session::SessionManager;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::info;

/// 启动装配主线（顺序不变量见方案 §四 R3）。
pub(crate) async fn run() {
    let _log_guard = logging::init();
    let repos = logging::load_repos();
    let map_service = MapService::new(repos.clone());
    logging::warm_map(&map_service, &repos).await;

    let (event_bus, _) = broadcast::channel::<BusEvent>(256);
    // session 管理：会话事件翻译进总线（channel → manager → 桥，顺序不可乱）
    let (session_tx, session_rx) = tokio::sync::mpsc::channel::<SessionStatusChanged>(64);
    let session_manager = SessionManager::new(session_tx);

    let cfg = logging::resolve_agent_cfg();
    // M3-3/S1-3：harness 装载上移到管线挂载之前——spawn_repo_pipeline 注入点 #8 需要持有它
    let harness = Arc::new(tokio::sync::RwLock::new(task_exec::load_harness().expect("harness 装载失败")));

    // 每个仓库一个地图 watcher，变更翻译为总线事件（启动挂载与 POST /api/repos 共用同一管线）
    for r in repos {
        crate::service::repo::spawn_pipeline(
            map_service.clone(),
            event_bus.clone(),
            session_manager.clone(),
            cfg.prompt_template.clone(),
            cfg.command.clone(),
            cfg.args.clone(),
            harness.clone(),
            r,
        )
        .await;
    }

    // M2-4：域 2 SQLite + 巡检槽位
    let data_dir_path = data_dir();
    let data_dir_str = data_dir_path.to_string_lossy().into_owned();
    std::fs::create_dir_all(&data_dir_str).unwrap_or_else(|e| panic!("数据目录不可写 {data_dir_str}: {e}"));
    let database = easyvibe_db::Database::connect_file(&format!("{data_dir_str}/easyvibe.db"))
        .await
        .expect("SQLite 初始化失败");
    info!("域 2 状态库: {data_dir_str}/easyvibe.db");
    let health_repo = Arc::new(easyvibe_db::SqliteHealthRepository::new(database.pool().clone()));
    let agent_session_repo = Arc::new(easyvibe_db::AgentSessionRepo::new(database.pool().clone()));
    let session_output_repo = Arc::new(easyvibe_db::SessionOutputRepo::new(database.pool().clone()));

    // 5 条落库/直播桥（channel/manager 在前，桥在此，顺序不可乱）
    bridges::spawn_session_terminal(session_rx, session_manager.clone(), agent_session_repo.clone(), session_output_repo.clone(), event_bus.clone());
    bridges::spawn_output_live(&session_manager, event_bus.clone());
    bridges::spawn_output_persist(&session_manager, session_output_repo.clone());
    bridges::spawn_meta_persist(&session_manager, agent_session_repo.clone());

    let patrol_service = Arc::new(easyvibe_ai_agent::PatrolService::new(health_repo.clone()));
    let settings_repo = Arc::new(easyvibe_db::SqliteSettingsRepository::new(database.pool().clone()));
    let task_repo = Arc::new(easyvibe_db::SqliteTaskRepository::new(database.pool().clone()));
    let approval_repo = Arc::new(easyvibe_db::SqliteApprovalRepository::new(database.pool().clone()));
    let conversation_repo = Arc::new(easyvibe_db::SqliteConversationRepository::new(database.pool().clone()));
    bridges::spawn_event_persist(&event_bus, database.pool());

    // Y4 清债：主密钥走 KeyProvider 抽象（当前=文件源；二期换系统钥匙串只换实现）
    let cipher = Arc::new(
        easyvibe_common::SecretCipher::from_provider(&easyvibe_common::FileKeyProvider::new(&data_dir_str))
            .expect("主密钥装载失败"),
    );
    let llm_mode = if std::env::var("EASYVIBE_LLM_MODE").map(|v| v == "stub").unwrap_or(false) {
        LlmMode::Stub
    } else {
        LlmMode::Anthropic
    };
    let assets = logging::resolve_assets();

    // M3-3/S1-3：harness 已在管线挂载前装载（见上），此处仅打日志
    info!(
        "harness: {} v{}（user_entry 技能 {} 个）",
        harness.read().await.dir.display(),
        harness.read().await.manifest.version,
        harness.read().await.user_entry_skills.len()
    );

    let state = build_state(BuildInputs {
        map_service,
        session_manager: session_manager.clone(),
        prompt_template: Arc::new(cfg.prompt_template),
        agent_command: Arc::new(cfg.command),
        agent_args: Arc::new(cfg.args),
        patrol_service,
        health_repo,
        agent_session_repo,
        session_output_repo,
        settings_repo,
        task_repo,
        approval_repo,
        conversation_repo,
        cipher,
        llm_mode,
        assets,
        event_bus,
        pool: database.pool().clone(),
        harness,
    })
    .await;

    schedulers::spawn_queue_drain(state.clone(), session_manager);
    let app = static_host::attach(build_router(state.clone()));
    schedulers::spawn_agent_detect(state.clone());
    schedulers::spawn_auto_patrol(state);

    // D5：端口可由桌面壳覆盖（开发 7101 / 桌面壳 7151，避免双开冲突）
    let port: u16 = std::env::var("EASYVIBE_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(7101);
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap_or_else(|e| panic!("绑定 {addr} 失败: {e}"));
    let standalone = static_host::standalone();
    static_host::announce(standalone, &addr, &data_dir_path);
    info!("EasyVibe backend listening on {addr}（standalone={standalone}）");
    axum::serve(listener, app).await.unwrap();
}

/// `build_state` 的输入（组合根对象图）。
struct BuildInputs {
    map_service: Arc<MapService>,
    session_manager: Arc<SessionManager>,
    prompt_template: Arc<String>,
    agent_command: Arc<String>,
    agent_args: Arc<Vec<String>>,
    patrol_service: Arc<easyvibe_ai_agent::PatrolService<easyvibe_db::SqliteHealthRepository>>,
    health_repo: Arc<easyvibe_db::SqliteHealthRepository>,
    agent_session_repo: Arc<easyvibe_db::AgentSessionRepo>,
    session_output_repo: Arc<easyvibe_db::SessionOutputRepo>,
    settings_repo: Arc<easyvibe_db::SqliteSettingsRepository>,
    task_repo: Arc<easyvibe_db::SqliteTaskRepository>,
    approval_repo: Arc<easyvibe_db::SqliteApprovalRepository>,
    conversation_repo: Arc<easyvibe_db::SqliteConversationRepository>,
    cipher: Arc<easyvibe_common::SecretCipher>,
    llm_mode: LlmMode,
    assets: logging::ResolvedAssets,
    event_bus: broadcast::Sender<BusEvent>,
    pool: easyvibe_db::sqlx::SqlitePool,
    harness: Arc<tokio::sync::RwLock<task_exec::Harness>>,
}

/// 构造 `AppState`（含执行引擎装配、重启收尸、pending 重入队、保鲜定时器）。
async fn build_state(i: BuildInputs) -> AppState {
    use easyvibe_db::TaskRepository as _;

    // c-arch-7 R3：task-engine 端口 ↔ 具体仓储的适配器落组合根（依赖倒置）
    let executor = task_exec::TaskExecutor::new_with_sessions(
        i.task_repo.clone(),
        i.approval_repo.clone(),
        i.agent_session_repo.clone(),
        i.session_manager.clone(),
        i.map_service.clone(),
        i.harness.clone(),
        std::env::var("EASYVIBE_MAX_PARALLEL").ok().and_then(|v| v.parse().ok()).unwrap_or(4),
        Arc::new(crate::db_ports::AgentSlotAdapter::new(
            i.settings_repo.clone(),
            Arc::new((*i.agent_command).clone()),
            Arc::new((*i.agent_args).clone()),
        )),
        Some(i.event_bus.clone()),
    );
    // M3-5（§11 🟡4）：重启会杀掉 spawn 的 agent（kill_on_drop）——running 任务先标记
    // interrupted（awaiting_approval 等用户决策的任务不受影响）；pending 任务照常重新入队
    match i.task_repo.interrupt_running().await {
        Ok(n) if n > 0 => tracing::warn!("[startup] {} 个 running 任务标记 interrupted（后端重启）", n),
        Ok(_) => {}
        Err(e) => tracing::warn!("[startup] interrupted 标记失败: {e}"),
    }
    // agent_sessions 同步收尸（与 tasks 清扫同策）：关应用/崩溃留下的 running 僵尸行——
    // 必须在 enqueue_pending 之前（spawn 会重新写 running）
    match i.agent_session_repo.interrupt_running(&chrono::Utc::now().to_rfc3339()).await {
        Ok(n) if n > 0 => tracing::warn!("[startup] {} 个 running 会话标记 failed（后端重启）", n),
        Ok(_) => {}
        Err(e) => tracing::warn!("[startup] agent_sessions 收尸失败: {e}"),
    }
    executor.enqueue_pending(None).await;

    // S2：定时落后度检查（默认 30 分钟，adv.freshnessCheckMinutes 可调）
    schedulers::spawn_freshness(i.map_service.clone(), i.settings_repo.clone(), i.event_bus.clone());

    AppState {
        map_service: i.map_service,
        session_manager: i.session_manager,
        prompt_template: i.prompt_template,
        agent_command: i.agent_command,
        agent_args: i.agent_args,
        patrol_service: i.patrol_service,
        health_repo: i.health_repo,
        agent_session_repo: i.agent_session_repo,
        session_output_repo: i.session_output_repo,
        settings_repo: i.settings_repo,
        cipher: i.cipher,
        task_repo: i.task_repo,
        approval_repo: i.approval_repo,
        conversation_repo: i.conversation_repo,
        event_repo: Arc::new(easyvibe_db::SqliteEventRepository::new(i.pool.clone())),
        chat_lock: Arc::new(tokio::sync::Mutex::new(())),
        executor,
        harness: i.harness,
        llm_mode: Arc::new(i.llm_mode),
        patrol_prompt: Arc::new(i.assets.patrol_prompt),
        submap_prompt: Arc::new(i.assets.submap_prompt),
        incremental_prompt: Arc::new(i.assets.incremental_prompt),
        schema_path: Arc::new(i.assets.schema_path),
        event_bus: i.event_bus,
        pool: i.pool,
        agent_detected: Default::default(),
        agent_test: Default::default(),
        agent_test_lock: Default::default(),
        session_queue: Default::default(),
    }
}
