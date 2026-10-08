//! 仓库域编排：注册/注销的完整流程 + watcher 管线挂载（自 `routes/repo.rs` 与
//! `bootstrap.rs` 的调用点原样下沉，零语义改动）。
//!
//! HTTP 边界（入参解析 / 状态码 / Json 包装）留 `routes/repo.rs`；启动装配在 `bootstrap.rs`。

use crate::db_ports::{RepoMaintenancePort as _, TaskPort as _};
use crate::state::{read_desktop_repos, write_desktop_repos, AppState};
use easyvibe_api_types::RepoInfo;
use easyvibe_common::ApiError;
use easyvibe_event_bus::BusEvent;
use easyvibe_map::{repo_from_root, MapService, Repo};
use easyvibe_session::SessionManager;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::info;

/// 仓库 watcher 管线出边端口（c-arch-16 R3）：把 easyvibe-pipeline crate 的
/// `spawn_repo_pipeline` 唯一调用点收成端口，适配器落装配格（`assembly/ports.rs`）——
/// 本 trait 是 server-api 侧的**唯一**消费面，故 server-api 零 easyvibe-pipeline 路径字面量。
///
/// **无返回值**：语义与现行 `tokio::spawn(spawn_repo_pipeline(...))` 逐条一致（spawn 即返回）。
/// 8 个具名入参的顺序与 `spawn_repo_pipeline` 形参**逐条对齐**，无 pipeline 类型外泄
/// （参数全是 map / session / event-bus 类型与 String）。
///
/// **生命周期约束（ΔS4）**：本端口有**两个生命周期不同**的调用点——启动期（`assembly::run`，
/// `AppState` 尚不存在）与运行期（`add_repo`，持 `&AppState`）。故适配器须在装配格内**先构造一次**，
/// 启动循环直接复用该 `Arc`，并把**同一 `Arc`** 注入 `AppState.pipeline_port`；启动顺序不变量
/// （harness 装载 → 管线挂载 → 建库 → build_state）保持不变。
#[async_trait::async_trait]
pub trait RepoPipelinePort: Send + Sync + 'static {
    async fn spawn(
        &self,
        repo: Repo,
        map_service: Arc<MapService>,
        event_bus: broadcast::Sender<BusEvent>,
        session_manager: Arc<SessionManager>,
        prompt_template: String,
        auto_init_suffix: String,
        agent_command: String,
        agent_args: Vec<String>,
    );
}

/// D5：单仓库 watcher 管线挂载（启动装配与 `POST /api/repos` 共用）。
/// 注入点 #8：spawn 前读锁解析 custom global 块，以**显式字符串**传给 easyvibe-pipeline crate，
/// 从而切断 `easyvibe-pipeline → task-engine` 反向依赖（pipeline 不感知 Harness 类型）。
/// 参数解析（harness 读锁）留在 server-api 侧；领域调用经 `port` 走装配格适配器。
pub(crate) async fn spawn_pipeline(
    port: &Arc<dyn RepoPipelinePort>,
    map_service: Arc<MapService>,
    event_bus: broadcast::Sender<BusEvent>,
    session_manager: Arc<SessionManager>,
    prompt_template: String,
    agent_command: String,
    agent_args: Vec<String>,
    harness: Arc<RwLock<crate::task_exec::Harness>>,
    repo: Repo,
) {
    let auto_init_suffix = crate::task_exec::custom_block(&harness.read().await.custom_neutral.global);
    port.spawn(
        repo,
        map_service,
        event_bus,
        session_manager,
        prompt_template,
        auto_init_suffix,
        agent_command,
        agent_args,
    )
    .await;
}

/// 注销返回（HTTP `data` 形状：`{wiped}`，与原内联字面量逐字段一致）。
#[derive(serde::Serialize)]
pub(crate) struct WipeResult {
    pub(crate) wiped: u64,
}

