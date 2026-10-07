//! 巡检流程编排（c-arch-13 R2：自 `service/map.rs` 原样搬迁，零语义改动）。
//!
//! 两条执行路径（Stub / Anthropic）+ 秒退假成功防线 + 健康历史落库 + 终态事件广播。
//! 共享前置 `global_custom_block` 与 `progress_done_ago_secs*` 留驻 `service::map`（唯一规范路径）。

use crate::service::map::global_custom_block;
use crate::state::*;
use easyvibe_ai_agent::agent_conf;
use easyvibe_api_types::SessionStatusChanged;
use easyvibe_common::ApiError;
use easyvibe_event_bus::{publish, BusEvent};
use easyvibe_map::concerns::{assign_concern_ids, diff_concerns, extract_concerns};
use tracing::info;

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
