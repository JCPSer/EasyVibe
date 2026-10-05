//! 启动装配：日志/仓库注册/DB/会话桥/引擎装载/监听（自 main.rs 保序搬迁）。

use crate::assets::{self, embedded_ui_available, resolve_text_asset};
use crate::router::build_router;
use crate::routes::map::start_patrol;
use crate::state::{data_dir, read_desktop_repos, resolve_agent_command, session_kind_from_label, AppState, LlmMode};
use crate::pipeline::spawn_repo_pipeline;
use easyvibe_db::{SettingsRepository as _, TaskRepository as _};
use crate::{freshness, task_exec};
use easyvibe_api_types::SessionStatusChanged;
use easyvibe_event_bus::queue::QueueState;
use easyvibe_event_bus::{publish, BusEvent};
use easyvibe_map::{repo_from_root, MapService};
use easyvibe_session::SessionManager;
use easyvibe_ai_agent::agent_conf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::info;

pub(crate) async fn run() {
    // Y3 清债：日志落盘（排障不再只靠终端）——数据目录 logs/ 按天滚动，双写 stderr
    let _log_guard = {
        let dir = data_dir().to_string_lossy().into_owned();
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

    // session 管理：会话事件翻译进总线（channel → manager → 桥，顺序不可乱）
    let (session_tx, mut session_rx) = tokio::sync::mpsc::channel::<SessionStatusChanged>(64);
    let session_manager = SessionManager::new(session_tx);
    // agent CLI 配置：命令/参数/提示词模板均可环境变量覆盖（测试可用 stub 命令）。
    // 默认 -p --bare --dangerously-skip-permissions --output-format stream-json：
    // bare 跳过宿主 hooks（防 grill-me 类钩子把无人值守任务带偏成访谈模式）；
    // skip-permissions 授予 Bash 等工具（实弹验证发现 headless 下 Bash 默认被拒，
    // agent 只能"分析后成功退出"什么都不写）；
    // stream-json --verbose = 流式输出（stream-json 必须配 verbose，否则 CLI 直接报错退出）——
    // 否则 claude -p 完成前零输出，"实时终端"永远空白
    // （2026-10-03 实弹：用户看着 LIVE 终端等全程，日志显示 stdout 全在退出瞬间到达）
    let agent_command = std::env::var("EASYVIBE_AGENT_CMD").unwrap_or_else(|_| "claude".into());
    // GUI 启动的进程 PATH 极薄（launchd 只有 /usr/bin:/bin:...），裸命令名 spawn 必败——
    // 实测桌面壳三个任务全部"spawn claude 失败: No such file or directory"。
    // 启动时把命令解析成绝对路径：PATH 直查 → 登录 shell PATH → 常见安装位兜底。
    let agent_command = resolve_agent_command(&agent_command);
    info!("[boot] agent CLI 解析为: {agent_command}");
    let agent_args: Vec<String> = std::env::var("EASYVIBE_AGENT_ARGS")
        .unwrap_or_else(|_| "-p --bare --dangerously-skip-permissions --output-format stream-json --verbose".into())
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    let (prompt_template, prompt_path) = resolve_text_asset(
        "EASYVIBE_PROMPT_PATH",
        "easyvibe-map-prompt-v2.2.md",
        assets::MAP_PROMPT,
        true,
    );
    info!("agent={} args={:?} prompt={}", agent_command, agent_args, prompt_path);

    // M3-3/S1-3：harness 装载上移到管线挂载之前——spawn_repo_pipeline 注入点 #8 需要持有它
    // （生产装载 = 迁移（哨兵短路）→ 密封自检重铺 → 纯装载）
    let harness = Arc::new(tokio::sync::RwLock::new(task_exec::load_harness().expect("harness 装载失败")));

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
            harness.clone(),
        ));
    }

    // M2-4：域 2 SQLite + 巡检槽位
    let data_dir_path = data_dir();
    let data_dir = data_dir_path.to_string_lossy().into_owned();
    std::fs::create_dir_all(&data_dir).unwrap_or_else(|e| panic!("数据目录不可写 {data_dir}: {e}"));
    let database = easyvibe_db::Database::connect_file(&format!("{data_dir}/easyvibe.db"))
        .await
        .expect("SQLite 初始化失败");
    info!("域 2 状态库: {data_dir}/easyvibe.db");
    let health_repo = Arc::new(easyvibe_db::SqliteHealthRepository::new(database.pool().clone()));
    let agent_session_repo = Arc::new(easyvibe_db::AgentSessionRepo::new(database.pool().clone()));
    let session_output_repo = Arc::new(easyvibe_db::SessionOutputRepo::new(database.pool().clone()));
    // session 管理：会话事件翻译进总线 + 终态落库（channel/manager 在前，桥在此，顺序不可乱）
    {
        let bus = event_bus.clone();
        let mgr_for_final = session_manager.clone();
        let repo_for_final = agent_session_repo.clone();
        let session_output_repo_final = session_output_repo.clone();
        tokio::spawn(async move {
            while let Some(s) = session_rx.recv().await {
                // M1/U1 终态收尾：label/kind/terminal_at 落库（INSERT OR IGNORE 兜底 Stub 巡检等无 spawn 会话）
                if matches!(s.status, easyvibe_api_types::SessionStatus::Succeeded | easyvibe_api_types::SessionStatus::Failed) {
                    let label = mgr_for_final.label_of(&s.session_id).await.unwrap_or_else(|| s.session_id.clone());
                    let kind = session_kind_from_label(&label);
                    let now = chrono::Utc::now().to_rfc3339();
                    let repo = repo_for_final.clone();
                    let sid = s.session_id.clone();
                    let repo_id = s.repo.clone();
                    let status = format!("{:?}", s.status).to_lowercase();
                    let out_repo = session_output_repo_final.clone();
                    tokio::spawn(async move {
                        if let Err(err) = repo.finalize(&sid, &repo_id, &status, &now, None, Some(&label), &kind).await {
                            tracing::warn!("[agent_sessions] finalize 失败 {sid}: {err}");
                        }
                        // M2 容量策略：终态后每会话只留最近 50k 行
                        if let Err(err) = out_repo.prune_session(&sid, 50_000).await {
                            tracing::warn!("[session_outputs] 修剪失败 {sid}: {err}");
                        }
                    });
                }
                publish(&bus, BusEvent::SessionStatus(s));
            }
        });
    }
    // 改进#2：agent 输出 → 事件总线（过程直播）
    {
        let mut rx = session_manager.subscribe_output();
        let bus = event_bus.clone();
        tokio::spawn(async move {
            while let Ok(o) = rx.recv().await {
                let stream = match o.stream {
                    easyvibe_session::OutputStream::Stdout => "stdout",
                    easyvibe_session::OutputStream::Stderr => "stderr",
                };
                publish(&bus, BusEvent::SessionOutput { session_id: o.session_id, seq: o.seq, stream: stream.into(), line: o.line });
            }
        });
    }
    // M2：会话输出行 → session_outputs 落盘（独立接收器 + 300ms 攒批——
    // 不占用 WS 桥路径；INSERT OR IGNORE 幂等，重放/补拉不重复）
    {
        let mut rx = session_manager.subscribe_output();
        let repo = session_output_repo.clone();
        tokio::spawn(async move {
            let mut pending: Vec<easyvibe_db::SessionOutputRow> = Vec::new();
            let mut ticker = tokio::time::interval(std::time::Duration::from_millis(300));
            ticker.tick().await; // 立即 tick 消耗
            loop {
                tokio::select! {
                    recv = rx.recv() => {
                        match recv {
                            Ok(o) => {
                                let stream = match o.stream {
                                    easyvibe_session::OutputStream::Stdout => "stdout",
                                    easyvibe_session::OutputStream::Stderr => "stderr",
                                };
                                pending.push(easyvibe_db::SessionOutputRow {
                                    session_id: o.session_id,
                                    seq: o.seq as i64,
                                    ts: chrono::Utc::now().to_rfc3339(),
                                    stream: stream.into(),
                                    line: o.line,
                                });
                                if pending.len() >= 200 {
                                    let batch = std::mem::take(&mut pending);
                                    let repo = repo.clone();
                                    tokio::spawn(async move {
                                        if let Err(e) = repo.append_batch(&batch).await {
                                            tracing::warn!("[session_outputs] 落盘失败: {e}");
                                        }
                                    });
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!("[session_outputs] 广播滞后，丢 {n} 行（回放端点兜底）");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    _ = ticker.tick() => {
                        if !pending.is_empty() {
                            let batch = std::mem::take(&mut pending);
                            let repo = repo.clone();
                            tokio::spawn(async move {
                                if let Err(e) = repo.append_batch(&batch).await {
                                    tracing::warn!("[session_outputs] 落盘失败: {e}");
                                }
                            });
                        }
                    }
                }
            }
        });
    }

    // M1/U1：会话元事件 → agent_sessions 落盘（model/usage 边读边写；DB 故障只告警不阻流）
    {
        use easyvibe_session::SessionMetaUpdate as Meta;
        let mut rx = session_manager.subscribe_meta();
        let repo = agent_session_repo.clone();
        tokio::spawn(async move {
            while let Ok(e) = rx.recv().await {
                let res = match &e.update {
                    Meta::Cli(cli) => {
                        let cmd = cli.rsplit(['/', '\\']).next().unwrap_or(cli).to_string();
                        repo.upsert_started(&e.session_id, &e.repo, &cmd, &chrono::Utc::now().to_rfc3339()).await
                    }
                    Meta::Label(label) => {
                        let kind = session_kind_from_label(label);
                        repo.set_label_kind(&e.session_id, label, kind).await
                    }
                    Meta::Model(m) => repo.set_model(&e.session_id, m).await,
                    Meta::Usage { input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cost_usd, duration_ms, turns } => {
                        repo.set_usage(&e.session_id, *input_tokens, *output_tokens, *cache_read_tokens, *cache_write_tokens, *cost_usd, *duration_ms, *turns).await
                    }
                    Meta::ExitCode(_) => Ok(()), // 退出码随终态事件 finalize 统一写
                };
                if let Err(err) = res {
                    tracing::warn!("[agent_sessions] 元事件落盘失败 {}: {err}", e.session_id);
                }
            }
        });
    }
    let patrol_service = Arc::new(easyvibe_ai_agent::PatrolService::new(health_repo.clone()));
    let settings_repo = Arc::new(easyvibe_db::SqliteSettingsRepository::new(database.pool().clone()));
    let task_repo = Arc::new(easyvibe_db::SqliteTaskRepository::new(database.pool().clone()));
    let approval_repo = Arc::new(easyvibe_db::SqliteApprovalRepository::new(database.pool().clone()));
    let conversation_repo = Arc::new(easyvibe_db::SqliteConversationRepository::new(database.pool().clone()));

    // R3 D1：总线持久化——关键服务端事件落 events 表（越界率/复检消费率的秤），
    // 集中一处订阅，业务代码零侵入（前端交互事件走 POST /events 另一条路）
    {
        use easyvibe_db::EventRepository as _;
        let mut rx = event_bus.subscribe();
        let events = Arc::new(easyvibe_db::SqliteEventRepository::new(database.pool().clone()));
        tokio::spawn(async move {
            while let Ok(e) = rx.recv().await {
                let (repo, name, payload) = match e {
                    BusEvent::TaskContractAlert { repo, task_id, files } => {
                        (repo, "task.contractAlert", serde_json::json!({ "taskId": task_id, "files": files.len() }))
                    }
                    BusEvent::TaskContractViolated { repo, task_id, files } => {
                        (repo, "task.contractViolated", serde_json::json!({ "taskId": task_id, "files": files.len() }))
                    }
                    BusEvent::PatrolFinished { repo, run_id, status } => {
                        (repo, "patrol.finished", serde_json::json!({ "runId": run_id, "status": status }))
                    }
                    _ => continue,
                };
                let _ = events.record(&repo, name, &payload.to_string()).await;
            }
        });
    }


    // Y4 清债：主密钥走 KeyProvider 抽象（当前=文件源；二期换系统钥匙串只换实现）
    let cipher = easyvibe_common::SecretCipher::from_provider(&easyvibe_common::FileKeyProvider::new(&data_dir))
        .expect("主密钥装载失败");

    let llm_mode = if std::env::var("EASYVIBE_LLM_MODE").map(|v| v == "stub").unwrap_or(false) {
        LlmMode::Stub
    } else {
        LlmMode::Anthropic
    };
    // 提示词/schema 统一走解析链（env → exe 旁 → cwd → 编译期内嵌落盘），不再因缺文件直接 panic——
    // 独立分发形态（双击 exe）此前死在这里：窗口一闪而过，用户看到的就是"没反应"
    let (patrol_prompt, _) = resolve_text_asset(
        "EASYVIBE_PATROL_PROMPT_PATH",
        "easyvibe-map-patrol-prompt-v2.md",
        assets::PATROL_PROMPT,
        true,
    );
    let (_, schema_path) = resolve_text_asset("EASYVIBE_SCHEMA_PATH", "easyvibe-map-schema-v1.1.json", assets::MAP_SCHEMA, true);
    let (submap_prompt, _) = resolve_text_asset(
        "EASYVIBE_SUBMAP_PROMPT_PATH",
        "easyvibe-module-submap-prompt.md",
        assets::SUBMAP_PROMPT,
        true,
    );

    // M3-3/S1-3：harness 已在管线挂载前装载（见上），此处仅打日志 + 任务执行引擎 + pending 恢复
    info!(
        "harness: {} v{}（user_entry 技能 {} 个）",
        harness.read().await.dir.display(),
        harness.read().await.manifest.version,
        harness.read().await.user_entry_skills.len()
    );
    let executor = task_exec::TaskExecutor::new_with_sessions(
        task_repo.clone(),
        approval_repo.clone(),
        agent_session_repo.clone(),
        session_manager.clone(),
        map_service.clone(),
        harness.clone(),
        Arc::new(agent_command.clone()),
        Arc::new(agent_args.clone()),
        std::env::var("EASYVIBE_MAX_PARALLEL").ok().and_then(|v| v.parse().ok()).unwrap_or(4),
        settings_repo.clone(),
        Some(event_bus.clone()),
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
                        publish(&bus, BusEvent::Freshness {
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
        agent_session_repo,
        session_output_repo,
        settings_repo,
        cipher: Arc::new(cipher),
        task_repo,
        approval_repo,
        conversation_repo,
        event_repo: Arc::new(easyvibe_db::SqliteEventRepository::new(database.pool().clone())),
        chat_lock: Arc::new(tokio::sync::Mutex::new(())),
        executor,
        harness: harness.clone(),
        llm_mode: Arc::new(llm_mode),
        patrol_prompt: Arc::new(patrol_prompt),
        submap_prompt: Arc::new(submap_prompt),
        schema_path: Arc::new(schema_path),
        event_bus,
        pool: database.pool().clone(),
        agent_detected: Default::default(),
        agent_test: Default::default(),
        agent_test_lock: Default::default(),
        session_queue: Default::default(),
    };
    // 队列排空（需求 v1 §4.1/§8）：
    // ① 事件驱动——终态 SessionStatus 且「事件 session_id 经 status_of_session 确认为终态」
    //   （B1 收紧：grace 迟到的 Succeeded 被护栏丢弃后，以 by_id 终态归属为准，不误触发）；
    // ② 12s 清扫器兜底（B3）——覆盖 try_send 背压丢弃等漏事件路径
    {
        let st = state.clone();
        tokio::spawn(async move {
            let mut rx = st.event_bus.subscribe();
            while let Ok(e) = rx.recv().await {
                let BusEvent::SessionStatus(s) = e else { continue };
                if !matches!(s.status, easyvibe_api_types::SessionStatus::Succeeded | easyvibe_api_types::SessionStatus::Failed) {
                    continue;
                }
                if let Some(cur) = st.session_manager.status_of_session(&s.session_id).await {
                    if matches!(cur.status, easyvibe_api_types::SessionStatus::Starting | easyvibe_api_types::SessionStatus::Running) {
                        continue;
                    }
                    st.session_queue.drain(&st, &s.repo).await;
                }
            }
        });
        tokio::spawn(QueueState::sweep_loop(state.session_queue.clone(), state.clone()));
    }
    let app = build_router(state.clone());
    // M1 配置体系：启动即探测执行 agent（异步——探测失败只是 missing 态，不阻断启动；
    // 用户装完后可 POST /api/agent/detect 重探）
    {
        let st_detect = state.clone();
        tokio::spawn(async move {
            let detected = agent_conf::detect_agents().await;
            info!("[agent] 启动探测完成：检测到 {} 个", detected.len());
            *st_detect.agent_detected.write().await = detected;
        });
    }
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
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap_or_else(|e| panic!("绑定 {addr} 失败: {e}"));
    // 独立形态（无外部静态目录 + 内嵌 UI）：打印人类可读横幅并自动打开浏览器——
    // 双击 exe 的完整体验是"弹窗出界面"，而不是一个看不懂的黑窗口（2026-10-04 修复）
    let standalone = std::env::var("EASYVIBE_STATIC_DIR").ok().filter(|d| !d.is_empty()).is_none()
        && embedded_ui_available();
    if standalone {
        let url = format!("http://{addr}");
        eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        eprintln!("  EasyVibe 已启动 → {url}");
        eprintln!("  数据目录: {}", data_dir_path.display());
        eprintln!("  关闭本窗口即退出服务");
        eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        #[cfg(target_os = "windows")]
        std::process::Command::new("cmd").args(["/C", "start", &url]).spawn().ok();
        #[cfg(target_os = "macos")]
        std::process::Command::new("open").arg(&url).spawn().ok();
    }
    info!("EasyVibe backend listening on {addr}（standalone={standalone}）");
    axum::serve(listener, app).await.unwrap();
}
