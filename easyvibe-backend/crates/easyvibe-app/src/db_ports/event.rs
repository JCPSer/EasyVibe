//! 事件域端口适配（c-arch-13 R1）。

use easyvibe_common::ApiError;

#[async_trait::async_trait]
pub(crate) trait EventPort {
    async fn record(&self, repo: &str, name: &str, payload: &str) -> Result<(), ApiError>;
    async fn summary(&self, repo: &str) -> Result<Vec<easyvibe_db::EventCountRow>, ApiError>;
}

#[async_trait::async_trait]
impl EventPort for easyvibe_db::SqliteEventRepository {
    async fn record(&self, repo: &str, name: &str, payload: &str) -> Result<(), ApiError> {
        easyvibe_db::EventRepository::record(self, repo, name, payload).await
    }
    async fn summary(&self, repo: &str) -> Result<Vec<easyvibe_db::EventCountRow>, ApiError> {
        easyvibe_db::EventRepository::summary(self, repo).await
    }
}
