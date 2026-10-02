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
}

impl From<TaskRowSql> for TaskRow {
    fn from(r: TaskRowSql) -> Self {
        Self { id: r.id, repo: r.repo, title: r.title, description: r.description, modules: r.modules, acceptance: r.acceptance, source: r.source, context: r.context, status: r.status, trust: r.trust, error: r.error, session_id: r.session_id, gate: r.gate, prompt_tokens: r.prompt_tokens, completion_tokens: r.completion_tokens, result: r.result, base_head: r.base_head, created_at: r.created_at, updated_at: r.updated_at, conversation_id: r.conversation_id,}
    }
}

impl TaskRepository for SqliteTaskRepository {
    async fn create(&self, t: &TaskRow) -> Result<(), ApiError> {
        sqlx::query(
            "INSERT INTO tasks (id, repo, title, description, modules, acceptance, source, context, status, trust, session_id, gate, prompt_tokens, completion_tokens, result, created_at, updated_at, conversation_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&t.id).bind(&t.repo).bind(&t.title).bind(&t.description).bind(&t.modules)
        .bind(&t.acceptance).bind(&t.source).bind(&t.context).bind(&t.status).bind(&t.trust)
        .bind(&t.session_id).bind(&t.gate).bind(t.prompt_tokens).bind(t.completion_tokens).bind(&t.result).bind(&t.created_at).bind(&t.updated_at).bind(&t.conversation_id)
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
            .bind(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string())
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
        .bind(now_secs())
        .execute(&self.pool).await.map_err(db_err)?;
        Ok(res.rows_affected())
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
            conversation_id: None,
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
