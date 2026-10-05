//! task 聚合域：任务宽表 CRUD 与状态机流转/产物/归因/返工链。
use easyvibe_common::ApiError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::core::{db_err, now_ms};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRow {
    pub id: String,
    pub repo: String,
    pub title: String,
    pub description: String,
    pub modules: String,   // JSON
    pub acceptance: String,
    pub source: String,
    pub context: String,   // JSON
    pub status: String,
    pub trust: String,
    pub error: Option<String>,
    pub session_id: Option<String>,
    pub gate: Option<String>,
    pub prompt_tokens: Option<i64>,     // M3-5：token 用量（CLI agent 无法回报时留 NULL）
    pub completion_tokens: Option<i64>,
    pub result: Option<String>,         // M4-1：产物归档 JSON（summary/changedModules/archivedPath/diffStat）
    pub base_head: Option<String>,      // 变更归因：任务启动时的 git HEAD（0008）
    pub created_at: String,
    pub updated_at: String,
    pub conversation_id: Option<String>,   // M4-2：任务←→会话关联
    pub origin_task_id: Option<String>,    // R3 D2：返工来源（复制为新任务的原始任务）
    pub successor_task_id: Option<String>, // R3 D2：原任务视角的反链（返工率聚合的命脉）
}

pub trait TaskRepository: Send + Sync {
    fn create(&self, t: &TaskRow) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    fn update_status(&self, id: &str, status: &str, error: Option<&str>) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    /// N27 原子关卡推进：UPDATE ... WHERE status='awaiting_approval' AND gate IS ?——
    /// 影响行数为 0 即并发审批/状态漂移，调用方返回 409。check-then-act 无锁竞态的消解点。
    fn try_advance_gate(
        &self,
        id: &str,
        expected_gate: Option<&str>,
        new_gate: Option<&str>,
        new_status: Option<&str>,
    ) -> impl std::future::Future<Output = Result<u64, ApiError>> + Send;
    fn list(&self, repo: &str, limit: i64) -> impl std::future::Future<Output = Result<Vec<TaskRow>, ApiError>> + Send;
    /// M4-2：会话关联任务（工作台影响面/待审批聚合）
    fn list_by_conversation(&self, conversation_id: &str) -> impl std::future::Future<Output = Result<Vec<TaskRow>, ApiError>> + Send;
    fn get(&self, id: &str) -> impl std::future::Future<Output = Result<Option<TaskRow>, ApiError>> + Send;
    fn set_session(&self, id: &str, session_id: &str) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    fn set_gate(&self, id: &str, gate: Option<&str>) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    /// 变更归因：记录任务启动时的 git HEAD（工作区脏时归因不混历史改动）
    fn set_base_head(&self, id: &str, head: &str) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    /// M4-1：任务终态采集产物（[EASYVIBE-RESULT] 解析 + diff stat + 归档路径）
    fn set_result(&self, id: &str, result_json: &str) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    /// M3-5（§11 🟡4）：后端重启会杀掉 spawn 的 agent（kill_on_drop）——
    /// 启动时把 running 任务标记 interrupted（awaiting_approval 是等用户决策，不受影响）
    fn interrupt_running(&self) -> impl std::future::Future<Output = Result<u64, ApiError>> + Send;
    /// R3 D2：返工链反链回填
    fn set_successor(&self, id: &str, successor: &str) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    /// 管理闭环（2026-10-03 现状重审 P0）：删除任务——级联清 approvals；
    /// 归档文件由路由层按需清理（development_docs/{id}.json 是任务自有产物）。
    /// 调用方负责先终止 running 会话。
    fn delete(&self, id: &str) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    /// 就地重试（failed/interrupted → pending 重新入队）：原子条件 UPDATE——
    /// 状态不在白名单内影响行数 0，调用方返回 409。同时清 error/gate/session 残留，
    /// 避免重试任务带着旧关卡/旧会话指针启动。
    fn reset_for_retry(&self, id: &str) -> impl std::future::Future<Output = Result<u64, ApiError>> + Send;
    /// 修改并复审（rejected → running 直达实施阶段）：原子条件 UPDATE——
    /// 只认 rejected（用户打回走复制新任务，审查打回走本通道）。
    fn reset_for_remediate(&self, id: &str) -> impl std::future::Future<Output = Result<u64, ApiError>> + Send;
    /// 管道回看·节点重开（2026-10-05 方案 §3.1）：把任务放回目标评审关。
    /// 允许源状态集覆盖「走到后面想回头」的全部场景：待审/失败/中断/审查打回/已归档——
    /// 原子条件 UPDATE（评审#B2：不能用 try_advance_gate，其 WHERE 硬编码 awaiting_approval，
    /// done 任务会恒 0 行）；并发安全靠状态集判定 + decide 的 N27 expected_gate 兜底。
    /// running/pending 不在集内：running 必须先终止（kill→failed 后本方法接），
    /// pending 是写互斥退回的瞬态（5s 自愈，不宜插队改关卡）。
    fn try_rewind(
        &self,
        id: &str,
        target_gate: &str,
    ) -> impl std::future::Future<Output = Result<u64, ApiError>> + Send;
    /// 修改并复审的上下文注入：写入 remediation 反馈（JSON 整体替换 context 列）
    fn set_context(&self, id: &str, context: &str) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
}

