//! 巡检健康域端口适配（c-arch-13 R1）。

use easyvibe_common::ApiError;

#[async_trait::async_trait]
pub(crate) trait HealthPort {
    async fn list_runs(&self, repo: &str, limit: i64) -> Result<Vec<easyvibe_db::PatrolRunRow>, ApiError>;
    async fn list_run_averages(&self, repo: &str, limit: i64) -> Result<Vec<easyvibe_db::RunModuleAvg>, ApiError>;
    async fn list_latest_run_modules(&self, repo: &str) -> Result<Vec<easyvibe_db::ModuleHealthRow>, ApiError>;
    async fn list_module_history(&self, repo: &str, module_id: &str, limit: i64) -> Result<Vec<easyvibe_db::ModuleHealthRow>, ApiError>;
    async fn prune_runs(&self, repo: &str, keep: i64) -> Result<u64, ApiError>;
}

#[async_trait::async_trait]
impl HealthPort for easyvibe_db::SqliteHealthRepository {
    async fn list_runs(&self, repo: &str, limit: i64) -> Result<Vec<easyvibe_db::PatrolRunRow>, ApiError> {
        easyvibe_db::HealthRepository::list_runs(self, repo, limit).await
    }
    async fn list_run_averages(&self, repo: &str, limit: i64) -> Result<Vec<easyvibe_db::RunModuleAvg>, ApiError> {
        easyvibe_db::HealthRepository::list_run_averages(self, repo, limit).await
    }
    async fn list_latest_run_modules(&self, repo: &str) -> Result<Vec<easyvibe_db::ModuleHealthRow>, ApiError> {
        easyvibe_db::HealthRepository::list_latest_run_modules(self, repo).await
    }
    async fn list_module_history(&self, repo: &str, module_id: &str, limit: i64) -> Result<Vec<easyvibe_db::ModuleHealthRow>, ApiError> {
        easyvibe_db::HealthRepository::list_module_history(self, repo, module_id, limit).await
    }
    async fn prune_runs(&self, repo: &str, keep: i64) -> Result<u64, ApiError> {
        easyvibe_db::HealthRepository::prune_runs(self, repo, keep).await
    }
}
