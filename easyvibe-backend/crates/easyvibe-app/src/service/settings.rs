//! settings 域编排（c-arch-7 R1 新增）：设置/密钥读写与诊断导出的业务编排。
//!
//! 自 `routes/settings.rs` 原样搬迁（零语义改动）——routes 只保留 HTTP 边界；
//! harness 自定义槽端点仍留 routes（纯 FS + harness 装载，不触库）。

use crate::db_ports::{HealthPort as _, SettingsPort as _, TaskPort as _};
use crate::state::*;
use easyvibe_common::ApiError;
use crate::VERSION;

/// 敏感 key 规则：以 .apiKey / apiKey 结尾自动加密 at rest。
pub(crate) fn is_sensitive_key(key: &str) -> bool {
    key.ends_with(".apiKey") || key.ends_with("apiKey")
}

/// 设置列表（scope 缺省 global；加密值解密回显供 UI 编辑）。
pub(crate) async fn list_settings(st: &AppState, scope: &str) -> Result<serde_json::Value, ApiError> {
    let rows = st.settings_repo.list(scope).await?;
    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            let value = if r.encrypted {
                // 本地单机：解密返回供 UI 编辑（网络传输仅限 127.0.0.1）
                st.cipher.decrypt(&r.value).unwrap_or_default()
            } else {
                r.value
            };
            serde_json::json!({
                "key": r.key,
                "value": serde_json::from_str::<serde_json::Value>(&value).unwrap_or(serde_json::Value::String(value)),
                "encrypted": r.encrypted,
                "updatedAt": r.updated_at,
            })
        })
        .collect();
    Ok(serde_json::json!({ "success": true, "data": items }))
}

/// 写设置（scope/key 合法性校验 → 敏感 key 加密 → 落库）。
pub(crate) async fn put_setting(st: &AppState, scope: &str, key: &str, value: serde_json::Value) -> Result<(), ApiError> {
    if scope.is_empty() || key.is_empty() || key.contains('/') || key.contains("..") {
        return Err(ApiError::BadRequest("非法 scope/key".into()));
    }
    let sensitive = is_sensitive_key(key);
    let raw = serde_json::to_string(&value).map_err(|e| ApiError::Internal(e.to_string()))?;
    let (value, encrypted) = if sensitive {
        (st.cipher.encrypt(&raw)?, true)
    } else {
        (raw, false)
    };
    let updated_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string();
    st.settings_repo.set(scope, key, value, encrypted, updated_at).await?;
    Ok(())
}

/// 删除设置。
pub(crate) async fn delete_setting(st: &AppState, scope: &str, key: &str) -> Result<(), ApiError> {
    st.settings_repo.delete(scope, key).await?;
    Ok(())
}

/// Y3：一键诊断导出——把"用户报障口头描述"变成"导出一个文件"
/// 最近 200 行日志 + 后端版本 + 各表计数（settings 的加密值剔除）。
pub(crate) async fn export_diagnostics(st: &AppState) -> Result<serde_json::Value, ApiError> {
    let dir = data_dir().to_string_lossy().into_owned();
    let log_path = format!("{dir}/logs/easyvibe.log");
    let logs = std::fs::read_to_string(&log_path)
        .map(|t| t.lines().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"))
        .unwrap_or_else(|_| "（日志文件不可读）".into());
    let tasks = st.task_repo.list(st.map_service.repos().await.first().map(|r| r.id.as_str()).unwrap_or(""), 100).await.unwrap_or_default();
    let runs = st.health_repo.list_runs(st.map_service.repos().await.first().map(|r| r.id.as_str()).unwrap_or(""), 20).await.unwrap_or_default();
    let aps = tasks.iter().take(10).map(|t| t.id.clone()).collect::<Vec<_>>();
    let body = serde_json::json!({
        "version": VERSION,
        "llm_mode": format!("{:?}", *st.llm_mode),
        "repos": st.map_service.repos().await.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
        "task_counts": {
            "by_status": tasks.iter().fold(serde_json::json!({}), |mut acc, t| {
                let k = t.status.clone();
                let o = acc.as_object_mut().unwrap();
                *o.entry(k).or_insert(serde_json::json!(0)) = serde_json::json!(o.get(&t.status).and_then(|v| v.as_i64()).unwrap_or(0) + 1);
                acc
            }),
        },
        "recent_runs": runs.iter().take(5).map(|r| serde_json::json!({"id": r.id, "status": r.status, "archScore": r.arch_score})).collect::<Vec<_>>(),
        "recent_logs": logs,
    });
    let _ = aps;
    Ok(serde_json::json!({ "success": true, "data": body }))
}
