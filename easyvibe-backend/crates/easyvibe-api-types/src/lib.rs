//! REST/WS 契约类型的唯一定义处。禁止依赖 axum/tower 等 HTTP 框架（见 backend-design.md §3）。
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub id: String,
    pub name: String,
    pub root: String,
}

/// WS 消息包：{"name": "domain.actionName", "data": {...}}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsMessage<T: Serialize> {
    pub name: String,
    pub data: T,
}

/// map.changed 事件的 data
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapChanged {
    pub repo: String,
    pub version: String,
}

/// map.invalid 事件的 data
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapInvalid {
    pub repo: String,
    pub error: String,
}

/// growth.log 事件（v2.2 协议；data 原样透传）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrowthEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(flatten)]
    pub payload: serde_json::Value,
}

/// 会话状态（M2-3 起）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Starting,
    Running,
    Succeeded,
    Failed,
}

/// session.statusChanged 事件的 data
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatusChanged {
    pub repo: String,
    pub session_id: String,
    pub status: SessionStatus,
}

// ---------------------------------------------------------------------------
// R5（c-arch-1 契约面收敛）：WS 事件名与 payload 结构入册。
// 事件名清单是**单一事实源**；后端 `ws.rs::translate` 与前端 `runtime/ws.ts` 的
// `msg.name` 订阅三方必须一致，由 `tests/contract_guard.rs` 交叉校验（改名即红）。
// ---------------------------------------------------------------------------

/// WS 事件名清单（10 类）。`map.invalid` 为内部失效通知（前端不按 name 订阅），
/// 是否计入以既有前端 `FROZEN_WS_EVENTS` 口径为准——此处与其全等。
pub const WS_EVENTS: &[&str] = &[
    "map.changed",
    "growth.event",
    "session.statusChanged",
    "queue.changed",
    "session.output",
    "patrol.finished",
    "freshness.changed",
    "task.contractAlert",
    "task.contractViolated",
    "task.statusChanged",
];

/// task.statusChanged 事件的 data
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStatusChanged {
    pub repo: String,
    pub task_id: String,
    pub status: String,
    pub gate: Option<String>,
}

/// task.contractAlert / task.contractViolated 事件的 data（同一形状）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskContractNotice {
    pub repo: String,
    pub task_id: String,
    pub files: Vec<String>,
}

/// freshness.changed 事件的 data
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreshnessChanged {
    pub repo: String,
    pub status: String,
    pub latest_commit_at: Option<i64>,
    pub commits_since_map: Option<i64>,
}

/// session.output 事件的 data（终端行直播）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOutput {
    pub session_id: String,
    pub seq: u64,
    pub stream: String,
    pub line: String,
}

/// patrol.finished 事件的 data（巡检终态）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatrolFinished {
    pub repo: String,
    pub run_id: String,
    pub status: String,
}

/// queue.changed 事件的 data（会话队列变更的载荷由 `QueueChange::to_payload` 决定，
/// 含 type/job/started/error 等自由形状字段——显式 `Value` 透传，见 §2.3.2 豁免清单）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueChanged {
    pub repo: String,
    #[serde(flatten)]
    pub payload: serde_json::Value,
}
