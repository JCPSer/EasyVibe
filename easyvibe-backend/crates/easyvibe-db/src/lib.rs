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
    pub prompt_tokens: Option<i64>,     // M3-5：token 用量记录（§10 #4 第一步）
    pub completion_tokens: Option<i64>,
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
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
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
            "UPDATE patrol_runs SET finished_at = ?, status = ?, arch_score = ?, error = ?, prompt_tokens = ?, completion_tokens = ? WHERE id = ?",
        )
        .bind(&fin.finished_at)
        .bind(&fin.status)
        .bind(fin.arch_score)
        .bind(&fin.error)
        .bind(fin.prompt_tokens)
        .bind(fin.completion_tokens)
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
            "SELECT id, repo, started_at, finished_at, status, model, arch_score, error, prompt_tokens, completion_tokens
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

// ---------- 任务（M3-2） ----------

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
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0).to_string();
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
}

// ---------- 使用证据埋点（R3 D1，战略报告 P0-2：门控的秤） ----------

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
            .bind(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0).to_string())
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

// ---------- 审批（M3-4） ----------

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

// ---------- 会话持久化（M3-5：对话历史落域 2，留痕与压缩的存储分离 §11 🟡5） ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRow {
    pub id: String,
    pub repo: String,
    pub summary: Option<String>,
    pub compacted_before: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub created_at: String,
    pub updated_at: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessageRow {
    pub id: i64,
    pub conversation_id: String,
    pub role: String, // user / assistant / system
    pub content: String,
    pub compacted: bool,
    pub tokens: i64,
    pub created_at: String,
}

pub trait ConversationRepository: Send + Sync {
    /// 每仓库一个会话（入口对话），不存在则创建
    fn get_or_create(&self, repo: &str) -> impl std::future::Future<Output = Result<ConversationRow, ApiError>> + Send;
    /// 追加一条消息，返回 rowid（用于压缩水位）
    fn append_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        tokens: i64,
    ) -> impl std::future::Future<Output = Result<i64, ApiError>> + Send;
    /// 回放从库读（R1 分页：before_id 之前的一页，升序返回；None=最新一页）
    fn list_messages(
        &self,
        conversation_id: &str,
        before_id: Option<i64>,
        limit: i64,
    ) -> impl std::future::Future<Output = Result<Vec<ConversationMessageRow>, ApiError>> + Send;
    /// 运行态上下文：仅未压缩消息（近期窗口原文保留）
    fn list_uncompacted(
        &self,
        conversation_id: &str,
    ) -> impl std::future::Future<Output = Result<Vec<ConversationMessageRow>, ApiError>> + Send;
    /// 压缩执行：水位之前的消息标 compacted（原文保留），摘要与累计 token 更新
    fn apply_compaction(
        &self,
        conversation_id: &str,
        before_id: i64,
        summary: &str,
        prompt_tokens_add: i64,
        completion_tokens_add: i64,
    ) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    /// token 用量累计（§10 #4 成本护栏第一步）
    fn add_tokens(
        &self,
        conversation_id: &str,
        prompt_tokens: i64,
        completion_tokens: i64,
    ) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    /// 清空会话（保留会话行，消息与摘要重置——"新对话"按钮的原料）
    fn reset(&self, conversation_id: &str) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    // ---- M4-2 多会话 ----
    /// 该仓库全部会话（最近活跃在前）
    fn list_by_repo(&self, repo: &str) -> impl std::future::Future<Output = Result<Vec<ConversationRow>, ApiError>> + Send;
    /// 新建命名会话（id 由调用方生成）
    fn create(&self, id: &str, repo: &str, title: Option<&str>) -> impl std::future::Future<Output = Result<ConversationRow, ApiError>> + Send;
    fn rename(&self, conversation_id: &str, title: &str) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    /// 删除会话（先删消息，FK 无级联）
    fn delete(&self, conversation_id: &str) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    fn count_messages(&self, conversation_id: &str) -> impl std::future::Future<Output = Result<i64, ApiError>> + Send;
}

