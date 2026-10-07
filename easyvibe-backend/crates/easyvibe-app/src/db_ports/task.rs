//! 任务域端口适配（c-arch-13 R1）。

use super::dto::{TaskDraft, TaskInfo};
use easyvibe_common::ApiError;

#[async_trait::async_trait]
pub(crate) trait TaskPort {
    async fn get(&self, id: &str) -> Result<Option<TaskInfo>, ApiError>;
    async fn list(&self, repo: &str, limit: i64) -> Result<Vec<TaskInfo>, ApiError>;
    async fn list_by_conversation(&self, conversation_id: &str) -> Result<Vec<TaskInfo>, ApiError>;
    async fn create(&self, t: &TaskDraft) -> Result<(), ApiError>;
    async fn set_successor(&self, id: &str, successor: &str) -> Result<(), ApiError>;
    async fn delete(&self, id: &str) -> Result<(), ApiError>;
}

fn into_task_info(r: easyvibe_db::TaskRow) -> TaskInfo {
    TaskInfo {
        id: r.id, title: r.title, description: r.description, modules: r.modules,
        acceptance: r.acceptance, source: r.source, status: r.status, trust: r.trust, error: r.error,
        session_id: r.session_id, gate: r.gate, result: r.result, created_at: r.created_at,
        updated_at: r.updated_at, conversation_id: r.conversation_id, origin_task_id: r.origin_task_id,
        successor_task_id: r.successor_task_id,
    }
}

#[async_trait::async_trait]
impl TaskPort for easyvibe_db::SqliteTaskRepository {
    async fn get(&self, id: &str) -> Result<Option<TaskInfo>, ApiError> {
        Ok(easyvibe_db::TaskRepository::get(self, id).await?.map(into_task_info))
    }
    async fn list(&self, repo: &str, limit: i64) -> Result<Vec<TaskInfo>, ApiError> {
        Ok(easyvibe_db::TaskRepository::list(self, repo, limit).await?.into_iter().map(into_task_info).collect())
    }
    async fn list_by_conversation(&self, conversation_id: &str) -> Result<Vec<TaskInfo>, ApiError> {
        Ok(easyvibe_db::TaskRepository::list_by_conversation(self, conversation_id).await?.into_iter().map(into_task_info).collect())
    }
    async fn create(&self, t: &TaskDraft) -> Result<(), ApiError> {
        let row = easyvibe_db::TaskRow {
            id: t.id.clone(), repo: t.repo.clone(), title: t.title.clone(), description: t.description.clone(),
            modules: t.modules.clone(), acceptance: t.acceptance.clone(), source: t.source.clone(),
            context: t.context.clone(), status: t.status.clone(), trust: t.trust.clone(), error: t.error.clone(),
            session_id: t.session_id.clone(), gate: t.gate.clone(), prompt_tokens: t.prompt_tokens,
            completion_tokens: t.completion_tokens, result: t.result.clone(), base_head: t.base_head.clone(),
            created_at: t.created_at.clone(), updated_at: t.updated_at.clone(),
            conversation_id: t.conversation_id.clone(), origin_task_id: t.origin_task_id.clone(),
            successor_task_id: t.successor_task_id.clone(),
        };
        easyvibe_db::TaskRepository::create(self, &row).await
    }
    async fn set_successor(&self, id: &str, successor: &str) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::set_successor(self, id, successor).await
    }
    async fn delete(&self, id: &str) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::delete(self, id).await
    }
}
