//! 设置域端口适配（c-arch-13 R1）。

use super::dto::SettingValue;
use easyvibe_common::ApiError;

#[async_trait::async_trait]
pub(crate) trait SettingsPort {
    async fn get(&self, scope: &str, key: &str) -> Result<Option<SettingValue>, ApiError>;
    async fn list(&self, scope: &str) -> Result<Vec<SettingValue>, ApiError>;
    async fn set(&self, scope: &str, key: &str, value: String, encrypted: bool, updated_at: String) -> Result<(), ApiError>;
    async fn delete(&self, scope: &str, key: &str) -> Result<(), ApiError>;
}

#[async_trait::async_trait]
impl SettingsPort for easyvibe_db::SqliteSettingsRepository {
    async fn get(&self, scope: &str, key: &str) -> Result<Option<SettingValue>, ApiError> {
        Ok(easyvibe_db::SettingsRepository::get(self, scope, key)
            .await?
            .map(|r| SettingValue { scope: r.scope, key: r.key, value: r.value, encrypted: r.encrypted, updated_at: r.updated_at }))
    }
    async fn list(&self, scope: &str) -> Result<Vec<SettingValue>, ApiError> {
        Ok(easyvibe_db::SettingsRepository::list(self, scope)
            .await?
            .into_iter()
            .map(|r| SettingValue { scope: r.scope, key: r.key, value: r.value, encrypted: r.encrypted, updated_at: r.updated_at })
            .collect())
    }
    async fn set(&self, scope: &str, key: &str, value: String, encrypted: bool, updated_at: String) -> Result<(), ApiError> {
        let row = easyvibe_db::SettingRow { scope: scope.to_string(), key: key.to_string(), value, encrypted, updated_at };
        easyvibe_db::SettingsRepository::set(self, &row).await
    }
    async fn delete(&self, scope: &str, key: &str) -> Result<(), ApiError> {
        easyvibe_db::SettingsRepository::delete(self, scope, key).await
    }
}
