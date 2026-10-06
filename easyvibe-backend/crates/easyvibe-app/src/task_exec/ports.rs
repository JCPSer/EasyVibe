//! task_exec::ports —— task-engine 切片内的持久化端口与本地 DTO（依赖倒置，c-arch-7 R2）。
//!
//! 为什么：task-engine 与 server-api 是同层 application 模块，直接共享同一套具体仓储
//! （persistence 层）会让 schema 变更同时击穿两侧，且分层图上不可见。端口定义在**依赖方**（本切片），
//! 具体仓储的适配器落**组合根**（db_ports.rs）——与 `easyvibe-event-bus::queue::QueueHost`
//! 同构（端口在依赖方、适配器在编排层），依赖方向维持 server-api → task-engine，不成环。
//!
//! 注释纪律：本文件属 task_exec 生产文件，persistence 层 crate 名在此为**禁用字面量**（判据见
//! `tests/arch_guard.rs` 与 `scripts/check_app_db_boundary.py`）——故注释中亦不书写该标识符。

use easyvibe_common::ApiError;

/// 任务记录（本地 DTO）：字段与持久层任务行 **全等**，使 `task.gate` / `task.status` 等
/// 访问点在状态机内零改动；由组合根适配器做一次性字段搬运。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: String,
    pub repo: String,
    pub title: String,
    pub description: String,
    pub modules: String,
    pub acceptance: String,
    pub source: String,
    pub context: String,
    pub status: String,
    pub trust: String,
    pub error: Option<String>,
    pub session_id: Option<String>,
    pub gate: Option<String>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub result: Option<String>,
    pub base_head: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub conversation_id: Option<String>,
    pub origin_task_id: Option<String>,
    pub successor_task_id: Option<String>,
}

/// 审批留痕记录（本地 DTO）：id 与 decided_at 由组合根适配器补齐（调用方只关心语义留痕）。
#[derive(Debug, Clone)]
pub struct ApprovalRecord {
    pub task_id: String,
    pub gate: String,
    pub decision: String,
    pub note: Option<String>,
}

/// 任务读写端口（全部方法一一对应持久层语义；`try_*` 系列保留原子条件 UPDATE 的影响行数契约）。
#[async_trait::async_trait]
pub trait TaskStore: Send + Sync + 'static {
    async fn get(&self, id: &str) -> Result<Option<TaskRecord>, ApiError>;
    async fn list(&self, repo: &str, limit: i64) -> Result<Vec<TaskRecord>, ApiError>;
    async fn update_status(&self, id: &str, status: &str, error: Option<&str>) -> Result<(), ApiError>;
    async fn set_gate(&self, id: &str, gate: Option<&str>) -> Result<(), ApiError>;
    /// N27 原子关卡推进：影响行数 0 = 并发审批/状态漂移（调用方 409）。
    async fn try_advance_gate(
        &self,
        id: &str,
        expected_gate: Option<&str>,
        new_gate: Option<&str>,
        new_status: Option<&str>,
    ) -> Result<u64, ApiError>;
    async fn set_context(&self, id: &str, context: &str) -> Result<(), ApiError>;
    async fn reset_for_retry(&self, id: &str) -> Result<u64, ApiError>;
    async fn reset_for_remediate(&self, id: &str) -> Result<u64, ApiError>;
    async fn try_rewind(&self, id: &str, target_gate: &str) -> Result<u64, ApiError>;
    async fn set_session(&self, id: &str, session_id: &str) -> Result<(), ApiError>;
    async fn set_base_head(&self, id: &str, head: &str) -> Result<(), ApiError>;
    async fn set_result(&self, id: &str, result_json: &str) -> Result<(), ApiError>;
}

/// 审批落痕与查询端口（`list_by_task` 供「修改并复审」取最近一条打回意见）。
#[async_trait::async_trait]
pub trait ApprovalStore: Send + Sync + 'static {
    async fn record(&self, approval: &ApprovalRecord) -> Result<(), ApiError>;
    async fn list_by_task(&self, task_id: &str) -> Result<Vec<ApprovalRecord>, ApiError>;
}

/// 会话归属写端口（原 AgentSessionRepo 在切片内的唯一写点：任务 → 影响模块归因）。
#[async_trait::async_trait]
pub trait SessionAttribution: Send + Sync + 'static {
    async fn set_module_id(&self, session_id: &str, module_id: &str) -> Result<(), ApiError>;
}

/// 槽位 agent 解析端口（settings 优先解析链在组合根侧适配；返回既有 ResolvedAgent，零新面）。
#[async_trait::async_trait]
pub trait AgentSlotResolver: Send + Sync + 'static {
    async fn resolve(&self, slot: Option<&str>) -> easyvibe_ai_agent::agent_conf::ResolvedAgent;
}

/// 固定槽位解析器：不读设置、直接返回给定命令/参数（测试夹具专用——切片内不得引用组合根适配器）。
#[cfg(test)]
pub struct FixedAgentSlot {
    pub command: std::sync::Arc<String>,
    pub args: std::sync::Arc<Vec<String>>,
}

#[cfg(test)]
impl FixedAgentSlot {
    pub fn new(command: std::sync::Arc<String>, args: std::sync::Arc<Vec<String>>) -> Self {
        Self { command, args }
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl AgentSlotResolver for FixedAgentSlot {
    async fn resolve(&self, _slot: Option<&str>) -> easyvibe_ai_agent::agent_conf::ResolvedAgent {
        easyvibe_ai_agent::agent_conf::ResolvedAgent {
            command: (*self.command).clone(),
            args: (*self.args).clone(),
            agent_type: "claude".into(),
            source: "fixed".into(),
            preset: "fixed".into(),
        }
    }
}
