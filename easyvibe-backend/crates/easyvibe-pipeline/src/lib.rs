//! 单仓库 watcher 管线：自动归纳 + map/growth/progress 三 watcher。
//! 启动挂载与 POST /api/repos 动态注册共用（新仓库热生效，无需重启）。
//!
//! 边界（由 `tests/module_size_guard.rs` 的禁用串表断言）：不含 HTTP 框架、应用共享状态，
//! 也不反向依赖应用层 crate。自定义层注入以**已解析的 `auto_init_suffix: String`** 入参传进来
//! （由持有该状态的应用层在 spawn 前读取），从而结构性切断到任务引擎的反向依赖。

use easyvibe_api_types::{MapChanged, MapInvalid};
use easyvibe_event_bus::{publish, BusEvent};
use easyvibe_map::{spawn_map_watcher, MapService, Repo};
use easyvibe_session::SessionManager;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::info;

/// D5：单仓库 watcher 管线——自动归纳（无合法地图时）+ map/growth/progress 三 watcher。
/// `auto_init_suffix` = 自定义层 global 块（方案 v2 注入点 #8：拼在 auto-init 归纳 prompt 尾部，
/// 由应用层在 spawn 时刻读锁解析后传入）。
pub async fn spawn_repo_pipeline(
    r: Repo,
    map_service: Arc<MapService>,
    event_bus: broadcast::Sender<BusEvent>,
    session_manager: Arc<SessionManager>,
    prompt_template: String,
    auto_init_suffix: String,
    agent_command: String,
    agent_args: Vec<String>,
) {
    // F1 打开仓库自动初始化（后端侧）：无合法地图的仓库启动即触发归纳，
    // 产物经 watcher 推送，前端 map.changed 后自动渲染
    if map_service.cached(&r.id).await.is_none() {
        info!("[auto-init] {} 无合法地图，自动触发归纳", r.id);
        // 注入点 #8：global 块拼在模板后（不影响 <REPO_ROOT> 占位——会话层替换发生在更下游）。
        // 用透明中和副本——与出厂框架同一中和防线（用户补充不得把拷问指令漏进透明 agent）
        let prompt = format!("{prompt_template}{auto_init_suffix}");
        match session_manager
            .start_induction(&r.id, &r.root, &prompt, &agent_command, &agent_args, None)
            .await
        {
            Ok(session) => {
                // I1 评审强制覆盖点：auto-init 自动归纳同样打气泡标签，不得遗漏
                session_manager.note_label(&session.session_id, "自动归纳".into()).await;
            }
            Err(e) => {
                tracing::warn!("[auto-init] {} 触发失败: {e}", r.id);
            }
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
                publish(&bus, BusEvent::Growth { repo: repo_id.clone(), event });
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
            publish(&bus, BusEvent::Progress { repo: repo_id.clone(), progress });
        }
    });
}
