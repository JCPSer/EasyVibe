//! 归纳启动编排（c-arch-13 R2 降级支：自 `service/map.rs` 原样搬迁，零语义改动）。
//!
//! 与终态收尾（`service::reinduce`）同属「归纳」一条流程；因合并会越过单文件 400 行线，
//! 按方案 ΔS2 走**扁平双文件**降级支（`reinduce.rs` + `reinduce_start.rs`），不作目录化。
//! 共享前置 `global_custom_block` 与 `progress_done_ago_secs_since` 留驻 `service::map`。

use crate::assets::{self, resolve_text_asset};
use crate::service::map::{global_custom_block, progress_done_ago_secs_since};
use crate::state::*;
use easyvibe_ai_agent::agent_conf;
use easyvibe_common::ApiError;
use easyvibe_event_bus::{publish, BusEvent};

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
