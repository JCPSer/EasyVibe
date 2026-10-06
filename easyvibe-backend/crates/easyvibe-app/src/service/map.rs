//! map 域编排（自 `routes/map.rs` 原样搬迁，零语义改动）。
//!
//! 四类编排：①地图读取 ②巡检 ③增量/全量归纳 ④子图分析 + 队列宿主桥。
//! HTTP 边界（入参解析 / ETag / 状态码 / 响应包装）留在 `routes/map.rs`。

use crate::assets::{self, resolve_text_asset};
use easyvibe_map::freshness;
use easyvibe_map::concerns::{assign_concern_ids, diff_concerns, extract_concerns};
use crate::state::*;
use easyvibe_ai_agent::agent_conf;
use easyvibe_api_types::SessionStatusChanged;
use easyvibe_common::ApiError;
use easyvibe_db::HealthRepository as _;
use easyvibe_event_bus::queue::{JobKind, QueueHost, QueuedJob};
use easyvibe_event_bus::{publish, BusEvent};
use tracing::info;

/// 自定义补充 global 块（方案 v2 注入点 #5-#7：归纳/子图/巡检 prompt 尾部追加）。
/// **用透明中和副本**——用户补充里的拷问类指令不得漏进透明 agent（2026-10-05 实弹）。
/// spawn 时刻读锁——custom 保存后对后续会话生效（热生效语义，与任务链一致）。
pub(crate) async fn global_custom_block(harness: &std::sync::Arc<tokio::sync::RwLock<crate::task_exec::Harness>>) -> String {
    crate::task_exec::custom_block(&harness.read().await.custom_neutral.global)
}

/// ① 地图读取：返回 (map body, etag)。ETag 比较与响应头留在 handler。
pub(crate) async fn load_map_with_etag(st: &AppState, id: &str) -> Result<(serde_json::Value, String), ApiError> {
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let snap = st.map_service.load_map(&repo).await?;
    let etag = format!("\"{}\"", snap.content_hash);
    Ok((snap.json, etag))
}

/// ① 地图读取：growth.log 事件数组（与文件行一致，前端状态机统一消费）
pub(crate) async fn load_growth(st: &AppState, id: &str) -> Result<serde_json::Value, ApiError> {
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    Ok(serde_json::json!(st.map_service.load_growth(&repo).await?))
}

/// ① 地图读取：子图。路径遍历防线：module_id 必须满足 Schema 的 id 字符集（审查 🔴1）
pub(crate) async fn load_submap(st: &AppState, id: &str, module_id: &str) -> Result<serde_json::Value, ApiError> {
    if !easyvibe_map::is_valid_id(module_id) {
        return Err(ApiError::BadRequest(format!("非法模块 id: {module_id}")));
    }
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    Ok(st.map_service.load_submap(&repo, module_id).await?)
}

/// ① 地图读取：S2 地图保鲜状态（§13.4——stale 地图上的对话/建议/健康分全是假数据自信工作）
pub(crate) async fn assess_freshness(st: &AppState, id: &str) -> Result<serde_json::Value, ApiError> {
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let snap = st.map_service.load_map(&repo).await?;
    let f = freshness::assess(&repo.root, &snap.json);
    Ok(serde_json::json!({
        "status": f.status.as_str(),
        "mapGeneratedAt": f.map_generated_at,
        "latestCommitAt": f.latest_commit_at,
        "commitsSinceMap": f.commits_since_map,
    }))
}

/// ① 地图读取：S2 模块健康历史（趋势图数据面；module_health_history 自 M2-4 落库以来的第一个消费者）
pub(crate) async fn list_health_history(st: &AppState, id: &str, module_id: &str) -> Result<serde_json::Value, ApiError> {
    if !easyvibe_map::is_valid_id(module_id) {
        return Err(ApiError::BadRequest(format!("非法模块 id: {module_id}")));
    }
    st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let rows = st.health_repo.list_module_history(id, module_id, 20).await?;
    Ok(serde_json::json!(rows))
}

