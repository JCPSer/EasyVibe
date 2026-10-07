//! task-engine 切片端口适配（c-arch-13 R1 自 db_ports.rs 纯搬运；单一端口域 = 任务执行引擎）。

use crate::task_exec::ports::{AgentSlotResolver, ApprovalRecord, ApprovalStore, SessionAttribution, TaskRecord, TaskStore};
use easyvibe_common::ApiError;
use std::sync::Arc;

/// 持久层任务行 → 切片内本地 DTO（字段逐一同名搬运）。
fn into_record(r: easyvibe_db::TaskRow) -> TaskRecord {
    TaskRecord {
        id: r.id,
        repo: r.repo,
        title: r.title,
        description: r.description,
        modules: r.modules,
        acceptance: r.acceptance,
        source: r.source,
        context: r.context,
        status: r.status,
        trust: r.trust,
        error: r.error,
        session_id: r.session_id,
        gate: r.gate,
        prompt_tokens: r.prompt_tokens,
        completion_tokens: r.completion_tokens,
        result: r.result,
        base_head: r.base_head,
        created_at: r.created_at,
        updated_at: r.updated_at,
        conversation_id: r.conversation_id,
        origin_task_id: r.origin_task_id,
        successor_task_id: r.successor_task_id,
    }
}

/// 任务仓储适配（全方法透传；`TaskRow → TaskRecord` 仅一次字段搬运）。
#[async_trait::async_trait]
impl TaskStore for easyvibe_db::SqliteTaskRepository {
    async fn get(&self, id: &str) -> Result<Option<TaskRecord>, ApiError> {
        Ok(easyvibe_db::TaskRepository::get(self, id).await?.map(into_record))
    }

    async fn list(&self, repo: &str, limit: i64) -> Result<Vec<TaskRecord>, ApiError> {
        Ok(easyvibe_db::TaskRepository::list(self, repo, limit).await?.into_iter().map(into_record).collect())
    }

    async fn update_status(&self, id: &str, status: &str, error: Option<&str>) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::update_status(self, id, status, error).await
    }

    async fn set_gate(&self, id: &str, gate: Option<&str>) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::set_gate(self, id, gate).await
    }

    async fn try_advance_gate(
        &self,
        id: &str,
        expected_gate: Option<&str>,
        new_gate: Option<&str>,
        new_status: Option<&str>,
    ) -> Result<u64, ApiError> {
        easyvibe_db::TaskRepository::try_advance_gate(self, id, expected_gate, new_gate, new_status).await
    }

    async fn set_context(&self, id: &str, context: &str) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::set_context(self, id, context).await
    }

    async fn reset_for_retry(&self, id: &str) -> Result<u64, ApiError> {
        easyvibe_db::TaskRepository::reset_for_retry(self, id).await
    }

    async fn reset_for_remediate(&self, id: &str) -> Result<u64, ApiError> {
        easyvibe_db::TaskRepository::reset_for_remediate(self, id).await
    }

    async fn try_rewind(&self, id: &str, target_gate: &str) -> Result<u64, ApiError> {
        easyvibe_db::TaskRepository::try_rewind(self, id, target_gate).await
    }

    async fn set_session(&self, id: &str, session_id: &str) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::set_session(self, id, session_id).await
    }

    async fn set_base_head(&self, id: &str, head: &str) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::set_base_head(self, id, head).await
    }

    async fn set_result(&self, id: &str, result_json: &str) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::set_result(self, id, result_json).await
    }
}

/// 审批仓储适配：id 与 decided_at 在此补齐（原状态机内的构造职责上提组合根）。
#[async_trait::async_trait]
impl ApprovalStore for easyvibe_db::SqliteApprovalRepository {
    async fn record(&self, approval: &ApprovalRecord) -> Result<(), ApiError> {
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let row = easyvibe_db::ApprovalRow {
            id: format!("ap-{}-{}-{}", approval.task_id, approval.gate, now_ms),
            task_id: approval.task_id.clone(),
            gate: approval.gate.clone(),
            decision: approval.decision.clone(),
            note: approval.note.clone(),
            decided_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                .to_string(),
        };
        easyvibe_db::ApprovalRepository::record(self, &row).await
    }

    async fn list_by_task(&self, task_id: &str) -> Result<Vec<ApprovalRecord>, ApiError> {
        Ok(easyvibe_db::ApprovalRepository::list_by_task(self, task_id)
            .await?
            .into_iter()
            .map(|a| ApprovalRecord {
                task_id: a.task_id,
                gate: a.gate,
                decision: a.decision,
                note: a.note,
            })
            .collect())
    }
}

/// 会话归属适配（tasks.modules 首个模块 → 会话行归因）。
#[async_trait::async_trait]
impl SessionAttribution for easyvibe_db::AgentSessionRepo {
    async fn set_module_id(&self, session_id: &str, module_id: &str) -> Result<(), ApiError> {
        easyvibe_db::AgentSessionRepo::set_module_id(self, session_id, module_id).await
    }
}

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
