//! 审批端口适配（c-arch-16 R7 自 task_engine.rs 纯搬家；单一端口域 = 审批仓储）。
//!
//! 零语义改动：`ApprovalRow` 的 id（`ap-{task_id}-{gate}-{now_ms}`）与 decided_at
//! （秒级字符串）生成职责原样保留在此。

use crate::task_exec::ports::{ApprovalRecord, ApprovalStore};
use easyvibe_common::ApiError;

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