/// 归纳完成超时防线（实弹#4：FENJUE 首归纳产物/进度俱齐，但 CLI 不收尾、会话恒 Running，
/// 前端"归纳中"假卡住）：progress.json phase=done 且文件落盘超过 grace 秒 → 判 agent 已交付。
/// 用文件 mtime（系统时间基准）而非 progress.updated_at（RFC3339 带时区，秒级判定会被时区坑）。
pub(crate) fn progress_done_ago_secs(repo_root: &std::path::Path) -> Option<u64> {
    let path = repo_root.join(".easyvibe/map/progress.json");
    let content = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    if v["phase"].as_str() != Some("done") {
        return None;
    }
    let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
    Some(std::time::SystemTime::now().duration_since(modified).ok()?.as_secs())
}

/// 2026-10-04 实弹修复：收尸的"进度 100%"必须出自本次会话——旧 progress.json
/// （上一次归纳留下的 done + 旧 mtime）会让新会话秒被杀（用户点"立即归纳"3 秒复活）。
/// 只有 mtime 晚于会话启动时间，才承认这个 done 是本会话产物。
pub(crate) fn progress_done_ago_secs_since(repo_root: &std::path::Path, since: std::time::SystemTime) -> Option<u64> {
    let path = repo_root.join(".easyvibe/map/progress.json");
    let content = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    if v["phase"].as_str() != Some("done") {
        return None;
    }
    let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
    if modified < since {
        return None;
    }
    Some(std::time::SystemTime::now().duration_since(modified).ok()?.as_secs())
}

/// ① 地图读取：S2.5 归纳进度（progress.json 透传——首归纳等待页显示真实阶段/百分比，
/// 不再只转圈；文件缺失（如巡检场景无 progress）返回 null，前端不渲染进度）
pub(crate) async fn load_progress(st: &AppState, id: &str) -> Result<serde_json::Value, ApiError> {
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let path = repo.root.join(".easyvibe/map/progress.json");
    if !path.exists() {
        return Ok(serde_json::Value::Null);
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| ApiError::Internal(format!("progress 读取失败: {e}")))?;
    let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| ApiError::Internal(format!("progress 解析失败: {e}")))?;
    Ok(v)
}

/// ④ 子图深入分析（试用反馈"子图加载失败"根因修复——v2.2 归纳不产子图，文件无人生产；
/// 此处把缺口变为能力：透明 agent 扫描模块文件产出子图，落盘 .easyvibe/modules/<id>.json，
/// 与归纳共用写互斥/会话机制。读时拉取无需 watcher，产出后重新展开即见）
pub(crate) async fn analyze_submap(st: &AppState, id: &str, module_id: &str) -> Result<serde_json::Value, ApiError> {
    if !easyvibe_map::is_valid_id(module_id) {
        return Err(ApiError::BadRequest(format!("非法模块 id: {module_id}")));
    }
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let snap = st.map_service.load_map(&repo).await?;
    let module = snap.json["modules"]
        .as_array()
        .and_then(|ms| ms.iter().find(|m| m["id"].as_str() == Some(module_id)).cloned())
        .ok_or_else(|| ApiError::NotFound(format!("模块 {module_id} 不在主地图中")))?;
    // 子图提示词每次请求重读（产品内置协议迭代快——避免"改了提示词要重启后端"的叠加
    // （本次实弹：路径修正后的提示词因后端未重启而仍用旧版，DeskWar 两次分析空跑）。
    // 读取失败回退启动时装载的副本，绝不阻断
    let submap_spec = assets::spec("submap");
    let (template, _) = resolve_text_asset(submap_spec.env, submap_spec.name, &st.submap_prompt, submap_spec.persist);
    ensure_agent_available(st)?;
    let prompt = template
        .replace("<REPO_ROOT>", &repo.root.to_string_lossy())
        .replace("<MODULE_ID>", module_id)
        .replace("<MODULE_JSON>", &serde_json::to_string(&module).unwrap_or_default());
    // 注入点 #6：自定义 global 块追加到子图分析 prompt 尾部
    let prompt = format!("{prompt}{}", global_custom_block(&st.harness).await);
    let resolved = agent_conf::resolve_agent(&st.settings_repo, None, &st.agent_command, &st.agent_args).await;
    let session = st
        .session_manager
        .start_induction(&repo.id, &repo.root, &prompt, &resolved.command, &resolved.args, None)
        .await?;
    // I1：气泡标签「分析模块 {name}」（name 缺失降级为 id）
    let module_name = module["name"].as_str().unwrap_or(module_id).to_string();
    st.session_manager.note_label(&session.session_id, format!("分析模块 {module_name}")).await;
    // L1 归因：子图会话直接归属模块（任务会话在 task_exec 经 tasks.modules 反查）
    if let Err(err) = st.agent_session_repo.set_module_id(&session.session_id, module_id).await {
        tracing::warn!("[agent_sessions] 子图归因失败 {}: {err}", session.session_id);
    }
    info!("[submap] 模块 {} 子图分析会话 {} 已启动", module_id, session.session_id);
    Ok(serde_json::json!(session))
}