pub struct SqliteConversationRepository {
    pool: SqlitePool,
}

impl SqliteConversationRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct ConversationRowSql {
    id: String, repo: String, summary: Option<String>, compacted_before: i64,
    prompt_tokens: i64, completion_tokens: i64, created_at: String, updated_at: String,
    title: Option<String>,   // M4-2 多会话：用户可命名
}

impl From<ConversationRowSql> for ConversationRow {
    fn from(r: ConversationRowSql) -> Self {
        Self {
            id: r.id, repo: r.repo, summary: r.summary, compacted_before: r.compacted_before,
            prompt_tokens: r.prompt_tokens, completion_tokens: r.completion_tokens,
            created_at: r.created_at, updated_at: r.updated_at,
            title: r.title,
        }
    }
}

#[derive(sqlx::FromRow)]
struct ConversationMessageRowSql {
    id: i64, conversation_id: String, role: String, content: String,
    compacted: i64, tokens: i64, created_at: String,
}

impl From<ConversationMessageRowSql> for ConversationMessageRow {
    fn from(r: ConversationMessageRowSql) -> Self {
        Self {
            id: r.id, conversation_id: r.conversation_id, role: r.role, content: r.content,
            compacted: r.compacted != 0, tokens: r.tokens, created_at: r.created_at,
        }
    }
}

fn now_secs() -> String {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string()
}

/// tasks 表时间戳统一口径：epoch 毫秒（create_task/try_advance_gate 即毫秒——
/// 2026-10-03 实弹 bug：update_status 等写秒，dev-docs 时间窗当毫秒解析导致产物全漏检）。
/// 会话/事件等内部自洽的表仍用 now_secs，不混用。
fn now_ms() -> String {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0).to_string()
}

/// 注销仓库的数据清除（2026-10-03 重审 P1）：抹掉该仓库在本地库的全部痕迹。
/// 删序：子表先、父表后（schema 无外键级联，全手动序）；settings 按 scope = repo id 清除。
/// 返回删除总行数（粗粒度观测值，日志用）。
pub async fn wipe_repo(pool: &sqlx::SqlitePool, repo: &str) -> Result<u64, ApiError> {
    let mut n = 0u64;
    macro_rules! del {
        ($sql:expr) => {
            n += sqlx::query($sql).bind(repo).execute(pool).await.map_err(db_err)?.rows_affected();
        };
    }
    del!("DELETE FROM module_health_history WHERE run_id IN (SELECT id FROM patrol_runs WHERE repo = ?)");
    del!("DELETE FROM patrol_runs WHERE repo = ?");
    del!("DELETE FROM events WHERE repo = ?");
    del!("DELETE FROM approvals WHERE task_id IN (SELECT id FROM tasks WHERE repo = ?)");
    del!("DELETE FROM conversation_messages WHERE conversation_id IN (SELECT id FROM conversations WHERE repo = ?)");
    del!("DELETE FROM conversations WHERE repo = ?");
    del!("DELETE FROM tasks WHERE repo = ?");
    del!("DELETE FROM settings WHERE scope = ?");
    Ok(n)
}

