//! 后端 → 前端事件类型与唯一出口。
//!
//! 原 `easyvibe-app/src/main.rs` 的 `BusEvent` / `publish` 逐字迁移，语义零改动
//! （尤其关键事件的 warn 留痕分支）。任务执行层与路由层均单向依赖本模块。

use easyvibe_api_types::{MapChanged, MapInvalid, SessionStatusChanged};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::queue::QueueChange;

/// 总线事件（内部枚举，发送时翻译为 WsMessage）
#[derive(Debug, Clone)]
pub enum BusEvent {
    MapChanged(MapChanged),
    MapInvalid(MapInvalid),
    Growth { repo: String, event: Value },
    Progress { repo: String, progress: Value },
    SessionStatus(SessionStatusChanged),
    TaskStatus { repo: String, task_id: String, status: String, gate: Option<String> },
    /// R2 裂缝#3：影响面合约越界（auto/supervised 无审批关，必须主动送达）
    TaskContractViolated { repo: String, task_id: String, files: Vec<String> },
    /// L2 过程预警：任务执行中哨兵巡检到的新增越界（预警 ≠ 终态红线判定）
    TaskContractAlert { repo: String, task_id: String, files: Vec<String> },
    /// S2：地图保鲜状态变化（git 有新提交而地图未更新——下游对话/建议/健康分全是假数据自信工作）
    Freshness { repo: String, status: String, latest_commit_at: Option<i64>, commits_since_map: Option<i64> },
    /// 改进#2：agent 过程直播——会话 stdout 行（子图分析/任务执行中的"它在干嘛"）
    SessionOutput { session_id: String, seq: u64, stream: String, line: String },
    /// R3 C1：巡检终态（健康历史落库后广播）——前端据此解除"巡检中"、刷新看板，
    /// 让"体检报告出来了"成为产品事件而不是用户刷新的猜测
    PatrolFinished { repo: String, run_id: String, status: String },
    /// 需求 v1 §5/§8：会话队列变更（入队/替换/取消/排空/重入队/失败）——气泡即时刷新
    QueueChanged { repo: String, change: QueueChange },
}

/// R3 C3：统一事件出口。broadcast 的 send 仅在"零订阅者"时失败（缓冲满不报错，而是 recv 端
/// Lagged——ws_handler 已记 warn）；关键事件（合约红线/预警）在此留痕，送达承诺必须可观测
pub fn publish(bus: &broadcast::Sender<BusEvent>, event: BusEvent) {
    let critical = matches!(
        event,
        BusEvent::TaskContractViolated { .. } | BusEvent::TaskContractAlert { .. }
    );
    if let Err(e) = bus.send(event) {
        if critical {
            tracing::warn!("[bus] 关键事件无订阅者被丢弃（红线送达承诺依赖可观测性）: {:?}", e.0);
        }
    }
}
