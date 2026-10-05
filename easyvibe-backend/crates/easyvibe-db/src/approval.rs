//! approval 聚合域：审批留痕写入与按任务查询。
use easyvibe_common::ApiError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::core::db_err;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRow {
    pub id: String,
    pub task_id: String,
    pub gate: String,
    pub decision: String, // approved / rejected / skipped
    pub note: Option<String>,
    pub decided_at: String,
}

pub trait ApprovalRepository: Send + Sync {
    fn record(&self, a: &ApprovalRow) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    fn list_by_task(&self, task_id: &str) -> impl std::future::Future<Output = Result<Vec<ApprovalRow>, ApiError>> + Send;
}

pub struct SqliteApprovalRepository {
    pool: SqlitePool,
}

impl SqliteApprovalRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct ApprovalRowSql {
    id: String, task_id: String, gate: String, decision: String, note: Option<String>, decided_at: String,
}

impl From<ApprovalRowSql> for ApprovalRow {
    fn from(r: ApprovalRowSql) -> Self {
        Self { id: r.id, task_id: r.task_id, gate: r.gate, decision: r.decision, note: r.note, decided_at: r.decided_at }
    }
}

impl ApprovalRepository for SqliteApprovalRepository {
    async fn record(&self, a: &ApprovalRow) -> Result<(), ApiError> {
        sqlx::query("INSERT INTO approvals (id, task_id, gate, decision, note, decided_at) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(&a.id).bind(&a.task_id).bind(&a.gate).bind(&a.decision).bind(&a.note).bind(&a.decided_at)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn list_by_task(&self, task_id: &str) -> Result<Vec<ApprovalRow>, ApiError> {
        let rows = sqlx::query_as::<_, ApprovalRowSql>(
            "SELECT id, task_id, gate, decision, note, decided_at FROM approvals WHERE task_id = ? ORDER BY decided_at",
        )
        .bind(task_id)
        .fetch_all(&self.pool).await.map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
}