/// D5 应用内仓库管理：动态注册（挂载 → 持久化 → 启动管线）。
pub(crate) async fn add_repo(st: &AppState, path: &str) -> Result<RepoInfo, ApiError> {
    let root = std::path::PathBuf::from(path.trim());
    if !root.is_dir() {
        return Err(ApiError::BadRequest(format!("目录不存在或不可读: {}", root.display())));
    }
    let repo = repo_from_root(&root);
    st.map_service.add_repo(repo.clone()).await?;
    // 持久化 + 启动该仓库的 watcher 管线（自动归纳由管线内决定）
    let mut roots = read_desktop_repos();
    if !roots.iter().any(|p| p == &root) {
        roots.push(root.clone());
        write_desktop_repos(&roots);
    }
    spawn_pipeline(
        &st.pipeline_port,
        st.map_service.clone(),
        st.event_bus.clone(),
        st.session_manager.clone(),
        (*st.prompt_template).clone(),
        (*st.agent_command).clone(),
        (*st.agent_args).clone(),
        st.harness.clone(),
        repo.clone(),
    )
    .await;
    info!("[repo-add] 动态注册 {} -> {}", repo.id, repo.root.display());
    Ok(RepoInfo { id: repo.id, name: repo.name, root: repo.root.to_string_lossy().into_owned() })
}

/// D5 应用内仓库管理：动态注销（杀会话 → 取消队列 → 可选数据清除 → 注销 → 持久化）。
/// 返回清除的行数（`wipe=false` 时为 0）。
pub(crate) async fn remove_repo(st: &AppState, id: &str, wipe: bool) -> Result<WipeResult, ApiError> {
    st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未挂载")))?;
    // 重审 P1：注销前杀活动会话——此前 running 的归纳/任务 agent 成孤儿，
    // 仓库写互斥被占死，进程只能等超时或后端退出（kill_on_drop）才释放
    if let Some(s) = st.session_manager.status_of(id).await {
        if matches!(s.status, easyvibe_api_types::SessionStatus::Starting | easyvibe_api_types::SessionStatus::Running) {
            let _ = st.session_manager.kill(&s.session_id).await;
        }
    }
    // 任务会话兜底（active 槽位只记最后一个注册者；running/awaiting 的任务逐个点杀，
    // 已终态会话 kill 返回 409 属预期，忽略）
    if let Ok(tasks) = st.task_repo.list(id, 500).await {
        for t in tasks.into_iter().filter(|t| matches!(t.status.as_str(), "running" | "awaiting_approval")) {
            if let Some(sid) = t.session_id {
                let _ = st.session_manager.kill(&sid).await;
            }
        }
    }
    // S1：顺带清该仓库队列项（kill 产生的终态事件会对已注销仓库触发 drain 空跑；
    // 广播 cancelled 让前端气泡即时清态）。解环后由 QueueState::cancel 完成「移除 + 广播」。
    let _ = st.session_queue.cancel(st, id).await;
    // 数据清除（?wipe=true）：抹掉该仓库在本地库的全部痕迹
    // （任务/审批/会话/消息/巡检历史/事件/仓库级设置）——默认保留，用户显式选择才清
    let wiped = if wipe { st.pool.wipe(id).await? } else { 0u64 };
    // 先取根再注销（注销后 find_repo 即查不到）
    let root = st.map_service.find_repo(id).await.map(|r| r.root);
    if !st.map_service.remove_repo(id).await {
        return Err(ApiError::NotFound(format!("仓库 {id} 未挂载")));
    }
    // 重审 P2 实弹 bug：remaining 从"本进程已挂载"算——dev 后端（env 只有 FENJUE）
    // 注销 FENJUE 会写出空文件，把 App 后端的 hover-client 行一并抹掉（桌面端"仓库消失"）
    // 持久化文件是跨进程共享事实源，必须以文件为基准：读文件 → 删被注销的根 → 写回
    let mut roots = read_desktop_repos();
    if let Some(root) = root {
        roots.retain(|p| p != &root);
    }
    write_desktop_repos(&roots);
    info!("[repo-remove] 注销 {}（数据清除 {} 行，文件剩 {} 个仓库）", id, wiped, roots.len());
    Ok(WipeResult { wiped })
}
