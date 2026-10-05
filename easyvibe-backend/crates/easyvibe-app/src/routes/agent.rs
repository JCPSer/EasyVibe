//! 资源域：agent 探测/测试与 LLM 连通性测试。

use crate::state::*;
use easyvibe_db::SettingsRepository as _;
use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Json,
};
use easyvibe_common::ApiError;
use easyvibe_ai_agent::agent_conf;
use tracing::info;

/// M1 配置体系：agent 状态面（探测 + 生效配置 + 预设目录 + 最近测试结果）
/// 预设目录由后端唯一提供——前端单选组不硬编码（初审修订点）
pub(crate) async fn agent_status(State(st): State<AppState>) -> Result<Response, AppError> {
    let detected = st.agent_detected.read().await.clone();
    Ok(agent_status_json(&st, detected).await.into_response())
}

pub(crate) async fn agent_detect(State(st): State<AppState>) -> Result<Response, AppError> {
    let detected = agent_conf::detect_agents().await;
    *st.agent_detected.write().await = detected.clone();
    info!("[agent] 手动重探完成：检测到 {} 个", detected.len());
    Ok(agent_status_json(&st, detected).await.into_response())
}

pub(crate) async fn agent_test(State(st): State<AppState>) -> Result<Response, AppError> {
    let _guard = st.agent_test_lock.lock().await;
    let resolved = agent_conf::resolve_agent(&st.settings_repo, None, &st.agent_command, &st.agent_args).await;
    info!("[agent] 测试连接：{} {:?}", resolved.command, resolved.args);
    let result = agent_conf::run_agent_test(&st.session_manager, &resolved).await;
    *st.agent_test.write().await = Some(result.clone());
    Ok(Json(serde_json::json!({ "success": true, "data": result })).into_response())
}

/// 模型服务连通性测试（设置面板"测试连接"按钮，2026-10-04 审计 P2 补口）：
/// 优先用请求体现填值（未保存也能测），缺省回落全局已存配置；max_tokens=1 的 ping，代价可忽略。
/// 认证头双发（与 AnthropicClient 同一纪律：x-api-key + Bearer 并存）。
#[derive(serde::Deserialize)]
pub(crate) struct LlmTestBody {
    pub(crate) service_id: String,
    pub(crate) base_url: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) api_key: Option<String>,
}

pub(crate) async fn llm_test_inner(st: &AppState, body: LlmTestBody) -> Result<serde_json::Value, ApiError> {
    let base = st.settings_repo.get("global", &format!("llm.service.{}", body.service_id)).await.ok().flatten()
        .and_then(|r| serde_json::from_str::<serde_json::Value>(&r.value).ok());
    let stored_key = st.settings_repo.get("global", &format!("llm.service.{}.apiKey", body.service_id)).await.ok().flatten()
        .and_then(|r| if r.encrypted { st.cipher.decrypt(&r.value).ok() } else { Some(r.value) })
        .and_then(|v| serde_json::from_str::<String>(&v).ok());
    if base.is_none() && body.base_url.as_deref().map(str::trim).filter(|s| !s.is_empty()).is_none() {
        return Err(ApiError::NotFound(format!("服务 {} 不存在（或未填 Base URL）", body.service_id)));
    }
    let obj = base.unwrap_or_default();
    let base_url = body.base_url.filter(|s| !s.trim().is_empty())
        .or_else(|| obj.get("baseUrl").and_then(|v| v.as_str()).map(Into::into))
        .unwrap_or_else(|| "https://api.anthropic.com".into());
    let model = body.model.filter(|s| !s.trim().is_empty())
        .or_else(|| obj.get("model").and_then(|v| v.as_str()).map(Into::into))
        .unwrap_or_else(|| "claude-sonnet-4-5".into());
    let api_key = body.api_key.filter(|s| !s.trim().is_empty()).or(stored_key).unwrap_or_default();
    let started = std::time::Instant::now();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| ApiError::Internal(format!("HTTP client 构建失败: {e}")))?;
    let mut req = client
        .post(format!("{}/v1/messages", base_url.trim_end_matches('/')))
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01");
    if !api_key.is_empty() {
        req = req.header("authorization", format!("Bearer {}", api_key));
    }
    let resp = req
        .json(&serde_json::json!({
            "model": model,
            "max_tokens": 1,
            "messages": [{ "role": "user", "content": "ping" }],
        }))
        .send()
        .await;
    let latency = started.elapsed().as_millis() as u64;
    match resp {
        Ok(r) if r.status().is_success() => Ok(serde_json::json!({
            "ok": true, "latencyMs": latency, "protocol": format!("{} · {}", model, base_url),
        })),
        Ok(r) => {
            let status = r.status().as_u16();
            let snippet: String = r.text().await.unwrap_or_default().chars().take(200).collect();
            Ok(serde_json::json!({
                "ok": false, "latencyMs": latency, "protocol": format!("HTTP {status}"), "error": snippet,
            }))
        }
        Err(e) => Ok(serde_json::json!({
            "ok": false, "latencyMs": latency, "protocol": "网络层", "error": e.to_string(),
        })),
    }
}

pub(crate) async fn llm_test(State(st): State<AppState>, Json(body): Json<LlmTestBody>) -> Result<Response, AppError> {
    Ok(Json(serde_json::json!({ "success": true, "data": llm_test_inner(&st, body).await? })).into_response())
}

/// status/detect 共用的响应体（探测结果传入——detect 用新鲜值，status 用内存态）
pub(crate) async fn agent_status_json(st: &AppState, detected: Vec<agent_conf::DetectedAgent>) -> axum::response::Response {
    let resolved = agent_conf::resolve_agent(&st.settings_repo, None, &st.agent_command, &st.agent_args).await;
    let found = std::path::Path::new(&resolved.command).is_file();
    let preset = cfg_get(st, "agent.preset").await.unwrap_or(serde_json::Value::String("claude".into()));
    let configured_args = cfg_get(st, "agent.args.global").await
        .and_then(|v| v.as_str().map(str::to_string))
        .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok());
    // 槽位参数（整体替换语义）——配置原样回显给设置面板
    let args_task = cfg_get(st, "agent.args.task").await
        .and_then(|v| v.as_str().map(str::to_string))
        .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok());
    let args_review = cfg_get(st, "agent.args.review").await
        .and_then(|v| v.as_str().map(str::to_string))
        .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok());
    let test = st.agent_test.read().await.clone();
    let protocol_ok = test.as_ref().map(|t| t.ok);
    Json(serde_json::json!({
        "success": true,
        "data": {
            "detected": detected,
            "configured": {
                "command": cfg_get(st, "agent.command").await,
                "args": configured_args,
                "argsTask": args_task,
                "argsReview": args_review,
                "preset": preset,
                "type": cfg_get(st, "agent.type").await,
            },
            "effective": {
                "command": resolved.command,
                "args": resolved.args,
                "type": resolved.agent_type,
                "source": resolved.source,
                "found": found,
            },
            "presets": agent_conf::PRESETS.iter().map(|p| serde_json::json!({
                "id": p.id, "label": p.label, "defaultArgs": p.default_args,
                "type": p.agent_type, "stability": p.stability,
            })).collect::<Vec<_>>(),
            "protocolOk": protocol_ok,
            "lastTest": test,
        }
    }))
    .into_response()
}

/// 读取一条 agent 相关设置（响应体组装用）
pub(crate) async fn cfg_get(st: &AppState, key: &str) -> Option<serde_json::Value> {
    use easyvibe_db::SettingsRepository as _;
    st.settings_repo.get("global", key).await.ok().flatten().map(|r| serde_json::Value::String(r.value))
}
