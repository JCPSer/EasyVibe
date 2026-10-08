//! 任务端口适配（c-arch-16 R7 从 task_engine.rs 拆分；单一端口域 = 任务仓储）。
//!
//! 原 22 处直连横跨 4 个端口域（TaskStore/ApprovalStore/SessionAttribution/AgentSlotResolver），
//! 按 R7 纯搬家为 4 个单域文件；本文件只承载 `TaskStore → TaskRepository/TaskRow/SqliteTaskRepository`（14 处）。

use crate::task_exec::ports::{TaskRecord, TaskStore};
use easyvibe_common::ApiError;

/// 持久层任务行 → 切片内本地 DTO（字段逐一同名搬运，22 字段顺序与类型全等）。
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