/// 触发重新归纳（写路径，M2-3）：spawn 外部 agent 按 v2.2 协议执行，
/// 三通道（progress/growth.log/map.json）由 watcher 自动直播，前端无需轮询
pub(crate) async fn start_reinduce(st: &AppState, id: &str, force_full: bool) -> Result<serde_json::Value, ApiError> {
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    ensure_agent_available(st)?;
    // 实弹验证发现：agent 可能"成功退出但什么都没写"（如模型能力不足只输出分析），
    // 因此记录会话前的地图哈希，终态后比对——未变化则告警（会话仍算成功：重归纳产出相同内容合法）。
    // 同时为增量判定（旧图合法性/体积）与全量 strict 失败还原旧图备好快照。
    let snap_before = st.map_service.load_map(&repo).await.ok();
    let hash_before = snap_before.as_ref().map(|s| s.content_hash);
    // B 方案判定：drain 到执行时现判（不在入队时判——session_queue_routes 不改）。
    // 全部满足才增量：旧图合法 + 锚点合法 + ≤3 提交 + 变更 ≤20 文件/2000 行 + git 仓库 + 旧图 ≤512KB。
    let mode = if force_full {
        easyvibe_map::induction::ReinduceMode::Full
    } else {
        easyvibe_map::induction::decide(&repo.root, snap_before.as_ref().map(|s| &s.json))
    };
    // 决策时 HEAD：增量时复用判定内读到的 HEAD（读一次、两用——diff 上界即成功后要写的锚点）；
    // 全量模式单独读一次，供终态锚点推进比对。非 git 为 None（不推进锚点）。
    let decision_head = match &mode {
        easyvibe_map::induction::ReinduceMode::Incremental(ctx) => Some(ctx.head.clone()),
        easyvibe_map::induction::ReinduceMode::Full => easyvibe_map::induction::head_sha(&repo.root),
    };
    let resolved = agent_conf::resolve_agent(&st.settings_repo, None, &st.agent_command, &st.agent_args).await;
    // prompt：增量模式预渲染全部占位符（<REPO_ROOT> 留给会话层，仿 patrol 先例），
    // custom global 块拼在增量 prompt 尾部（白名单纪律段之后——custom 若要求全量扫描，以增量白名单为准）
    let prompt = match &mode {
        easyvibe_map::induction::ReinduceMode::Incremental(ctx) => {
            // 每次 spawn 重读（与 submap 同一纪律：提示词迭代免重启）
            let inc_spec = assets::spec("incremental");
            let (template, _) =
                resolve_text_asset(inc_spec.env, inc_spec.name, &st.incremental_prompt, inc_spec.persist);
            let current_map = snap_before
                .as_ref()
                .and_then(|s| serde_json::to_string(&s.json).ok())
                .unwrap_or_default();
            let rendered = easyvibe_map::induction::render_incremental_prompt(&template, ctx, &current_map, &st.schema_path);
            format!("{rendered}{}", global_custom_block(&st.harness).await)
        }
        easyvibe_map::induction::ReinduceMode::Full => {
            // 注入点 #5：自定义 global 块追加到归纳 prompt 尾部
            format!("{}{}", st.prompt_template, global_custom_block(&st.harness).await)
        }
    };
    let session = st
        .session_manager
        .start_induction(&repo.id, &repo.root, &prompt, &resolved.command, &resolved.args, None)
        .await?;
    // I1：气泡标签「归纳」（增量回退的全量任务同样——前端契约不感知模式）
    st.session_manager.note_label(&session.session_id, "归纳".into()).await;

    // 终态后产物核验
    // R3 P0-2：必须按「自己 spawn 的会话」轮询（status_of_session），不能用仓库级 status_of——
    // 归纳终态后 2s 窗内新起的巡检/重归纳会话会被本循环误读，grace 收尸会把别的会话误判 Succeeded
    let st2 = st.clone();
    let repo2 = repo.clone();
    let session_id = session.session_id.clone();
    let started = std::time::SystemTime::now(); // 收尸防线的时间锚：只认本会话写出的 100%（2026-10-04 实弹）
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            match st2.session_manager.status_of_session(&session_id).await {
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
                    // 终态按 mode 执行收尾：增量 = 读 patch→合成→校验→落盘/回退入队；全量 = strict 终态校验
                    match &mode {
                        easyvibe_map::induction::ReinduceMode::Incremental(ctx) => {
                            crate::service::reinduce::handle_incremental_terminal(&st2, &repo2, ctx, &decision_head).await;
                        }
                        easyvibe_map::induction::ReinduceMode::Full => {
                            crate::service::reinduce::handle_full_terminal(&st2, &repo2, &snap_before, &decision_head).await;
                        }
                    }
                    // 归纳终态：立即重估保鲜并推送 freshness.changed——否则头部"落后提示"要等
                    // 30 分钟定时器才刷新（2026-10-05 实弹：归纳完成后 chip 仍显示"已过时 1 个新提交"）
                    if let Ok(snap) = st2.map_service.load_map(&repo2).await {
                        let f = easyvibe_map::freshness::assess(&repo2.root, &snap.json);
                        publish(&st2.event_bus, BusEvent::Freshness {
                            repo: repo2.id.clone(),
                            status: f.status.as_str().to_string(),
                            latest_commit_at: f.latest_commit_at,
                            commits_since_map: f.commits_since_map,
                        });
                    }
                    break;
                }
                // 实弹#4 防线：进度 100% 落盘超过 90s 但会话仍 Running（agent 已交付未自行退出）
                // → 按成功收尸解除"归纳中"假卡住。产物合法性由 watcher 校验保证（不出残图）。
                // 重审 P1 修订：收尸必须连进程一起杀（grace_finish）——此前只记终态不杀，
                // 已交付但不退出的 claude 成为无法击杀的僵尸（kill 对已终态会话返回 409），
                // 只能等自然退出或后端退出（kill_on_drop）才释放。
                Some(s) => {
                    const GRACE_SECS: u64 = 90;
                    if let Some(ago) = progress_done_ago_secs_since(&repo2.root, started) {
                        if ago > GRACE_SECS {
                            tracing::warn!(
                                "[reinduce] 进度 100% 已落盘 {}s 但会话仍未退出——按成功收尸并终止进程（agent 未自行退出，实弹#4 + 重审 P1）",
                                ago
                            );
                            let _ = st2.session_manager.grace_finish(&s.session_id).await;
                            break;
                        }
                    }
                }
                None => {}
            }
        }
    });

    Ok(serde_json::json!(session))
}

