//! 资源域：agent 探测/测试与 LLM 连通性测试。
//!
//! c-arch-7 R1：业务编排与设置读取已下沉 `crate::service::agent`，本文件只做 HTTP 边界。

use crate::state::*;
use axum::{
    extract::State,
    routing::{get, post},
    response::{IntoResponse, Response},
    Json, Router,
};
use crate::service::agent::LlmTestBody;

/// 本域路由（R1 自注册）：agent 探测/测试与 LLM 连通性测试。
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/agent/status", get(agent_status))
        .route("/agent/detect", post(agent_detect))
        .route("/agent/test", post(agent_test))
        .route("/llm/test", post(llm_test))
}

/// M1 配置体系：agent 状态面（探测 + 生效配置 + 预设目录 + 最近测试结果）
pub(crate) async fn agent_status(State(st): State<AppState>) -> Result<Response, AppError> {
    let detected = st.agent_detected.read().await.clone();
    Ok(Json(crate::service::agent::agent_status(&st, detected).await?).into_response())
}

pub(crate) async fn agent_detect(State(st): State<AppState>) -> Result<Response, AppError> {
    Ok(Json(crate::service::agent::agent_detect(&st).await?).into_response())
}

pub(crate) async fn agent_test(State(st): State<AppState>) -> Result<Response, AppError> {
    Ok(Json(crate::service::agent::agent_test(&st).await?).into_response())
}

/// 模型服务连通性测试（设置面板"测试连接"按钮）：编排在 `service::agent::llm_test_inner`。
pub(crate) async fn llm_test(State(st): State<AppState>, Json(body): Json<LlmTestBody>) -> Result<Response, AppError> {
    Ok(Json(crate::service::agent::llm_test_inner(&st, body).await?).into_response())
}
