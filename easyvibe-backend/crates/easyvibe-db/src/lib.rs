//! 域 2（IDE 自有状态）的 SQLite 数据层：迁移 + Repository trait + sqlx 实现。
//! 形状照抄 AionCore：Service 只依赖 trait；测试用内存库。
use easyvibe_common::ApiError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

pub use sqlx;

// ---------- 模型 ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatrolRunRow {
    pub id: String,
    pub repo: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: String, // running / succeeded / failed
    pub model: Option<String>,
    pub arch_score: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleHealthRow {
    pub run_id: String,
    pub module_id: String,
    pub name: Option<String>,
    pub score: i64,
    pub coupling: Option<String>,
    pub complexity: Option<String>,
    pub churn: Option<String>,
    pub decay_flags: String, // JSON array
    pub review_note: Option<String>,
    pub concerns: String, // JSON array
}

// ---------- 数据库 ----------

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// 文件库（生产）
    pub async fn connect_file(path: &str) -> Result<Self, ApiError> {
        let url = format!("sqlite://{path}?mode=rwc");
        let pool = SqlitePool::connect(&url).await.map_err(db_err)?;
        Self::migrate(&pool).await?;
        Ok(Self { pool })
    }

    /// 内存库（测试）：单连接保证内存数据一致性（审查：原默认 10 连接会让内存库裂库）
    pub async fn connect_memory() -> Result<Self, ApiError> {
        let pool = sqlx::pool::PoolOptions::<sqlx::Sqlite>::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .map_err(db_err)?;
        Self::migrate(&pool).await?;
        Ok(Self { pool })
    }

    async fn migrate(pool: &SqlitePool) -> Result<(), ApiError> {
        sqlx::migrate!("./migrations")
            .run(pool)
            .await
            .map_err(|e| ApiError::Internal(format!("migrate: {e}")))
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

fn db_err(e: sqlx::Error) -> ApiError {
    ApiError::Internal(format!("db: {e}"))
}

// ---------- Repository ----------

#[derive(Debug, Clone)]
pub struct NewPatrolRun {
    pub id: String,
    pub repo: String,
    pub started_at: String,
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct FinishPatrolRun {
    pub id: String,
    pub finished_at: String,
    pub status: String, // succeeded / failed
    pub arch_score: Option<i64>,
    pub error: Option<String>,
}

pub trait HealthRepository: Send + Sync {
    fn create_run(&self, run: &NewPatrolRun) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    fn finish_run(&self, fin: &FinishPatrolRun) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    fn insert_module_health(
        &self,
        row: &ModuleHealthRow,
    ) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    fn list_runs(
        &self,
        repo: &str,
        limit: i64,
    ) -> impl std::future::Future<Output = Result<Vec<PatrolRunRow>, ApiError>> + Send;
    fn list_module_history(
        &self,
        repo: &str,
        module_id: &str,
        limit: i64,
    ) -> impl std::future::Future<Output = Result<Vec<ModuleHealthRow>, ApiError>> + Send;
}

pub struct SqliteHealthRepository {
    pool: SqlitePool,
}

impl SqliteHealthRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl HealthRepository for SqliteHealthRepository {
    async fn create_run(&self, run: &NewPatrolRun) -> Result<(), ApiError> {
        sqlx::query(
            "INSERT INTO patrol_runs (id, repo, started_at, status, model) VALUES (?, ?, ?, 'running', ?)",
        )
        .bind(&run.id)
        .bind(&run.repo)
        .bind(&run.started_at)
        .bind(&run.model)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn finish_run(&self, fin: &FinishPatrolRun) -> Result<(), ApiError> {
        sqlx::query(
            "UPDATE patrol_runs SET finished_at = ?, status = ?, arch_score = ?, error = ? WHERE id = ?",
        )
        .bind(&fin.finished_at)
        .bind(&fin.status)
        .bind(fin.arch_score)
        .bind(&fin.error)
        .bind(&fin.id)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn insert_module_health(&self, row: &ModuleHealthRow) -> Result<(), ApiError> {
        sqlx::query(
            "INSERT OR REPLACE INTO module_health_history
             (run_id, module_id, name, score, coupling, complexity, churn, decay_flags, review_note, concerns)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.run_id)
        .bind(&row.module_id)
        .bind(&row.name)
        .bind(row.score)
        .bind(&row.coupling)
        .bind(&row.complexity)
        .bind(&row.churn)
        .bind(&row.decay_flags)
        .bind(&row.review_note)
        .bind(&row.concerns)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn list_runs(&self, repo: &str, limit: i64) -> Result<Vec<PatrolRunRow>, ApiError> {
        let rows = sqlx::query_as::<_, PatrolRunRowSql>(
            "SELECT id, repo, started_at, finished_at, status, model, arch_score, error
             FROM patrol_runs WHERE repo = ? ORDER BY started_at DESC LIMIT ?",
        )
        .bind(repo)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn list_module_history(&self, repo: &str, module_id: &str, limit: i64) -> Result<Vec<ModuleHealthRow>, ApiError> {
        let rows = sqlx::query_as::<_, ModuleHealthRowSql>(
            "SELECT h.* FROM module_health_history h
             JOIN patrol_runs r ON r.id = h.run_id
             WHERE r.repo = ? AND h.module_id = ?
             ORDER BY r.started_at DESC LIMIT ?",
        )
        .bind(repo)
        .bind(module_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
}

// sqlx 行映射（camelCase 手动桥接）
#[derive(sqlx::FromRow)]
struct PatrolRunRowSql {
    id: String,
    repo: String,
    started_at: String,
    finished_at: Option<String>,
    status: String,
    model: Option<String>,
    arch_score: Option<i64>,
    error: Option<String>,
}

impl From<PatrolRunRowSql> for PatrolRunRow {
    fn from(r: PatrolRunRowSql) -> Self {
        Self {
            id: r.id,
            repo: r.repo,
            started_at: r.started_at,
            finished_at: r.finished_at,
            status: r.status,
            model: r.model,
            arch_score: r.arch_score,
            error: r.error,
        }
    }
}

#[derive(sqlx::FromRow)]
struct ModuleHealthRowSql {
    run_id: String,
    module_id: String,
    name: Option<String>,
    score: i64,
    coupling: Option<String>,
    complexity: Option<String>,
    churn: Option<String>,
    decay_flags: String,
    review_note: Option<String>,
    concerns: String,
}

impl From<ModuleHealthRowSql> for ModuleHealthRow {
    fn from(r: ModuleHealthRowSql) -> Self {
        Self {
            run_id: r.run_id,
            module_id: r.module_id,
            name: r.name,
            score: r.score,
            coupling: r.coupling,
            complexity: r.complexity,
            churn: r.churn,
            decay_flags: r.decay_flags,
            review_note: r.review_note,
            concerns: r.concerns,
        }
    }
}

// ---------- 配置体系（backend-design §10） ----------

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

    fn now() -> String {
        format!("{:?}", std::time::SystemTime::now())
    }

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

    #[tokio::test]
    async fn run_and_module_history_roundtrip() {
        let db = Database::connect_memory().await.unwrap();
        let repo = SqliteHealthRepository::new(db.pool().clone());

        repo.create_run(&NewPatrolRun {
            id: "run-1".into(),
            repo: "demo".into(),
            started_at: now(),
            model: "stub".into(),
        })
        .await
        .unwrap();

        repo.insert_module_health(&ModuleHealthRow {
            run_id: "run-1".into(),
            module_id: "order-service".into(),
            name: Some("订单服务".into()),
            score: 62,
            coupling: Some("high".into()),
            complexity: Some("high".into()),
            churn: Some("medium".into()),
            decay_flags: "[\"god_module\"]".into(),
            review_note: Some("拆分".into()),
            concerns: "[]".into(),
        })
        .await
        .unwrap();

        repo.finish_run(&FinishPatrolRun {
            id: "run-1".into(),
            finished_at: now(),
            status: "succeeded".into(),
            arch_score: Some(58),
            error: None,
        })
        .await
        .unwrap();

        let runs = repo.list_runs("demo", 10).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "succeeded");
        assert_eq!(runs[0].arch_score, Some(58));

        let history = repo.list_module_history("demo", "order-service", 10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].score, 62);
    }
}
