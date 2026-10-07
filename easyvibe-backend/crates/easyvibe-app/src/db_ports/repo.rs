//! 仓库维护域端口适配（c-arch-13 R1）。

use easyvibe_common::ApiError;

/// 注销仓库的跨表清除（原 `easyvibe_db::wipe_repo(&st.pool, id)`）。
#[async_trait::async_trait]
pub(crate) trait RepoMaintenancePort {
    async fn wipe(&self, repo: &str) -> Result<u64, ApiError>;
}

#[async_trait::async_trait]
impl RepoMaintenancePort for easyvibe_db::sqlx::SqlitePool {
    async fn wipe(&self, repo: &str) -> Result<u64, ApiError> {
        easyvibe_db::wipe_repo(self, repo).await
    }
}
