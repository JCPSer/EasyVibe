//! 槽位 agent 解析端口适配（c-arch-16 R7 自 task_engine.rs 纯搬家；单一端口域 = 设置仓储）。
//!
//! 口径注记：本文件另引 `easyvibe_ai_agent::agent_conf::resolve_agent`（**非** `easyvibe_db`，
//! 不在 R6「db_ports 单域断言」的约束面内）；其保留理由见方案 §R1 的 agent-runtime 裁决。

use crate::task_exec::ports::AgentSlotResolver;
use std::sync::Arc;

/// 槽位 agent 解析适配：settings 优先解析链（agent_conf::resolve_agent）落组合根，
/// 切片只看见「按槽位解析」的端口语义。
pub(crate) struct AgentSlotAdapter {
    settings: Arc<easyvibe_db::SqliteSettingsRepository>,
    command: Arc<String>,
    args: Arc<Vec<String>>,
}

impl AgentSlotAdapter {
    pub(crate) fn new(
        settings: Arc<easyvibe_db::SqliteSettingsRepository>,
        command: Arc<String>,
        args: Arc<Vec<String>>,
    ) -> Self {
        Self { settings, command, args }
    }
}

#[async_trait::async_trait]
impl AgentSlotResolver for AgentSlotAdapter {
    async fn resolve(&self, slot: Option<&str>) -> easyvibe_ai_agent::agent_conf::ResolvedAgent {
        easyvibe_ai_agent::agent_conf::resolve_agent(&self.settings, slot, &self.command, &self.args).await
    }
}
