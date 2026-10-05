//! agent-observability 聚合域：agent_sessions 生命周期与 usage 聚合。
use easyvibe_common::ApiError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::core::db_err;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionRow {
    pub id: String,
    pub repo: String,
    pub kind: String,
    pub label: Option<String>,
    pub cli: Option<String>,
    pub model: Option<String>,
    pub parent_session_id: Option<String>,
    pub task_id: Option<String>,
    pub module_id: Option<String>,
    pub started_at: String,
    pub terminal_at: Option<String>,
    pub status: String,
    pub exit_code: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<i64>,
    pub turns: Option<i64>,
    pub usage_source: Option<String>,
}

#[derive(sqlx::FromRow)]
struct AgentSessionRowSql {
    id: String,
    repo: String,
    kind: String,
    label: Option<String>,
    cli: Option<String>,
    model: Option<String>,
    parent_session_id: Option<String>,
    task_id: Option<String>,
    module_id: Option<String>,
    started_at: String,
    terminal_at: Option<String>,
    status: String,
    exit_code: Option<i64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_read_tokens: Option<i64>,
    cache_write_tokens: Option<i64>,
    cost_usd: Option<f64>,
    duration_ms: Option<i64>,
    turns: Option<i64>,
    usage_source: Option<String>,
}

impl From<AgentSessionRowSql> for AgentSessionRow {
    fn from(r: AgentSessionRowSql) -> Self {
        Self {
            id: r.id, repo: r.repo, kind: r.kind, label: r.label, cli: r.cli, model: r.model,
            parent_session_id: r.parent_session_id, task_id: r.task_id, module_id: r.module_id,
            started_at: r.started_at, terminal_at: r.terminal_at, status: r.status,
            exit_code: r.exit_code, input_tokens: r.input_tokens, output_tokens: r.output_tokens,
            cache_read_tokens: r.cache_read_tokens, cache_write_tokens: r.cache_write_tokens,
            cost_usd: r.cost_usd, duration_ms: r.duration_ms, turns: r.turns,
            usage_source: r.usage_source,
        }
    }
}