/// 触发巡检（M2-4，实弹 #2 修订）：两条执行路径——
/// - Stub 模式：ai-agent PatrolService 零成本确定性巡检（测试/demo）
/// - 真实模式：**session spawn**（与归纳同路径）。实弹发现直调无 tools 声明的 API
///   只会得到模型的 tool_call 幻觉（DSML 伪调用），真实巡检需要 Bash 核查
///   （wc/git/grep），是工具型任务，必须由带工具的 agent 执行。
/// 两条路径共用写互斥（try_register），终态后健康历史落域 2。
pub(crate) async fn start_patrol(st: &AppState, id: &str) -> Result<serde_json::Value, ApiError> {
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    if *st.llm_mode == LlmMode::Anthropic {
        ensure_agent_available(st)?;
    }
    let run_id = format!("patrol-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));

    match *st.llm_mode {
        LlmMode::Stub => {
            st.session_manager
                .try_register(SessionStatusChanged { repo: repo.id.clone(), session_id: run_id.clone(), status: easyvibe_api_types::SessionStatus::Running })
                .await?;
            // I1：气泡标签「巡检」
            st.session_manager.note_label(&run_id, "巡检".into()).await;
            let snap = match st.map_service.load_map(&repo).await {
                Ok(s) => s,
                Err(e) => {
                    st.session_manager.note_status(SessionStatusChanged { repo: repo.id.clone(), session_id: run_id.clone(), status: easyvibe_api_types::SessionStatus::Failed }).await;
                    return Err(e);
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
                st2.session_manager.note_status(SessionStatusChanged { repo: repo2.id.clone(), session_id: run_id_task.clone(), status }).await;
                // R3 C1：巡检终态广播——前端解除"巡检中"并刷新健康看板
                publish(&st2.event_bus, BusEvent::PatrolFinished {
                    repo: repo2.id,
                    run_id: run_id_task,
                    status: format!("{:?}", status).to_lowercase(),
                });
                if let Err(e) = result {
                    tracing::warn!("[patrol] 失败: {e}");
                }
            });
            Ok(serde_json::json!({ "started": true, "runId": run_id, "mode": "stub" }))
        }
        LlmMode::Anthropic => {
            // 2026-10-05 问题项新旧对照：spawn 前快照上轮 concerns（必须在 start_induction 之前——
            // 窗口期内 agent 完成原子写回后快照会拿到新图，对照全错；评审#S1/S6）。
            // 读失败降级为空快照（等同首巡语义），不得阻断巡检启动。
            let snap = st.map_service.load_map(&repo).await.ok();
            let old_concerns = snap.as_ref().map(|s| extract_concerns(&s.json)).unwrap_or_default();
            // 评审#S4：占位符在 agent 路径原样传入（会话层只替换 <REPO_ROOT>）——
            // 此处预渲染 <CURRENT_MAP>（顺带解决上轮地图不可见的 id 继承依据）与 <SCHEMA_PATH>。
            let prompt = match snap.as_ref().and_then(|s| serde_json::to_string(&s.json).ok()) {
                Some(map_json) if map_json.len() < 512 * 1024 => st
                    .patrol_prompt
                    .replace("<CURRENT_MAP>", &map_json)
                    .replace("<SCHEMA_PATH>", &st.schema_path),
                _ => st.patrol_prompt.to_string(),
            };
            // 注入点 #7：自定义 global 块追加到巡检 prompt 尾部
            let prompt = format!("{prompt}{}", global_custom_block(&st.harness).await);
            // 真实巡检 = 工具型执行：spawn 带工具的 CLI agent，prompt 要求原子写回 map.json
            let resolved = agent_conf::resolve_agent(&st.settings_repo, None, &st.agent_command, &st.agent_args).await;
            // 秒退假成功防线：记录 spawn 前 map.json mtime——agent 进程退出码 0 但没写回
            // 地图 = 空转（实弹：13:20 巡检 started==finished 同毫秒，分数照抄旧图当成功）
            let pre_mtime = std::fs::metadata(repo.map_path()).and_then(|m| m.modified()).ok();
            let session = st
                .session_manager
                .start_induction(&repo.id, &repo.root, &prompt, &resolved.command, &resolved.args, None)
                .await?;
            // I1：气泡标签「巡检」
            st.session_manager.note_label(&session.session_id, "巡检".into()).await;
            // 终态后：解析产物地图，健康历史落域 2（succeeded 但产物缺 health 也算失败记录）
            // run_id 用时间戳独立生成，不复用 session_id——会话计数器在后端重启后归零，
            // 会与历史 patrol_runs 行主键碰撞导致落库失败（实弹：UNIQUE constraint failed）
            let st2 = st.clone();
            let repo2 = repo.clone();
            // R3 P0-2：同 reinduce——按自己 spawn 的会话轮询，不用仓库级 status_of（多会话交错会把
            // 别的会话终态写进健康历史，归错 run）
            let session_id = session.session_id.clone();
            let run_id = format!("patrol-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
            let run_id_task = run_id.clone();
            let model = st.agent_command.to_string();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let Some(s) = st2.session_manager.status_of_session(&session_id).await else { continue };
                    if matches!(s.status, easyvibe_api_types::SessionStatus::Starting | easyvibe_api_types::SessionStatus::Running) { continue }
                    let mut succeeded = s.status == easyvibe_api_types::SessionStatus::Succeeded;
                    let mut fail_reason: Option<String> = None;
                    // 秒退假成功防线：退出码 0 但 map.json mtime 未前进 = agent 空转
                    if succeeded {
                        let post_mtime = std::fs::metadata(repo2.map_path()).and_then(|m| m.modified()).ok();
                        if matches!((pre_mtime, post_mtime), (Some(a), Some(b)) if a == b) {
                            succeeded = false;
                            fail_reason = Some("秒退假成功防线：agent 未写回地图（map.json mtime 未变）——按失败记录".into());
                            tracing::warn!("[patrol] {}", fail_reason.as_deref().unwrap_or_default());
                        }
                    }
                    let result = async {
                        let mut snap = st2.map_service.load_map(&repo2).await?;
                        let diff;
                        if succeeded {
                            // id 兜底注入（LLM 不遵守 id 规则时由后端保证稳定）+ 原子写回
                            if assign_concern_ids(&old_concerns, &mut snap.json) > 0 {
                                easyvibe_map::atomic_write_json(&repo2.map_path(), &snap.json).await?;
                                info!("[patrol] concerns id 兜底注入完成并写回 map.json");
                            }
                            diff = Some(diff_concerns(&old_concerns, &snap.json).to_string());
                        } else {
                            // 仅 succeeded 写对照（评审#Q1：失败时读到的可能是未写回旧图，存了反而误导）
                            diff = None;
                        }
                        st2.patrol_service
                            .record_from_map(&run_id_task, &repo2.id, &model, &snap.json, succeeded, fail_reason, diff)
                            .await
                    }
                    .await;
                    if let Err(e) = result {
                        tracing::warn!("[patrol] 健康历史落库失败: {e}");
                    }
                    // R3 C1：巡检终态广播（健康历史已落库）——前端解除"巡检中"并刷新看板
                    // 广播用防线修正后的状态（秒退空转报 failed，前端不会误以为成功）
                    publish(&st2.event_bus, BusEvent::PatrolFinished {
                        repo: repo2.id.clone(),
                        run_id: run_id_task.clone(),
                        status: if succeeded { "succeeded".into() } else { "failed".into() },
                    });
                    // 巡检也会写回地图：即时重估保鲜并推送（与归纳终态同一纪律——不等 30 分钟定时器）
                    if let Ok(snap) = st2.map_service.load_map(&repo2).await {
                        let f = easyvibe_map::freshness::assess(&repo2.root, &snap.json);
                        publish(&st2.event_bus, BusEvent::Freshness {
                            repo: repo2.id.clone(),
                            status: f.status.as_str().to_string(),
                            latest_commit_at: f.latest_commit_at,
                            commits_since_map: f.commits_since_map,
                        });
                    }
                    break;
                }
            });
            Ok(serde_json::json!({ "started": true, "sessionId": session.session_id, "runId": run_id, "mode": "agent" }))
        }
    }
}

/// 解环适配：server-api 侧实现队列宿主契约，向 event-bus 状态机注入
/// 「发事件 / 查活动会话 / 执行任务」三项能力。执行入口回指 start_* 编排函数
/// 属正向的 server-api → task-engine/自身逻辑，不构成 task-engine → server-api 反向边。
#[async_trait::async_trait]
impl QueueHost for AppState {
    fn publish_event(&self, ev: BusEvent) {
        publish(&self.event_bus, ev);
    }

    async fn has_active_session(&self, repo: &str) -> bool {
        matches!(
            self.session_manager.status_of(repo).await,
            Some(s) if matches!(
                s.status,
                easyvibe_api_types::SessionStatus::Starting | easyvibe_api_types::SessionStatus::Running
            )
        )
    }

    async fn run_job(&self, repo: &str, job: &QueuedJob) -> Result<(), ApiError> {
        match job.kind {
            JobKind::Patrol => start_patrol(self, repo).await.map(|_| ()),
            JobKind::Reinduce => start_reinduce(self, repo, job.force_full).await.map(|_| ()),
            JobKind::Submap => {
                let module_id = job.module_id.clone().unwrap_or_default();
                analyze_submap(self, repo, &module_id).await.map(|_| ())
            }
        }
    }
}
