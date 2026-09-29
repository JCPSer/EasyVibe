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
