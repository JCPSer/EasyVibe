//! map 域：共享前置 + 读取路径 + 队列宿主桥（c-arch-13 R2：三条写流程各自收口后留驻本文件）。
//!
//! 写路径已按流程迁出：巡检 → `service::patrol`、子图分析 → `service::submap`、
//! 归纳启动 → `service::reinduce_start`（归纳终态收尾本就在 `service::reinduce`）。
//! 本文件保留：①地图读取 ②进程知识（`progress_done_ago_secs*`）③共享前置 `global_custom_block`
//! ④`impl QueueHost for AppState`（三分支回指三条流程入口——同层正向回指，非反向边）。
//! HTTP 边界（入参解析 / ETag / 状态码 / 响应包装）留在 `routes/map.rs`。

use crate::db_ports::HealthPort as _;
use crate::state::*;
use easyvibe_common::ApiError;
use easyvibe_event_bus::queue::{JobKind, QueueHost, QueuedJob};
use easyvibe_event_bus::{publish, BusEvent};
use easyvibe_map::freshness;

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
            JobKind::Patrol => crate::service::patrol::start_patrol(self, repo).await.map(|_| ()),
            JobKind::Reinduce => crate::service::reinduce_start::start_reinduce(self, repo, job.force_full).await.map(|_| ()),
            JobKind::Submap => {
                let module_id = job.module_id.clone().unwrap_or_default();
                crate::service::submap::analyze_submap(self, repo, &module_id).await.map(|_| ())
            }
        }
    }
}