pub struct SqliteTaskRepository {
    pool: SqlitePool,
}

impl SqliteTaskRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct TaskRowSql {
    id: String, repo: String, title: String, description: String,
    modules: String, acceptance: String, source: String, context: String,
    status: String, trust: String, error: Option<String>, session_id: Option<String>, gate: Option<String>,
    prompt_tokens: Option<i64>, completion_tokens: Option<i64>, result: Option<String>, base_head: Option<String>, created_at: String, updated_at: String,
    pub conversation_id: Option<String>,   // M4-2：任务←→会话关联
    pub origin_task_id: Option<String>,    // R3 D2
    pub successor_task_id: Option<String>, // R3 D2
}

impl From<TaskRowSql> for TaskRow {
    fn from(r: TaskRowSql) -> Self {
        Self { id: r.id, repo: r.repo, title: r.title, description: r.description, modules: r.modules, acceptance: r.acceptance, source: r.source, context: r.context, status: r.status, trust: r.trust, error: r.error, session_id: r.session_id, gate: r.gate, prompt_tokens: r.prompt_tokens, completion_tokens: r.completion_tokens, result: r.result, base_head: r.base_head, created_at: r.created_at, updated_at: r.updated_at, conversation_id: r.conversation_id, origin_task_id: r.origin_task_id, successor_task_id: r.successor_task_id,}
    }
}

