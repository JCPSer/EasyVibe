//! 会话归属端口适配（c-arch-16 R7 自 task_engine.rs 纯搬家；单一端口域 = 会话仓储）。

use crate::task_exec::ports::SessionAttribution;
use easyvibe_common::ApiError;

/// 会话归属适配（tasks.modules 首个模块 → 会话行归因）。
#[async_trait::async_trait]
impl SessionAttribution for easyvibe_db::AgentSessionRepo {
    async fn set_module_id(&self, session_id: &str, module_id: &str) -> Result<(), ApiError> {
        easyvibe_db::AgentSessionRepo::set_module_id(self, session_id, module_id).await
    }
}
