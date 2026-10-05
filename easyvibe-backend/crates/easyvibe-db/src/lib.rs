//! 域 2（IDE 自有状态）的 SQLite 数据层：迁移 + Repository 门面。
//! 形状照抄 AionCore：Service 只依赖 trait；测试用内存库。
//!
//! 结构调整（2026-10-05）：按聚合域拆子模块（health/settings/task/approval/
//! conversation/event/agent/session_output），本文件只保留 Database 连接池、
//! 迁移入口与对外符号 re-export。
use easyvibe_common::ApiError;
use sqlx::SqlitePool;

pub use sqlx;

pub mod core;
pub mod health;
pub mod settings;
pub mod task;
pub mod approval;
pub mod conversation;
pub mod event;
pub mod agent;
pub mod session_output;

use core::db_err;

pub use core::wipe_repo;
pub use agent::{AgentSessionRepo, AgentSessionRow, UsageDailyRow, UsageGroupRow, UsageModuleRow, UsageTotalsRow};
pub use approval::{ApprovalRepository, ApprovalRow, SqliteApprovalRepository};
pub use conversation::{ConversationMessageRow, ConversationRepository, ConversationRow, SqliteConversationRepository};
pub use event::{EventCountRow, EventRepository, SqliteEventRepository};
pub use health::{FinishPatrolRun, HealthRepository, ModuleHealthRow, NewPatrolRun, PatrolRunRow, RunModuleAvg, SqliteHealthRepository};
pub use session_output::{SessionOutputRepo, SessionOutputRow};
pub use settings::{SettingRow, SettingsRepository, SqliteSettingsRepository};
pub use task::{SqliteTaskRepository, TaskRepository, TaskRow};

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
