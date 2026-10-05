//! agent-observability 聚合域：session_outputs 输出行（回放/补拉/裁剪）。
use easyvibe_common::ApiError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::core::db_err;

/// M2：会话输出行（回放/补拉的数据单元；seq 每会话单调，(session_id, seq) 主键幂等）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOutputRow {
    pub session_id: String,
    pub seq: i64,
    pub ts: String,
    pub stream: String,
    pub line: String,
}

#[derive(sqlx::FromRow)]
struct SessionOutputRowSql {
    session_id: String,
    seq: i64,
    ts: String,
    stream: String,
    line: String,
}

impl From<SessionOutputRowSql> for SessionOutputRow {
    fn from(r: SessionOutputRowSql) -> Self {
        Self { session_id: r.session_id, seq: r.seq, ts: r.ts, stream: r.stream, line: r.line }
    }
}

pub struct SessionOutputRepo {
    pool: SqlitePool,
}

impl SessionOutputRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// 批量追加（落盘桥 300ms 节流攒批后调用；INSERT OR IGNORE 幂等——重放/补拉不重复）
    pub async fn append_batch(&self, rows: &[SessionOutputRow]) -> Result<(), ApiError> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        for r in rows {
            sqlx::query("INSERT OR IGNORE INTO session_outputs (session_id, seq, ts, stream, line) VALUES (?, ?, ?, ?, ?)")
                .bind(&r.session_id)
                .bind(r.seq)
                .bind(&r.ts)
                .bind(&r.stream)
                .bind(&r.line)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
        }
        tx.commit().await.map_err(db_err)?;
        Ok(())
    }

    /// 回放/补拉：after_seq 之后的行（升序）
    pub async fn fetch_after(&self, session_id: &str, after_seq: i64, limit: i64) -> Result<Vec<SessionOutputRow>, ApiError> {
        let rows = sqlx::query_as::<_, SessionOutputRowSql>(
            "SELECT session_id, seq, ts, stream, line FROM session_outputs WHERE session_id = ? AND seq > ? ORDER BY seq LIMIT ?",
        )
        .bind(session_id)
        .bind(after_seq)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// 容量策略：每会话只留最近 keep 行（终态 finalize 时调用）
    pub async fn prune_session(&self, session_id: &str, keep: i64) -> Result<u64, ApiError> {
        let res = sqlx::query(
            "DELETE FROM session_outputs WHERE session_id = ? AND seq NOT IN (
                SELECT seq FROM session_outputs WHERE session_id = ? ORDER BY seq DESC LIMIT ?)",
        )
        .bind(session_id)
        .bind(session_id)
        .bind(keep)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    #[tokio::test]
    async fn session_outputs_append_fetch_prune() {
        let db = Database::connect_memory().await.unwrap();
        let repo = SessionOutputRepo::new(db.pool().clone());
        let mk = |seq: i64| SessionOutputRow {
            session_id: "s1".into(), seq, ts: "2026-10-05T08:00:00Z".into(),
            stream: if seq % 2 == 0 { "stderr".into() } else { "stdout".into() },
            line: format!("line-{seq}"),
        };
        repo.append_batch(&(1..=5).map(mk).collect::<Vec<_>>()).await.unwrap();
        repo.append_batch(&[mk(3)]).await.unwrap(); // 幂等重放
        let rows = repo.fetch_after("s1", 0, 100).await.unwrap();
        assert_eq!(rows.len(), 5, "重放不得产生重复行");
        assert_eq!(rows[0].line, "line-1");
        assert_eq!(rows[1].stream, "stderr");
        let tail = repo.fetch_after("s1", 3, 10).await.unwrap();
        assert_eq!(tail.len(), 2, "afterSeq 补拉语义");
        let deleted = repo.prune_session("s1", 2).await.unwrap();
        assert_eq!(deleted, 3);
        assert_eq!(repo.fetch_after("s1", 0, 100).await.unwrap().len(), 2);
    }
}