// ---- 用量聚合行（U2/L1/L2；SUM 列全可空——无数据时 None 而非 0） ----

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotalsRow {
    pub sessions: i64,
    pub succeeded: Option<i64>,
    pub failed: Option<i64>,
    pub failed_cost: Option<f64>,
    pub cost: Option<f64>,
    pub reported: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct UsageTotalsSql {
    sessions: i64,
    succeeded: Option<i64>,
    failed: Option<i64>,
    failed_cost: Option<f64>,
    cost: Option<f64>,
    reported: Option<i64>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_read: Option<i64>,
}

impl From<UsageTotalsSql> for UsageTotalsRow {
    fn from(r: UsageTotalsSql) -> Self {
        Self { sessions: r.sessions, succeeded: r.succeeded, failed: r.failed, failed_cost: r.failed_cost,
               cost: r.cost, reported: r.reported, input_tokens: r.input_tokens, output_tokens: r.output_tokens,
               cache_read: r.cache_read }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageDailyRow {
    pub day: String,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct UsageDailySql {
    day: String,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
}

impl From<UsageDailySql> for UsageDailyRow {
    fn from(r: UsageDailySql) -> Self {
        Self { day: r.day, input_tokens: r.input_tokens, output_tokens: r.output_tokens }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageGroupRow {
    pub name: String,
    pub sessions: i64,
    pub value: Option<f64>,
}

#[derive(sqlx::FromRow)]
struct UsageGroupSql {
    name: String,
    sessions: i64,
    value: Option<f64>,
}

impl From<UsageGroupSql> for UsageGroupRow {
    fn from(r: UsageGroupSql) -> Self {
        Self { name: r.name, sessions: r.sessions, value: r.value }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageModuleRow {
    pub name: String,
    pub sessions: i64,
    pub cost: Option<f64>,
    pub failed: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct UsageModuleSql {
    name: String,
    sessions: i64,
    cost: Option<f64>,
    failed: Option<i64>,
}

impl From<UsageModuleSql> for UsageModuleRow {
    fn from(r: UsageModuleSql) -> Self {
        Self { name: r.name, sessions: r.sessions, cost: r.cost, failed: r.failed }
    }
}

/// agent 会话仓库：运行页历史回放 + 用量页统计的持久层。
/// 写入原则：边读边落（看门任务事件分派经 meta 广播进来），usage 列只 UPDATE 不 INSERT 默认值——
/// NULL 即「未回报」，聚合时 SUM 不会把「未知」吃成「零」。
pub struct AgentSessionRepo {
    pool: SqlitePool,
}

impl AgentSessionRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// 会话启动（spawn 成功即写；try_register 的外部会话如 Stub 巡检在 finalize 补插）。
    /// INSERT OR IGNORE：同一 session_id 重复触发（Starting→Running 双事件）只落一行。
    pub async fn upsert_started(&self, id: &str, repo: &str, cli: &str, started_at: &str) -> Result<(), ApiError> {
        sqlx::query(
            "INSERT OR IGNORE INTO agent_sessions (id, repo, cli, started_at, status) VALUES (?, ?, ?, ?, 'running')",
        )
        .bind(id)
        .bind(repo)
        .bind(cli)
        .bind(started_at)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// L1 归因：会话归属架构模块（任务会话反查 tasks.modules / 子图会话直接写入）
    pub async fn set_module_id(&self, id: &str, module_id: &str) -> Result<(), ApiError> {
        sqlx::query("UPDATE agent_sessions SET module_id = ? WHERE id = ?")
            .bind(module_id)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    /// M1.1：运行期落库标签与类型（note_label 即播——此前仅终态 finalize 才写，
    /// 运行中的会话在用量/运行页显示 unknown 或「归纳」误标，2026-10-05 实弹）
    pub async fn set_label_kind(&self, id: &str, label: &str, kind: &str) -> Result<(), ApiError> {
        sqlx::query("UPDATE agent_sessions SET label = ?, kind = ? WHERE id = ?")
            .bind(label)
            .bind(kind)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn set_model(&self, id: &str, model: &str) -> Result<(), ApiError> {
        sqlx::query("UPDATE agent_sessions SET model = ? WHERE id = ?")
            .bind(model)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    /// usage 全量覆写（claude result 事件为会话级累计值，last-write-wins）。
    pub async fn set_usage(
        &self,
        id: &str,
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_write_tokens: i64,
        cost_usd: f64,
        duration_ms: i64,
        turns: i64,
    ) -> Result<(), ApiError> {
        sqlx::query(
            "UPDATE agent_sessions SET input_tokens = ?, output_tokens = ?, cache_read_tokens = ?, \
             cache_write_tokens = ?, cost_usd = ?, duration_ms = ?, turns = ?, usage_source = 'claude-result' WHERE id = ?",
        )
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(cache_read_tokens)
        .bind(cache_write_tokens)
        .bind(cost_usd)
        .bind(duration_ms)
        .bind(turns)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// 终态收尾：status/terminal_at/exit_code/label/kind。INSERT OR IGNORE 在前——
    /// 无 spawn 的外部会话（Stub 巡检，cli='stub'）此前无行，这里补插地基行。
    pub async fn finalize(
        &self,
        id: &str,
        repo: &str,
        status: &str,
        terminal_at: &str,
        exit_code: Option<i64>,
        label: Option<&str>,
        kind: &str,
    ) -> Result<(), ApiError> {
        sqlx::query(
            "INSERT OR IGNORE INTO agent_sessions (id, repo, cli, kind, started_at, status) VALUES (?, ?, 'stub', ?, ?, 'running')",
        )
        .bind(id)
        .bind(repo)
        .bind(kind)
        .bind(terminal_at)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        sqlx::query(
            "UPDATE agent_sessions SET status = ?, terminal_at = ?, exit_code = ?, label = ?, kind = ? WHERE id = ?",
        )
        .bind(status)
        .bind(terminal_at)
        .bind(exit_code)
        .bind(label)
        .bind(kind)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// 总量统计（KPI 行）：cost/tokens 的 SUM 与「自报次数」分开——NULL 语义诚实降级
    pub async fn usage_totals(&self, repo: &str, since: &str) -> Result<UsageTotalsRow, ApiError> {
        let row = sqlx::query_as::<_, UsageTotalsSql>(
            "SELECT COUNT(*) AS sessions,                 SUM(CASE WHEN status='succeeded' THEN 1 ELSE 0 END) AS succeeded,                 SUM(CASE WHEN status='failed' THEN 1 ELSE 0 END) AS failed,                 CAST(SUM(CASE WHEN status='failed' THEN cost_usd ELSE 0 END) AS REAL) AS failed_cost,                 CAST(SUM(cost_usd) AS REAL) AS cost,                 SUM(CASE WHEN cost_usd IS NOT NULL THEN 1 ELSE 0 END) AS reported,                 SUM(input_tokens) AS input_tokens, SUM(output_tokens) AS output_tokens,                 SUM(cache_read_tokens) AS cache_read              FROM agent_sessions WHERE repo = ? AND started_at >= ?",
        )
        .bind(repo)
        .bind(since)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.into())
    }

    /// 按日 tokens（堆积柱）：started_at 为 RFC3339 文本，日期截前 10 位
    pub async fn usage_daily(&self, repo: &str, since: &str) -> Result<Vec<UsageDailyRow>, ApiError> {
        let rows = sqlx::query_as::<_, UsageDailySql>(
            "SELECT substr(started_at, 1, 10) AS day,                 SUM(input_tokens) AS input_tokens, SUM(output_tokens) AS output_tokens              FROM agent_sessions WHERE repo = ? AND started_at >= ?              GROUP BY day ORDER BY day",
        )
        .bind(repo)
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// 按类型分布：会话数 + 花费（子 agent 的 kind 已折入 task/subagent-*，前端再按父类聚合）
    pub async fn usage_by_kind(&self, repo: &str, since: &str) -> Result<Vec<UsageGroupRow>, ApiError> {
        let rows = sqlx::query_as::<_, UsageGroupSql>(
            "SELECT kind AS name, COUNT(*) AS sessions, CAST(SUM(cost_usd) AS REAL) AS value              FROM agent_sessions WHERE repo = ? AND started_at >= ?              GROUP BY kind ORDER BY value DESC",
        )
        .bind(repo)
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// 按模型分布：会话数 + tokens（真实模型名来自 system init 事件）
    pub async fn usage_by_model(&self, repo: &str, since: &str) -> Result<Vec<UsageGroupRow>, ApiError> {
        let rows = sqlx::query_as::<_, UsageGroupSql>(
            "SELECT model AS name, COUNT(*) AS sessions,                 CAST(SUM(COALESCE(input_tokens,0) + COALESCE(output_tokens,0)) AS REAL) AS value              FROM agent_sessions WHERE repo = ? AND started_at >= ? AND model IS NOT NULL              GROUP BY model ORDER BY value DESC",
        )
        .bind(repo)
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// L1 按模块归因：会话数 + 花费 + 失败数（治理账单；module_id 为 NULL 的会话不入此表）
    pub async fn usage_by_module(&self, repo: &str, since: &str) -> Result<Vec<UsageModuleRow>, ApiError> {
        let rows = sqlx::query_as::<_, UsageModuleSql>(
            "SELECT module_id AS name, COUNT(*) AS sessions, CAST(SUM(cost_usd) AS REAL) AS cost,                 SUM(CASE WHEN status='failed' THEN 1 ELSE 0 END) AS failed              FROM agent_sessions WHERE repo = ? AND started_at >= ? AND module_id IS NOT NULL              GROUP BY module_id ORDER BY cost DESC LIMIT 10",
        )
        .bind(repo)
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn list(&self, repo: &str, limit: i64) -> Result<Vec<AgentSessionRow>, ApiError> {
        let rows = sqlx::query_as::<_, AgentSessionRowSql>(
            "SELECT id, repo, kind, label, cli, model, parent_session_id, task_id, started_at, terminal_at, \
             status, exit_code, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, \
             cost_usd, duration_ms, turns, usage_source, module_id
             FROM agent_sessions WHERE repo = ? ORDER BY started_at DESC LIMIT ?",
        )
        .bind(repo)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    // M1/U1：agent_sessions 生命周期——upsert_started 幂等 / usage 覆写 / finalize 兜底插入 / NULL 语义
    #[tokio::test]
    async fn agent_sessions_lifecycle() {
        let db = Database::connect_memory().await.unwrap();
        let repo = AgentSessionRepo::new(db.pool().clone());
        // spawn 路径：Cli 事件写行
        repo.upsert_started("sess-one", "demo", "claude", "2026-10-05T06:00:00Z").await.unwrap();
        repo.upsert_started("sess-one", "demo", "claude", "2026-10-05T06:00:00Z").await.unwrap(); // 幂等
        // system init：模型名
        repo.set_model("sess-one", "claude-sonnet-4-6").await.unwrap();
        // result：usage 覆写（NULL 列被填）
        repo.set_usage("sess-one", 12000, 3000, 8000, 4000, 0.0421, 52000, 3).await.unwrap();
        // 终态 finalize
        repo.finalize("sess-one", "demo", "succeeded", "2026-10-05T06:01:00Z", Some(0), Some("自动归纳"), "induce").await.unwrap();
        let rows = repo.list("demo", 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.kind, "induce");
        assert_eq!(r.cli.as_deref(), Some("claude"));
        assert_eq!(r.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(r.input_tokens, Some(12000));
        assert_eq!(r.cost_usd.map(|c| (c * 10000.0).round()), Some(421.0));
        assert_eq!(r.usage_source.as_deref(), Some("claude-result"));
        assert_eq!(r.exit_code, Some(0));
        // Stub 巡检路径：无 spawn 无 Cli 事件，finalize INSERT OR IGNORE 兜底（cli='stub'）
        repo.finalize("patrol-x", "demo", "succeeded", "2026-10-05T07:00:00Z", None, Some("巡检"), "patrol").await.unwrap();
        let rows = repo.list("demo", 10).await.unwrap();
        assert_eq!(rows.len(), 2);
        let stub = rows.iter().find(|r| r.id == "patrol-x").unwrap();
        assert_eq!(stub.cli.as_deref(), Some("stub"));
        // 被 kill 的会话：usage 列恒 NULL（诚实降级，区别于 0）
        assert!(stub.input_tokens.is_none());
        assert!(stub.cost_usd.is_none());
        // 跨仓库隔离
        assert!(repo.list("other", 10).await.unwrap().is_empty());
    }

    // 2026-10-05 实弹回归（用量页转圈）：无失败会话时 failed_cost 的 CASE-ELSE 0 使 SUM 返回
    // INTEGER，sqlx 按 f64 解码直接 500——所有 f64 聚合列必须 CAST AS REAL 兜底。
    #[tokio::test]
    async fn usage_aggregates_cast_real_when_no_failed_sessions() {
        let db = Database::connect_memory().await.unwrap();
        let repo = AgentSessionRepo::new(db.pool().clone());
        // 只有成功会话（无 failed 行）——failed_cost 的 SUM 全是整数 0
        repo.upsert_started("s-ok", "demo", "claude", "2026-10-05T06:00:00Z").await.unwrap();
        repo.set_usage("s-ok", 1000, 500, 0, 0, 0.01, 1500, 1).await.unwrap();
        repo.finalize("s-ok", "demo", "succeeded", "2026-10-05T06:01:00Z", Some(0), None, "task").await.unwrap();
        // 完全空的仓库（SUM 全 NULL）
        let totals = repo.usage_totals("demo", "2026-01-01T00:00:00Z").await.unwrap();
        assert_eq!(totals.sessions, 1);
        assert_eq!(totals.failed_cost, Some(0.0), "无失败会话时 failed_cost 应为 0.0 而非解码错误");
        assert_eq!(totals.cost.map(|c| (c * 100.0).round()), Some(1.0));
        let empty = repo.usage_totals("empty", "2026-01-01T00:00:00Z").await.unwrap();
        assert_eq!(empty.sessions, 0);
        assert_eq!(empty.cost, None);
        // by_kind / by_model / by_module 的 REAL 列同理（tokens 聚合也是整数路径）
        repo.upsert_started("s-ok2", "demo", "claude", "2026-10-05T07:00:00Z").await.unwrap();
        repo.set_model("s-ok2", "claude-sonnet-4-6").await.unwrap();
        repo.set_usage("s-ok2", 2000, 1000, 0, 0, 0.02, 3000, 1).await.unwrap();
        repo.finalize("s-ok2", "demo", "succeeded", "2026-10-05T07:01:00Z", Some(0), None, "induce").await.unwrap();
        let kinds = repo.usage_by_kind("demo", "2026-01-01T00:00:00Z").await.unwrap();
        assert!(kinds.iter().any(|k| k.name == "task" && k.value.map(|v| (v * 100.0).round()) == Some(1.0)));
        let models = repo.usage_by_model("demo", "2026-01-01T00:00:00Z").await.unwrap();
        assert!(models.iter().any(|m| m.name == "claude-sonnet-4-6" && m.value == Some(3000.0)));
    }
}