impl ConversationRepository for SqliteConversationRepository {
    async fn get_or_create(&self, repo: &str) -> Result<ConversationRow, ApiError> {
        // M4-2 多会话：默认会话 = 该仓库最近活跃的会话（旧行为一仓一会话时即唯一会话），无则建 "chat:{repo}"
        if let Some(row) = sqlx::query_as::<_, ConversationRowSql>(
            "SELECT * FROM conversations WHERE repo = ? ORDER BY updated_at DESC LIMIT 1",
        ).bind(repo).fetch_optional(&self.pool).await.map_err(db_err)? {
            return Ok(row.into());
        }
        let id = format!("chat:{repo}");
        let now = now_secs();
        sqlx::query("INSERT INTO conversations (id, repo, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(&id).bind(repo).bind(&now).bind(&now)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(sqlx::query_as::<_, ConversationRowSql>("SELECT * FROM conversations WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool).await.map_err(db_err)?.into())
    }

    async fn list_by_repo(&self, repo: &str) -> Result<Vec<ConversationRow>, ApiError> {
        Ok(sqlx::query_as::<_, ConversationRowSql>("SELECT * FROM conversations WHERE repo = ? ORDER BY updated_at DESC")
            .bind(repo).fetch_all(&self.pool).await.map_err(db_err)?.into_iter().map(Into::into).collect())
    }

    async fn create(&self, id: &str, repo: &str, title: Option<&str>) -> Result<ConversationRow, ApiError> {
        let now = now_secs();
        sqlx::query("INSERT INTO conversations (id, repo, title, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
            .bind(id).bind(repo).bind(title).bind(&now).bind(&now)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(sqlx::query_as::<_, ConversationRowSql>("SELECT * FROM conversations WHERE id = ?")
            .bind(id).fetch_one(&self.pool).await.map_err(db_err)?.into())
    }

    async fn rename(&self, conversation_id: &str, title: &str) -> Result<(), ApiError> {
        sqlx::query("UPDATE conversations SET title = ?, updated_at = ? WHERE id = ?")
            .bind(title).bind(now_secs()).bind(conversation_id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn delete(&self, conversation_id: &str) -> Result<(), ApiError> {
        sqlx::query("DELETE FROM conversation_messages WHERE conversation_id = ?").bind(conversation_id)
            .execute(&self.pool).await.map_err(db_err)?;
        sqlx::query("DELETE FROM conversations WHERE id = ?").bind(conversation_id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn count_messages(&self, conversation_id: &str) -> Result<i64, ApiError> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversation_messages WHERE conversation_id = ?")
            .bind(conversation_id).fetch_one(&self.pool).await.map_err(db_err)?;
        Ok(row.0)
    }

    async fn append_message(&self, conversation_id: &str, role: &str, content: &str, tokens: i64) -> Result<i64, ApiError> {
        let res = sqlx::query(
            "INSERT INTO conversation_messages (conversation_id, role, content, tokens, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(conversation_id).bind(role).bind(content).bind(tokens).bind(now_secs())
        .execute(&self.pool).await.map_err(db_err)?;
        sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
            .bind(now_secs()).bind(conversation_id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(res.last_insert_rowid())
    }

    async fn list_messages(&self, conversation_id: &str, before_id: Option<i64>, limit: i64) -> Result<Vec<ConversationMessageRow>, ApiError> {
        // R1 清债：倒序取一页再翻回升序（before_id=None 取最新一页）
        let rows = match before_id {
            Some(b) => sqlx::query_as::<_, ConversationMessageRowSql>(
                "SELECT * FROM conversation_messages WHERE conversation_id = ? AND id < ? ORDER BY id DESC LIMIT ?",
            )
            .bind(conversation_id).bind(b).bind(limit)
            .fetch_all(&self.pool).await.map_err(db_err)?,
            None => sqlx::query_as::<_, ConversationMessageRowSql>(
                "SELECT * FROM conversation_messages WHERE conversation_id = ? ORDER BY id DESC LIMIT ?",
            )
            .bind(conversation_id).bind(limit)
            .fetch_all(&self.pool).await.map_err(db_err)?,
        };
        let mut rows: Vec<ConversationMessageRow> = rows.into_iter().map(Into::into).collect();
        rows.reverse();
        let _ = limit;
        Ok(rows)
    }

    async fn list_uncompacted(&self, conversation_id: &str) -> Result<Vec<ConversationMessageRow>, ApiError> {
        let rows = sqlx::query_as::<_, ConversationMessageRowSql>(
            "SELECT * FROM conversation_messages WHERE conversation_id = ? AND compacted = 0 ORDER BY id",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool).await.map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn apply_compaction(&self, conversation_id: &str, before_id: i64, summary: &str, pt_add: i64, ct_add: i64) -> Result<(), ApiError> {
        sqlx::query("UPDATE conversation_messages SET compacted = 1 WHERE conversation_id = ? AND id <= ?")
            .bind(conversation_id).bind(before_id)
            .execute(&self.pool).await.map_err(db_err)?;
        sqlx::query(
            "UPDATE conversations SET summary = ?, compacted_before = ?, prompt_tokens = prompt_tokens + ?, completion_tokens = completion_tokens + ?, updated_at = ? WHERE id = ?",
        )
        .bind(summary).bind(before_id).bind(pt_add).bind(ct_add).bind(now_secs()).bind(conversation_id)
        .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn add_tokens(&self, conversation_id: &str, prompt_tokens: i64, completion_tokens: i64) -> Result<(), ApiError> {
        sqlx::query(
            "UPDATE conversations SET prompt_tokens = prompt_tokens + ?, completion_tokens = completion_tokens + ?, updated_at = ? WHERE id = ?",
        )
        .bind(prompt_tokens).bind(completion_tokens).bind(now_secs()).bind(conversation_id)
        .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn reset(&self, conversation_id: &str) -> Result<(), ApiError> {
        sqlx::query("DELETE FROM conversation_messages WHERE conversation_id = ?")
            .bind(conversation_id)
            .execute(&self.pool).await.map_err(db_err)?;
        sqlx::query("UPDATE conversations SET summary = NULL, compacted_before = 0, prompt_tokens = 0, completion_tokens = 0, updated_at = ? WHERE id = ?")
            .bind(now_secs()).bind(conversation_id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }
}

// ---------- agent_sessions（M1/U1，2026-10-05）----------

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

    /// Codex reports tokens but no price, model, or cache-write count.
    pub async fn set_codex_usage(&self, id: &str, input: i64, output: i64, cached: i64) -> Result<(), ApiError> {
        sqlx::query("UPDATE agent_sessions SET input_tokens = ?, output_tokens = ?, cache_read_tokens = ?, usage_source = 'codex-turn' WHERE id = ?")
            .bind(input).bind(output).bind(cached).bind(id)
            .execute(&self.pool).await.map_err(db_err)?;
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
            "SELECT COUNT(*) AS sessions,                 SUM(CASE WHEN status='succeeded' THEN 1 ELSE 0 END) AS succeeded,                 SUM(CASE WHEN status='failed' THEN 1 ELSE 0 END) AS failed,                 SUM(CASE WHEN status='failed' THEN cost_usd ELSE 0 END) AS failed_cost,                 SUM(cost_usd) AS cost,                 SUM(CASE WHEN cost_usd IS NOT NULL THEN 1 ELSE 0 END) AS reported,                 SUM(input_tokens) AS input_tokens, SUM(output_tokens) AS output_tokens,                 SUM(cache_read_tokens) AS cache_read              FROM agent_sessions WHERE repo = ? AND started_at >= ?",
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
            "SELECT kind AS name, COUNT(*) AS sessions, SUM(cost_usd) AS cost              FROM agent_sessions WHERE repo = ? AND started_at >= ?              GROUP BY kind ORDER BY cost DESC",
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
            "SELECT model AS name, COUNT(*) AS sessions,                 SUM(COALESCE(input_tokens,0) + COALESCE(output_tokens,0)) AS tokens              FROM agent_sessions WHERE repo = ? AND started_at >= ? AND model IS NOT NULL              GROUP BY model ORDER BY tokens DESC",
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
            "SELECT module_id AS name, COUNT(*) AS sessions, SUM(cost_usd) AS cost,                 SUM(CASE WHEN status='failed' THEN 1 ELSE 0 END) AS failed              FROM agent_sessions WHERE repo = ? AND started_at >= ? AND module_id IS NOT NULL              GROUP BY module_id ORDER BY cost DESC LIMIT 10",
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

    fn now() -> String {
        format!("{:?}", std::time::SystemTime::now())
    }

    #[tokio::test]
    async fn codex_usage_keeps_unreported_cost_unknown() {
        let db = Database::connect_memory().await.unwrap();
        let repo = AgentSessionRepo::new(db.pool().clone());
        repo.upsert_started("codex", "r", "codex", "2026-10-05T08:00:00Z").await.unwrap();
        repo.set_codex_usage("codex", 100, 10, 40).await.unwrap();
        let rows = repo.list("r", 10).await.unwrap();
        let r = &rows[0];
        assert_eq!(r.input_tokens, Some(100));
        assert_eq!(r.output_tokens, Some(10));
        assert_eq!(r.cache_read_tokens, Some(40));
        assert_eq!(r.usage_source.as_deref(), Some("codex-turn"));
        assert!(r.cost_usd.is_none());
        assert!(r.cache_write_tokens.is_none());
        assert!(r.model.is_none());
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
    async fn conversation_messages_paginate() {
        // R1 清债：回放分页（先爆热区第一名的回归网）
        let db = Database::connect_memory().await.unwrap();
        let repo = SqliteConversationRepository::new(db.pool().clone());
        let conv = repo.get_or_create("demo").await.unwrap();
        for i in 0..60 {
            repo.append_message(&conv.id, "user", &format!("msg-{i}"), 1).await.unwrap();
        }
        let page1 = repo.list_messages(&conv.id, None, 50).await.unwrap();
        assert_eq!(page1.len(), 50);
        assert_eq!(page1[0].content, "msg-10", "最旧的在 60-50=10");
        let page2 = repo.list_messages(&conv.id, Some(page1[0].id), 50).await.unwrap();
        assert_eq!(page2.len(), 10, "第二页应 10 条");
        assert_eq!(page2[0].content, "msg-0");
        // 衔接：page2 最后一条 id < page1 第一条 id
        assert!(page2.last().unwrap().id < page1.first().unwrap().id);
    }

    #[tokio::test]
    async fn conversation_roundtrip_and_compaction() {
        let db = Database::connect_memory().await.unwrap();
        let repo = SqliteConversationRepository::new(db.pool().clone());
        let conv = repo.get_or_create("demo").await.unwrap();
        assert_eq!(conv.id, "chat:demo");

        let m1 = repo.append_message(&conv.id, "user", "问题一", 10).await.unwrap();
        let _m2 = repo.append_message(&conv.id, "assistant", "回答一", 20).await.unwrap();
        let m3 = repo.append_message(&conv.id, "user", "问题二", 10).await.unwrap();
        let _m4 = repo.append_message(&conv.id, "assistant", "回答二", 20).await.unwrap();

        // 未压缩水位前：运行态仅见未压缩消息
        repo.apply_compaction(&conv.id, m3 - 1, "摘要：决策 X；未决问题 Y", 100, 50).await.unwrap();
        let all = repo.list_messages(&conv.id, None, 50).await.unwrap();
        assert_eq!(all.len(), 4, "原文全部保留（留痕可回放）");
        assert!(all.iter().take(2).all(|m| m.compacted));
        assert!(!all[2].compacted, "近期窗口原文保留");
        let fresh = repo.list_uncompacted(&conv.id).await.unwrap();
        assert_eq!(fresh.len(), 2);

        // token 累计
        repo.add_tokens(&conv.id, 30, 15).await.unwrap();
        let after = repo.get_or_create("demo").await.unwrap();
        assert_eq!(after.prompt_tokens, 130);
        assert_eq!(after.completion_tokens, 65);
        assert!(after.summary.as_deref().unwrap().contains("未决问题 Y"), "摘要须携带会话状态（§11 🟡6）");

        // 重置（新对话）
        repo.reset(&conv.id).await.unwrap();
        assert!(repo.list_messages(&conv.id, None, 50).await.unwrap().is_empty());
        let clean = repo.get_or_create("demo").await.unwrap();
        assert_eq!(clean.prompt_tokens, 0);
        assert!(clean.summary.is_none());
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
                repo.finish_run(&FinishPatrolRun { id, finished_at: now(), status, arch_score: arch, error: None, prompt_tokens: None, completion_tokens: None }).await.unwrap();
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
