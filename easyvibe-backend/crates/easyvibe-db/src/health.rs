//! health 聚合域：巡检 run 与模块健康历史（row 定义 + 仓储 trait + sqlx 实现）。
use easyvibe_common::ApiError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::core::db_err;

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
    pub prompt_tokens: Option<i64>,     // M3-5：token 用量记录（§10 #4 第一步）
    pub completion_tokens: Option<i64>,
    /// 2026-10-05 问题项新旧对照（JSON：{fixed:[{id,finding}],new:[…],persisted:n, moduleGone:[…]}）；
    /// 仅 succeeded 巡检写入，失败/进行时为 NULL
    pub concerns_diff: Option<String>,
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
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    /// 问题项新旧对照 JSON（仅 succeeded 写入；评审#Q1：失败存 NULL 防误导）
    pub concerns_diff: Option<String>,
}

/// M4-3 健康看板：单次巡检的模块分聚合（趋势图"模块平均"线 + 巡检记录表的数据面）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunModuleAvg {
    pub run_id: String,
    pub module_avg: i64,
    pub module_count: i64,
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
    /// M4-3：近 limit 次巡检各自的模块平均分（JOIN 聚合，一次查询；无模块行的 run 不出现在结果里）
    fn list_run_averages(
        &self,
        repo: &str,
        limit: i64,
    ) -> impl std::future::Future<Output = Result<Vec<RunModuleAvg>, ApiError>> + Send;
    /// M4-3：最近一次成功巡检的模块明细（健康看板"最差模块排行"；按分数升序）
    fn list_latest_run_modules(
        &self,
        repo: &str,
    ) -> impl std::future::Future<Output = Result<Vec<ModuleHealthRow>, ApiError>> + Send;
    /// 历史清理（2026-10-03 重审 P1）：只保留最近 keep 次已终态巡检
    /// （running 的不动——它是正在发生的事实），连带删 module_health_history。
    /// 返回删除的 run 数。
    fn prune_runs(&self, repo: &str, keep: i64) -> impl std::future::Future<Output = Result<u64, ApiError>> + Send;
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
            "UPDATE patrol_runs SET finished_at = ?, status = ?, arch_score = ?, error = ?, prompt_tokens = ?, completion_tokens = ?, concerns_diff = ? WHERE id = ?",
        )
        .bind(&fin.finished_at)
        .bind(&fin.status)
        .bind(fin.arch_score)
        .bind(&fin.error)
        .bind(fin.prompt_tokens)
        .bind(fin.completion_tokens)
        .bind(&fin.concerns_diff)
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
            "SELECT id, repo, started_at, finished_at, status, model, arch_score, error, prompt_tokens, completion_tokens, concerns_diff
             FROM patrol_runs WHERE repo = ? ORDER BY started_at DESC LIMIT ?",
        )
        .bind(repo)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn prune_runs(&self, repo: &str, keep: i64) -> Result<u64, ApiError> {
        // 先删明细再删主表（子表无外键级联，手动序）；running 永不进删除集——
        // "正在发生的事实"不能被清理动作抹掉
        sqlx::query(
            "DELETE FROM module_health_history WHERE run_id IN (
                SELECT id FROM patrol_runs
                WHERE repo = ? AND status != 'running'
                ORDER BY started_at DESC LIMIT -1 OFFSET ?)",
        )
        .bind(repo)
        .bind(keep.max(0))
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        let r = sqlx::query(
            "DELETE FROM patrol_runs
             WHERE repo = ? AND status != 'running' AND id NOT IN (
                SELECT id FROM patrol_runs
                WHERE repo = ? AND status != 'running'
                ORDER BY started_at DESC LIMIT ?)",
        )
        .bind(repo)
        .bind(repo)
        .bind(keep.max(0))
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(r.rows_affected())
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

    async fn list_run_averages(&self, repo: &str, limit: i64) -> Result<Vec<RunModuleAvg>, ApiError> {
        let rows = sqlx::query_as::<_, (String, i64, i64)>(
            "SELECT r.id, CAST(ROUND(AVG(h.score)) AS INTEGER), COUNT(*)
             FROM patrol_runs r JOIN module_health_history h ON h.run_id = r.id
             WHERE r.repo = ?
             GROUP BY r.id ORDER BY r.started_at DESC LIMIT ?",
        )
        .bind(repo)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(run_id, module_avg, module_count)| RunModuleAvg { run_id, module_avg, module_count })
            .collect())
    }

    async fn list_latest_run_modules(&self, repo: &str) -> Result<Vec<ModuleHealthRow>, ApiError> {
        let rows = sqlx::query_as::<_, ModuleHealthRowSql>(
            "SELECT h.* FROM module_health_history h
             WHERE h.run_id = (
                 SELECT id FROM patrol_runs WHERE repo = ? AND status = 'succeeded'
                 ORDER BY started_at DESC LIMIT 1
             )
             ORDER BY h.score ASC",
        )
        .bind(repo)
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
    concerns_diff: Option<String>,
    arch_score: Option<i64>,
    error: Option<String>,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
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
            prompt_tokens: r.prompt_tokens,
            completion_tokens: r.completion_tokens,
            concerns_diff: r.concerns_diff,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn now() -> String {
        format!("{:?}", std::time::SystemTime::now())
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
            prompt_tokens: Some(100),
            concerns_diff: None,
            completion_tokens: Some(50),
        })
        .await
        .unwrap();

        let runs = repo.list_runs("demo", 10).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "succeeded");
        assert_eq!(runs[0].arch_score, Some(58));
        assert_eq!(runs[0].prompt_tokens, Some(100), "token 用量随 run 落库（§10 #4）");

        let history = repo.list_module_history("demo", "order-service", 10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].score, 62);
    }

    #[tokio::test]
    async fn run_averages_and_latest_modules() {
        // M4-3 健康看板数据面：每次巡检的模块平均 + 最近一次成功巡检的模块排行
        let db = Database::connect_memory().await.unwrap();
        let repo = SqliteHealthRepository::new(db.pool().clone());

        let mk_run = |id: &str, started: &str, status: &str, arch: Option<i64>| {
            let repo = SqliteHealthRepository::new(db.pool().clone());
            let id = id.to_string();
            let started = started.to_string();
            let status = status.to_string();
            async move {
                repo.create_run(&NewPatrolRun { id: id.clone(), repo: "demo".into(), started_at: started, model: "stub".into() }).await.unwrap();
                repo.finish_run(&FinishPatrolRun { id, finished_at: now(), status, arch_score: arch, error: None, prompt_tokens: None, completion_tokens: None, concerns_diff: None }).await.unwrap();
            }
        };
        let mk_module = |run_id: &str, module_id: &str, score: i64| {
            ModuleHealthRow {
                run_id: run_id.into(), module_id: module_id.into(), name: Some(module_id.into()),
                score, coupling: None, complexity: None, churn: None,
                decay_flags: "[]".into(), review_note: None, concerns: "[]".into(),
            }
        };
        let insert_module = |row: &ModuleHealthRow| {
            let repo = SqliteHealthRepository::new(db.pool().clone());
            let row = row.clone();
            async move { repo.insert_module_health(&row).await.unwrap() }
        };

        mk_run("run-old", "2026-09-01T09:00:00", "succeeded", Some(60)).await;
        mk_run("run-mid", "2026-09-15T09:00:00", "succeeded", Some(65)).await;
        mk_run("run-latest", "2026-09-30T09:00:00", "succeeded", Some(72)).await;
        mk_run("run-failed", "2026-09-30T21:00:00", "failed", None).await;

        insert_module(&mk_module("run-old", "m1", 60)).await;
        insert_module(&mk_module("run-old", "m2", 80)).await;
        insert_module(&mk_module("run-mid", "m1", 65)).await;
        insert_module(&mk_module("run-mid", "m2", 75)).await;
        insert_module(&mk_module("run-latest", "m1", 62)).await;
        insert_module(&mk_module("run-latest", "m2", 66)).await;
        insert_module(&mk_module("run-latest", "m3", 90)).await;

        let avgs = repo.list_run_averages("demo", 10).await.unwrap();
        assert_eq!(avgs.len(), 3, "failed 且无模块行的 run 不参与平均");
        assert_eq!(avgs[0].run_id, "run-latest", "按 started_at 倒序");
        assert_eq!(avgs[0].module_avg, 73, "(62+66+90)/3 = 72.67 → ROUND = 73");
        assert_eq!(avgs[0].module_count, 3);
        assert_eq!(avgs[1].module_avg, 70);
        assert_eq!(avgs[2].module_avg, 70, "(60+80)/2");

        let latest = repo.list_latest_run_modules("demo").await.unwrap();
        assert_eq!(latest.len(), 3);
        assert_eq!(latest[0].module_id, "m1", "按分数升序（最差在前）");
        assert_eq!(latest[0].score, 62);
        assert_eq!(latest[2].score, 90);

        // 边界：从未成功巡检 → 空表而非报错
        assert!(repo.list_latest_run_modules("nope").await.unwrap().is_empty());
    }
}
