//! settings 聚合域：scope+key 配置读写（含加密信封桥接）。
use easyvibe_common::ApiError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::core::db_err;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingRow {
    pub scope: String,
    pub key: String,
    pub value: String, // JSON；encrypted=1 时为加密信封 JSON
    pub encrypted: bool,
    pub updated_at: String,
}

pub trait SettingsRepository: Send + Sync {
    fn get(&self, scope: &str, key: &str) -> impl std::future::Future<Output = Result<Option<SettingRow>, ApiError>> + Send;
    fn set(&self, row: &SettingRow) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    fn list(&self, scope: &str) -> impl std::future::Future<Output = Result<Vec<SettingRow>, ApiError>> + Send;
    fn delete(&self, scope: &str, key: &str) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
}

pub struct SqliteSettingsRepository {
    pool: SqlitePool,
}

impl SqliteSettingsRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl SettingsRepository for SqliteSettingsRepository {
    async fn get(&self, scope: &str, key: &str) -> Result<Option<SettingRow>, ApiError> {
        let row = sqlx::query_as::<_, SettingRowSql>(
            "SELECT scope, key, value, encrypted, updated_at FROM settings WHERE scope = ? AND key = ?",
        )
        .bind(scope)
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.map(Into::into))
    }

    async fn set(&self, row: &SettingRow) -> Result<(), ApiError> {
        sqlx::query(
            "INSERT INTO settings (scope, key, value, encrypted, updated_at) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (scope, key) DO UPDATE SET value = excluded.value, encrypted = excluded.encrypted, updated_at = excluded.updated_at",
        )
        .bind(&row.scope)
        .bind(&row.key)
        .bind(&row.value)
        .bind(row.encrypted as i64)
        .bind(&row.updated_at)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn list(&self, scope: &str) -> Result<Vec<SettingRow>, ApiError> {
        let rows = sqlx::query_as::<_, SettingRowSql>(
            "SELECT scope, key, value, encrypted, updated_at FROM settings WHERE scope = ? ORDER BY key",
        )
        .bind(scope)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn delete(&self, scope: &str, key: &str) -> Result<(), ApiError> {
        sqlx::query("DELETE FROM settings WHERE scope = ? AND key = ?")
            .bind(scope)
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct SettingRowSql {
    scope: String,
    key: String,
    value: String,
    encrypted: i64,
    updated_at: String,
}

impl From<SettingRowSql> for SettingRow {
    fn from(r: SettingRowSql) -> Self {
        Self { scope: r.scope, key: r.key, value: r.value, encrypted: r.encrypted != 0, updated_at: r.updated_at }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    #[tokio::test]
    async fn settings_roundtrip() {
        let db = Database::connect_memory().await.unwrap();
        let repo = SqliteSettingsRepository::new(db.pool().clone());
        repo.set(&SettingRow {
            scope: "global".into(),
            key: "slot.patrol".into(),
            value: "\"default\"".into(),
            encrypted: false,
            updated_at: "t1".into(),
        })
        .await
        .unwrap();
        let got = repo.get("global", "slot.patrol").await.unwrap().unwrap();
        assert_eq!(got.value, "\"default\"");
        assert!(!got.encrypted);
        repo.set(&SettingRow {
            scope: "hover-client".into(),
            key: "slot.patrol".into(),
            value: "envelope".into(),
            encrypted: true,
            updated_at: "t2".into(),
        })
        .await
        .unwrap();
        // 仓库覆盖与全局共存（生效解析的数据源）
        assert_eq!(repo.list("hover-client").await.unwrap().len(), 1);
        assert_eq!(repo.list("global").await.unwrap().len(), 1);
        repo.delete("hover-client", "slot.patrol").await.unwrap();
        assert!(repo.get("hover-client", "slot.patrol").await.unwrap().is_none());
    }
}
