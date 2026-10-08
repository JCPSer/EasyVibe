//! 装配格端口适配器（c-arch-16 R2/R3）：整个 app crate 内 `easyvibe_git::` /
//! `easyvibe_pipeline::` 字面量的**唯一**落点。
//!
//! 边界层（server-api）只见 `crate::service::{git::GitPort, repo::RepoPipelinePort}` 端口 trait
//! 与 `AppState` 上的 `Arc<dyn …>` 字段；具体领域 crate 的调用在此收口。依赖方向仍为
//! `assembly → server-api`（同层单向）：端口在依赖方（server-api）、实现在组合根（本格），
//! 与 `impl QueueHost for AppState` / `db_ports` 家族同构，只是此处适配器**零状态**。

use crate::service::git::GitPort;
use crate::service::repo::RepoPipelinePort;
use easyvibe_common::ApiError;
use easyvibe_event_bus::BusEvent;
use easyvibe_map::{MapService, Repo};
use easyvibe_session::SessionManager;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::broadcast;

/// 领域模型 → 响应 `Value`（c-arch-16 R2）：序列化由适配器承担，端口面因此零领域类型。
/// 错误信封与原 `service::git::to_value` 逐字等价（`ApiError::Internal` + 同一文案）。
fn to_value<T: serde::Serialize>(v: T) -> Result<Value, ApiError> {
    serde_json::to_value(v).map_err(|e| ApiError::Internal(format!("响应序列化失败: {e}")))
}

/// git 域适配器（零状态）：easyvibe-git crate 9 个纯函数逐条透传；
/// 读操作序列化为 `Value`，写操作原样返回（行为与改造前调用点逐条等价）。
pub(crate) struct GitAdapter;

#[async_trait::async_trait]
impl GitPort for GitAdapter {
    async fn status(&self, root: &Path) -> Result<Value, ApiError> {
        to_value(easyvibe_git::status(root).await?)
    }

    async fn log(&self, root: &Path, limit: i64) -> Result<Value, ApiError> {
        to_value(easyvibe_git::log(root, limit).await?)
    }

    async fn show_commit(&self, root: &Path, hash: &str) -> Result<Value, ApiError> {
        to_value(easyvibe_git::show_commit(root, hash).await?)
    }

    async fn diff(&self, root: &Path, path: &str, staged: bool) -> Result<Value, ApiError> {
        to_value(easyvibe_git::diff(root, path, staged).await?)
    }

    async fn commit_all(&self, root: &Path, message: &str) -> Result<String, ApiError> {
        easyvibe_git::commit_all(root, message).await
    }

    async fn pull(&self, root: &Path) -> Result<(), ApiError> {
        easyvibe_git::pull(root).await
    }

    async fn push(&self, root: &Path) -> Result<(), ApiError> {
        easyvibe_git::push(root).await
    }

    async fn discard(&self, root: &Path, path: &str) -> Result<(), ApiError> {
        easyvibe_git::discard(root, path).await
    }

    async fn discard_all(&self, root: &Path) -> Result<(), ApiError> {
        easyvibe_git::discard_all(root).await
    }
}

/// 管线适配器（零状态）：easyvibe-pipeline crate 的 `spawn_repo_pipeline` 唯一落点。
/// 语义与现行 `tokio::spawn(spawn_repo_pipeline(...))` 一致（spawn 即返回、无返回值）。
pub(crate) struct PipelineAdapter;

#[async_trait::async_trait]
impl RepoPipelinePort for PipelineAdapter {
    async fn spawn(
        &self,
        repo: Repo,
        map_service: Arc<MapService>,
        event_bus: broadcast::Sender<BusEvent>,
        session_manager: Arc<SessionManager>,
        prompt_template: String,
        auto_init_suffix: String,
        agent_command: String,
        agent_args: Vec<String>,
    ) {
        tokio::spawn(easyvibe_pipeline::spawn_repo_pipeline(
            repo,
            map_service,
            event_bus,
            session_manager,
            prompt_template,
            auto_init_suffix,
            agent_command,
            agent_args,
        ));
    }
}
