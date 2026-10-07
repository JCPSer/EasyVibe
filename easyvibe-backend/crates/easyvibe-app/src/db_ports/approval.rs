//! 审批域端口适配（c-arch-13 R1）。

use easyvibe_common::ApiError;

/// 审批端口（返回具体行类型不构成类型穿透——service 侧不写名，且 JSON 形状零改动）。
#[async_trait::async_trait]
pub(crate) trait ApprovalPort {
    async fn list_by_task(&self, task_id: &str) -> Result<Vec<easyvibe_db::ApprovalRow>, ApiError>;
}

#[async_trait::async_trait]
impl ApprovalPort for easyvibe_db::SqliteApprovalRepository {
    async fn list_by_task(&self, task_id: &str) -> Result<Vec<easyvibe_db::ApprovalRow>, ApiError> {
        easyvibe_db::ApprovalRepository::list_by_task(self, task_id).await
    }
}