impl TaskRepository for SqliteTaskRepository {
    async fn create(&self, t: &TaskRow) -> Result<(), ApiError> {
        sqlx::query(
            "INSERT INTO tasks (id, repo, title, description, modules, acceptance, source, context, status, trust, session_id, gate, prompt_tokens, completion_tokens, result, created_at, updated_at, conversation_id, origin_task_id, successor_task_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&t.id).bind(&t.repo).bind(&t.title).bind(&t.description).bind(&t.modules)
        .bind(&t.acceptance).bind(&t.source).bind(&t.context).bind(&t.status).bind(&t.trust)
        .bind(&t.session_id).bind(&t.gate).bind(t.prompt_tokens).bind(t.completion_tokens).bind(&t.result).bind(&t.created_at).bind(&t.updated_at).bind(&t.conversation_id).bind(&t.origin_task_id).bind(&t.successor_task_id)
        .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    /// R3 D2：返工链反链——复制出新任务时回填原任务的 successor
    async fn set_successor(&self, id: &str, successor: &str) -> Result<(), ApiError> {
        sqlx::query("UPDATE tasks SET successor_task_id = ? WHERE id = ?")
            .bind(successor).bind(id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn try_advance_gate(&self, id: &str, expected_gate: Option<&str>, new_gate: Option<&str>, new_status: Option<&str>) -> Result<u64, ApiError> {
        let now = now_ms();
        let r = sqlx::query(
            "UPDATE tasks SET gate = COALESCE(?, gate), status = COALESCE(?, status), updated_at = ? WHERE id = ? AND status = 'awaiting_approval' AND gate IS ?",
        )
        .bind(new_gate)
        .bind(new_status)
        .bind(&now)
        .bind(id)
        .bind(expected_gate)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(r.rows_affected())
    }

    async fn update_status(&self, id: &str, status: &str, error: Option<&str>) -> Result<(), ApiError> {
        sqlx::query("UPDATE tasks SET status = ?, error = ?, updated_at = ? WHERE id = ?")
            .bind(status).bind(error)
            .bind(now_ms())
            .bind(id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn list_by_conversation(&self, conversation_id: &str) -> Result<Vec<TaskRow>, ApiError> {
        let rows = sqlx::query_as::<_, TaskRowSql>("SELECT * FROM tasks WHERE conversation_id = ? ORDER BY created_at DESC")
            .bind(conversation_id).fetch_all(&self.pool).await.map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn list(&self, repo: &str, limit: i64) -> Result<Vec<TaskRow>, ApiError> {
        let rows = sqlx::query_as::<_, TaskRowSql>(
            "SELECT * FROM tasks WHERE repo = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(repo).bind(limit)
        .fetch_all(&self.pool).await.map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn get(&self, id: &str) -> Result<Option<TaskRow>, ApiError> {
        let row = sqlx::query_as::<_, TaskRowSql>("SELECT * FROM tasks WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool).await.map_err(db_err)?;
        Ok(row.map(Into::into))
    }

    async fn set_gate(&self, id: &str, gate: Option<&str>) -> Result<(), ApiError> {
        sqlx::query("UPDATE tasks SET gate = ? WHERE id = ?")
            .bind(gate).bind(id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn set_session(&self, id: &str, session_id: &str) -> Result<(), ApiError> {
        sqlx::query("UPDATE tasks SET session_id = ? WHERE id = ?")
            .bind(session_id).bind(id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn set_base_head(&self, id: &str, head: &str) -> Result<(), ApiError> {
        sqlx::query("UPDATE tasks SET base_head = ? WHERE id = ?")
            .bind(head).bind(id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn set_result(&self, id: &str, result_json: &str) -> Result<(), ApiError> {
        sqlx::query("UPDATE tasks SET result = ? WHERE id = ?")
            .bind(result_json).bind(id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn interrupt_running(&self) -> Result<u64, ApiError> {
        let res = sqlx::query(
            "UPDATE tasks SET status = 'interrupted', error = '后端重启，agent 会话已终止（kill_on_drop）；请重新发起任务', updated_at = ? WHERE status = 'running'",
        )
        .bind(now_ms())
        .execute(&self.pool).await.map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn delete(&self, id: &str) -> Result<(), ApiError> {
        // 级联先清审批留痕（外键无约束，手动级联）
        sqlx::query("DELETE FROM approvals WHERE task_id = ?").bind(id)
            .execute(&self.pool).await.map_err(db_err)?;
        sqlx::query("DELETE FROM tasks WHERE id = ?").bind(id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn reset_for_retry(&self, id: &str) -> Result<u64, ApiError> {
        let res = sqlx::query(
            "UPDATE tasks SET status = 'pending', error = NULL, gate = NULL, session_id = NULL, updated_at = ? WHERE id = ? AND status IN ('failed', 'interrupted')",
        )
        .bind(now_ms())
        .bind(id)
        .execute(&self.pool).await.map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn reset_for_remediate(&self, id: &str) -> Result<u64, ApiError> {
        let res = sqlx::query(
            "UPDATE tasks SET status = 'running', error = NULL, gate = 'p:implement', session_id = NULL, updated_at = ? WHERE id = ? AND status = 'rejected'",
        )
        .bind(now_ms())
        .bind(id)
        .execute(&self.pool).await.map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn set_context(&self, id: &str, context: &str) -> Result<(), ApiError> {
        sqlx::query("UPDATE tasks SET context = ? WHERE id = ?")
            .bind(context)
            .bind(id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn try_rewind(&self, id: &str, target_gate: &str) -> Result<u64, ApiError> {
        let res = sqlx::query(
            "UPDATE tasks SET status = 'awaiting_approval', error = NULL, gate = ?, session_id = NULL, updated_at = ? WHERE id = ? AND status IN ('awaiting_approval','failed','interrupted','rejected','done')",
        )
        .bind(target_gate)
        .bind(now_ms())
        .bind(id)
        .execute(&self.pool).await.map_err(db_err)?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    #[tokio::test]
    async fn task_roundtrip() {
        let db = Database::connect_memory().await.unwrap();
        let repo = SqliteTaskRepository::new(db.pool().clone());
        let t = TaskRow {
            id: "task-1".into(), repo: "demo".into(), title: "修复耦合".into(),
            description: "d".into(), modules: "[\"m1\"]".into(), acceptance: "a".into(),
            source: "concern".into(), context: "{}".into(), status: "pending".into(),
            trust: "manual".into(), error: None, session_id: None, gate: None,
            prompt_tokens: None, completion_tokens: None, result: None, base_head: None,
            created_at: "1".into(), updated_at: "1".into(), conversation_id: None,
            origin_task_id: None, successor_task_id: None,
        };
        repo.create(&t).await.unwrap();
        repo.update_status("task-1", "running", None).await.unwrap();
        let list = repo.list("demo", 10).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].status, "running");
        assert_eq!(list[0].source, "concern");
        assert!(repo.get("task-1").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn task_timestamps_uniform_epoch_ms() {
        // 2026-10-03 实弹 bug 回归网：updated_at 曾写秒（create/try_advance_gate 写毫秒）——
        // dev-docs 时间窗当毫秒解析，产物文档全漏检（评审卡空）。此处锁死：所有写路径
        // 的 updated_at 必须是 13 位毫秒。
        let db = Database::connect_memory().await.unwrap();
        let repo = SqliteTaskRepository::new(db.pool().clone());
        let mk = |id: &str, status: &str| TaskRow {
            id: id.into(), repo: "demo".into(), title: "t".into(), description: "d".into(),
            modules: "[]".into(), acceptance: String::new(), source: "manual".into(), context: "{}".into(),
            status: status.into(), trust: "manual".into(), error: None, session_id: None, gate: None,
            prompt_tokens: None, completion_tokens: None, result: None, base_head: None,
            created_at: "1791016035807".into(), updated_at: "1791016035807".into(), conversation_id: None,
            origin_task_id: None, successor_task_id: None,
        };
        repo.create(&mk("task-ms", "pending")).await.unwrap();
        repo.update_status("task-ms", "running", None).await.unwrap();
        let n = repo.get("task-ms").await.unwrap().unwrap().updated_at;
        assert_eq!(n.len(), 13, "update_status 必须写毫秒，实际: {n}");

        repo.create(&mk("task-gate", "awaiting_approval")).await.unwrap();
        repo.set_gate("task-gate", Some("plan")).await.unwrap();
        repo.try_advance_gate("task-gate", Some("plan"), Some("p:analysis"), Some("running")).await.unwrap();
        let g = repo.get("task-gate").await.unwrap().unwrap().updated_at;
        assert_eq!(g.len(), 13, "try_advance_gate 必须写毫秒，实际: {g}");

        repo.create(&mk("task-retry", "failed")).await.unwrap();
        repo.reset_for_retry("task-retry").await.unwrap();
        let r = repo.get("task-retry").await.unwrap().unwrap().updated_at;
        assert_eq!(r.len(), 13, "reset_for_retry 必须写毫秒，实际: {r}");
        // 数据修复迁移（0012）：秒值旧行升级毫秒——内存库跑同一迁移，直接验证幂等
        sqlx::query("UPDATE tasks SET updated_at = '1791016109' WHERE id = 'task-ms'").execute(db.pool()).await.unwrap();
        sqlx::query("UPDATE tasks SET updated_at = CAST(updated_at AS INTEGER) * 1000 WHERE length(updated_at) <= 10")
            .execute(db.pool()).await.unwrap();
        let fixed = repo.get("task-ms").await.unwrap().unwrap().updated_at;
        assert_eq!(fixed, "1791016109000", "秒值旧行必须升级为毫秒");
    }

    #[tokio::test]
    async fn running_tasks_interrupted_on_restart() {
        let db = Database::connect_memory().await.unwrap();
        let repo = SqliteTaskRepository::new(db.pool().clone());
        let mk = |id: &str, status: &str| TaskRow {
            id: id.into(), repo: "demo".into(), title: "t".into(), description: "d".into(),
            modules: "[]".into(), acceptance: "a".into(), source: "manual".into(), context: "{}".into(),
            status: status.into(), trust: "auto".into(), error: None, session_id: None, gate: None,
            prompt_tokens: None, completion_tokens: None, result: None, base_head: None, created_at: "1".into(), updated_at: "1".into(),
            conversation_id: None, origin_task_id: None, successor_task_id: None,
        };
        repo.create(&mk("task-run", "running")).await.unwrap();
        repo.create(&mk("task-wait", "awaiting_approval")).await.unwrap();
        repo.create(&mk("task-pend", "pending")).await.unwrap();
        let n = repo.interrupt_running().await.unwrap();
        assert_eq!(n, 1, "只有 running 被标记（§11 🟡4）");
        assert_eq!(repo.get("task-run").await.unwrap().unwrap().status, "interrupted");
        assert!(repo.get("task-run").await.unwrap().unwrap().error.unwrap().contains("重启"));
        assert_eq!(repo.get("task-wait").await.unwrap().unwrap().status, "awaiting_approval", "等用户决策的任务不受影响");
        assert_eq!(repo.get("task-pend").await.unwrap().unwrap().status, "pending", "排队任务重新入队语义不受影响");
    }
}
