//! 装配格 · 定时器 / 启动探测 / 队列宿主（后台任务工厂）。
//!
//! c-arch-10 R2/R3：自 `bootstrap.rs` 内联闭包**纯搬运**；唯一语义修订是 auto-patrol 的
//! 触发入口由「手工构造 axum `State`/`Path` 调 routes handler」改为直接调
//! `crate::service::map::start_patrol`，消除装配格 → routes 的层次倒置（触发条件与日志不变）。

use crate::state::AppState;
use easyvibe_event_bus::queue::QueueState;
use easyvibe_event_bus::{publish, BusEvent};
use easyvibe_map::{freshness, MapService};
use easyvibe_session::SessionManager;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::info;

/// 地图保鲜定时器：每轮读 `adv.freshnessCheckMinutes`（默认 30，max(1)）；
/// 状态变化或非 fresh 才推 `freshness.changed`（只检查不自动重归纳）。
pub(crate) fn spawn_freshness(
    map_service: Arc<MapService>,
    settings_repo: Arc<easyvibe_db::SqliteSettingsRepository>,
    event_bus: broadcast::Sender<BusEvent>,
) {
    use easyvibe_db::SettingsRepository as _;
    let bus = event_bus;
    let maps = map_service;
    let settings = settings_repo;
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

/// 启动即探测执行 agent（异步——探测失败只是 missing 态，不阻断启动）。
pub(crate) fn spawn_agent_detect(state: AppState) {
    let st_detect = state;
    tokio::spawn(async move {
        let detected = easyvibe_ai_agent::agent_conf::detect_agents().await;
        info!("[agent] 启动探测完成：检测到 {} 个", detected.len());
        *st_detect.agent_detected.write().await = detected;
    });
}

/// 定时巡检（默认关：`adv.autoPatrolEnabled=true` 开启，间隔 `adv.autoPatrolHours` 默认 24h）。
/// 无活动会话才触发（写互斥天然排队）。
pub(crate) fn spawn_auto_patrol(state: AppState) {
    use easyvibe_db::{HealthRepository as _, SettingsRepository as _};
    let st_for_patrol = state;
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
                let fresh_enough = st_for_patrol.health_repo.list_runs(&repo.id, 1).await.ok()
                    .and_then(|runs| runs.first().and_then(|r| r.started_at.parse::<i64>().ok()))
                    .map(|t| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64 - t < hours * 3600)
                    .unwrap_or(false);
                if fresh_enough { continue }
                info!("[auto-patrol] {} 距上次巡检超 {}h，自动触发", repo.id, hours);
                let st2 = st_for_patrol.clone();
                let repo_id = repo.id.clone();
                tokio::spawn(async move {
                    if let Err(e) = crate::service::map::start_patrol(&st2, &repo_id).await {
                        tracing::warn!("[auto-patrol] {} 触发失败: {:?}", repo_id, e);
                    }
                });
            }
        }
    });
}

/// 队列排空（需求 v1 §4.1/§8）：① 事件驱动——终态 SessionStatus 且
/// 「事件 session_id 经 status_of_session 确认为终态」；② 12s 清扫器兜底（B3）。
pub(crate) fn spawn_queue_drain(state: AppState, session_manager: Arc<SessionManager>) {
    let st = state.clone();
    tokio::spawn(async move {
        let mut rx = st.event_bus.subscribe();
        while let Ok(e) = rx.recv().await {
            let BusEvent::SessionStatus(s) = e else { continue };
            if !matches!(s.status, easyvibe_api_types::SessionStatus::Succeeded | easyvibe_api_types::SessionStatus::Failed) {
                continue;
            }
            if let Some(cur) = session_manager.status_of_session(&s.session_id).await {
                if matches!(cur.status, easyvibe_api_types::SessionStatus::Starting | easyvibe_api_types::SessionStatus::Running) {
                    continue;
                }
                st.session_queue.drain(&st, &s.repo).await;
            }
        }
    });
    tokio::spawn(QueueState::sweep_loop(state.session_queue.clone(), state));
}
