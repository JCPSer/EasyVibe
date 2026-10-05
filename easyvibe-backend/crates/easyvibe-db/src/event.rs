//! event 聚合域：使用证据事件写入与计数聚合。
use easyvibe_common::ApiError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::core::{db_err, now_ms};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct EventCountRow {
    pub name: String,
    pub count: i64,
}

pub trait EventRepository: Send + Sync {
    fn record(&self, repo: &str, name: &str, payload: &str) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    /// 门控读数：按事件名计数（L3 三道门、Harness 验证指标的数据源）
    fn summary(&self, repo: &str) -> impl std::future::Future<Output = Result<Vec<EventCountRow>, ApiError>> + Send;
}

pub struct SqliteEventRepository {
    pool: SqlitePool,
}

impl SqliteEventRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl EventRepository for SqliteEventRepository {
    async fn record(&self, repo: &str, name: &str, payload: &str) -> Result<(), ApiError> {
        sqlx::query("INSERT INTO events (repo, name, payload, created_at) VALUES (?, ?, ?, ?)")
            .bind(repo).bind(name).bind(payload)
            .bind(now_ms())
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn summary(&self, repo: &str) -> Result<Vec<EventCountRow>, ApiError> {
        let rows = sqlx::query_as::<_, EventCountRow>("SELECT name, COUNT(*) AS count FROM events WHERE repo = ? GROUP BY name ORDER BY count DESC")
            .bind(repo)
            .fetch_all(&self.pool).await.map_err(db_err)?;
        Ok(rows)
    }
}
