//! 基础层：错误类型、事件名常量、ID/时间工具。无任何内部依赖。
use serde::Serialize;

pub mod events {
    /// WS 事件名（两级 camelCase，见 backend-design.md §4）
    pub const MAP_CHANGED: &str = "map.changed";
    pub const MAP_INVALID: &str = "map.invalid";
    pub const GROWTH_EVENT: &str = "growth.event";
    pub const PROGRESS_UPDATED: &str = "progress.updated";
    pub const SESSION_STATUS_CHANGED: &str = "session.statusChanged";
    pub const AGENT_SLOT_UPDATED: &str = "agent.slotUpdated";
}

/// 统一 API 错误（状态码映射见 backend-design.md §5）
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("map invalid: {0}")]
    MapInvalid(String),
    #[error("internal: {0}")]
    Internal(String),
}

/// 统一成功响应包
#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self { success: true, data: Some(data), message: None }
    }
}

/// 统一错误响应包
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub success: bool,
    pub error: String,
    pub code: String,
}
