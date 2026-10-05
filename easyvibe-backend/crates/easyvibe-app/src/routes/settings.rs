//! 资源域：设置/密钥/harness 管理与诊断导出。

use crate::state::*;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use easyvibe_common::ApiError;
use tracing::info;
use crate::VERSION;
use crate::task_exec;
use easyvibe_db::{SettingRow, SettingsRepository as _};

/// 敏感 key 规则：以 .apiKey / apiKey 结尾自动加密 at rest
pub(crate) fn is_sensitive_key(key: &str) -> bool {
    key.ends_with(".apiKey") || key.ends_with("apiKey")
}

pub(crate) async fn list_settings(State(st): State<AppState>, axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> Result<Response, AppError> {
    let scope = q.get("scope").cloned().unwrap_or_else(|| "global".into());
    let rows = st.settings_repo.list(&scope).await?;
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
    Ok(Json(serde_json::json!({ "success": true, "data": items })).into_response())
}

#[derive(serde::Deserialize)]
pub(crate) struct PutSettingRequest {
    scope: String,
    key: String,
    value: serde_json::Value,
}

pub(crate) async fn put_setting(State(st): State<AppState>, Json(body): Json<PutSettingRequest>) -> Result<Response, AppError> {
    if body.scope.is_empty() || body.key.is_empty() || body.key.contains('/') || body.key.contains("..") {
        return Err(AppError(ApiError::BadRequest("非法 scope/key".into())));
    }
    let sensitive = is_sensitive_key(&body.key);
    let raw = serde_json::to_string(&body.value).map_err(|e| AppError(ApiError::Internal(e.to_string())))?;
    let (value, encrypted) = if sensitive {
        (st.cipher.encrypt(&raw)?, true)
    } else {
        (raw, false)
    };
    st.settings_repo
        .set(&SettingRow {
            scope: body.scope,
            key: body.key,
            value,
            encrypted,
            updated_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string(),
        })
        .await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

pub(crate) async fn delete_setting(State(st): State<AppState>, Path((scope, key)): Path<(String, String)>) -> Result<Response, AppError> {
    st.settings_repo.delete(&scope, &key).await?;
    Ok(Json(serde_json::json!({ "success": true })).into_response())
}

/// Y3：一键诊断导出——把"用户报障口头描述"变成"导出一个文件"
/// 最近 200 行日志 + 后端版本 + 各表计数（settings 的加密值剔除）
pub(crate) async fn export_diagnostics(State(st): State<AppState>) -> Result<Response, AppError> {
    let dir = data_dir().to_string_lossy().into_owned();
    let log_path = format!("{dir}/logs/easyvibe.log");
    let logs = std::fs::read_to_string(&log_path)
        .map(|t| t.lines().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"))
        .unwrap_or_else(|_| "（日志文件不可读）".into());
    use easyvibe_db::{HealthRepository as _, TaskRepository as _};
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
    Ok(Json(serde_json::json!({ "success": true, "data": body })).into_response())
}

/// S1-3：harness 状态（manifest + 文件清单）——S3 管理界面的数据面
pub(crate) async fn get_harness(State(st): State<AppState>) -> Result<Response, AppError> {
    let h = st.harness.read().await;
    let mut files: Vec<String> = vec![];
    if let Ok(entries) = std::fs::read_dir(&h.dir) {
        for e in entries.flatten() {
            if e.path().is_file() {
                files.push(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    files.sort();
    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "dir": h.dir.to_string_lossy(),
            "manifest": {
                "id": h.manifest.id, "version": h.manifest.version, "builtin": h.manifest.builtin,
                "routeRules": h.manifest.route_rules,
                "skills": { "userEntry": h.manifest.skills.user_entry, "transparent": h.manifest.skills.transparent },
            },
            "frameworkNeutralized": h.framework_transparent.contains("透明执行模式"),
            "userEntrySkillCount": h.user_entry_skills.len(),
            "files": files,
        }
    }))
    .into_response())
}

/// S1-3：恢复默认——现有用户层整体改名备份（.backup-<ts>），出厂底账全量重铺，
/// 装载后热换单一事实源（chat 与 executor 立即生效，无需重启）
pub(crate) async fn reset_harness(State(st): State<AppState>) -> Result<Response, AppError> {
    let dir = task_exec::harness_dir();
    if dir.exists() {
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let backup = dir.with_file_name(format!("harness.backup-{ts}"));
        std::fs::rename(&dir, &backup).map_err(|e| ApiError::Internal(format!("harness 备份失败: {e}")))?;
    }
    task_exec::deploy_builtin_force(&dir)?;
    let fresh = task_exec::load_harness()?;
    let version = fresh.manifest.version.clone();
    *st.harness.write().await = fresh;
    info!("[harness] 已恢复默认 v{}", version);
    Ok(Json(serde_json::json!({ "success": true, "data": { "version": version } })).into_response())
}

/// 规则文件可编辑化（2026-10-04：Harness 板块编辑能力）——列出 harness 目录全部文件（递归，相对路径）。
pub(crate) async fn list_harness_files() -> Result<Response, AppError> {
    let dir = task_exec::harness_dir();
    let mut files: Vec<serde_json::Value> = vec![];
    fn walk(base: &std::path::Path, cur: &std::path::Path, out: &mut Vec<serde_json::Value>) {
        if let Ok(rd) = std::fs::read_dir(cur) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() { walk(base, &p, out) } else if let Ok(rel) = p.strip_prefix(base) {
                    out.push(serde_json::json!({ "path": rel.to_string_lossy() }));
                }
            }
        }
    }
    walk(&dir, &dir, &mut files);
    files.sort_by_key(|f| f["path"].as_str().unwrap_or_default().to_string());
    Ok(Json(serde_json::json!({ "success": true, "data": { "files": files } })).into_response())
}

/// harness 文件读取：canonicalize 越界防线（与 dev-doc 同纪律），只收 harness 目录内文件。
pub(crate) async fn get_harness_file(
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let rel = q.get("path").cloned().unwrap_or_default();
    let dir = task_exec::harness_dir();
    let full = dir.join(&rel);
    let (Ok(canonical), Ok(dir_canon)) = (full.canonicalize(), dir.canonicalize()) else {
        return Err(AppError(ApiError::NotFound("文件不存在".into())));
    };
    if !canonical.starts_with(&dir_canon) || !canonical.is_file() {
        return Err(AppError(ApiError::NotFound("文件不存在（路径越界或非文件）".into())));
    }
    let content = std::fs::read_to_string(&canonical).map_err(|e| ApiError::Internal(format!("文件读取失败: {e}")))?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "path": rel, "content": content } })).into_response())
}

/// harness 文件写入：同防线；写后热换单一事实源（chat 与 executor 立即生效，与 reset 同纪律）。
pub(crate) async fn put_harness_file(
    State(st): State<AppState>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let rel = body.get("path").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let content = body.get("content").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    if rel.is_empty() {
        return Err(AppError(ApiError::BadRequest("path 不能为空".into())));
    }
    let dir = task_exec::harness_dir();
    let full = dir.join(&rel);
    let dir_canon = dir.canonicalize().map_err(|e| ApiError::Internal(format!("harness 目录不可读: {e}")))?;
    // 已存在：canonicalize 校验越界；不存在：校验相对路径本身不带越界段
    if full.exists() {
        let canonical = full.canonicalize().map_err(|_| ApiError::NotFound("文件不存在".into()))?;
        if !canonical.starts_with(&dir_canon) || !canonical.is_file() {
            return Err(AppError(ApiError::NotFound("文件不存在（路径越界或非文件）".into())));
        }
    } else if rel.split('/').any(|s| s == ".." || s.is_empty()) || rel.starts_with('/') {
        return Err(AppError(ApiError::BadRequest("路径越界".into())));
    }
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ApiError::Internal(format!("目录创建失败: {e}")))?;
    }
    std::fs::write(&full, content).map_err(|e| ApiError::Internal(format!("文件写入失败: {e}")))?;
    let fresh = task_exec::load_harness()?;
    let version = fresh.manifest.version.clone();
    *st.harness.write().await = fresh;
    info!("[harness] 文件 {} 已更新并热装载（v{}）", rel, version);
    Ok(Json(serde_json::json!({ "success": true, "data": { "path": rel, "version": version } })).into_response())
}

/// 备份列表（2026-10-04 审计 P1：reset 有备份无恢复入口的另一半）：
/// harness 目录的兄弟目录中凡是 harness.backup-* 的都算备份，按名倒序（新在前）。
pub(crate) async fn list_harness_backups() -> Result<Response, AppError> {
    let dir = task_exec::harness_dir();
    let Some(parent) = dir.parent() else { return Ok(Json(serde_json::json!({ "success": true, "data": { "backups": [] } })).into_response()) };
    let mut backups: Vec<serde_json::Value> = vec![];
    if let Ok(rd) = std::fs::read_dir(parent) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with("harness.backup-") || !e.path().is_dir() { continue }
            let mtime = std::fs::metadata(e.path()).ok().and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0);
            backups.push(serde_json::json!({ "name": name, "mtimeMs": mtime }));
        }
    }
    backups.sort_by(|a, b| b["mtimeMs"].as_u64().cmp(&a["mtimeMs"].as_u64()));
    Ok(Json(serde_json::json!({ "success": true, "data": { "backups": backups } })).into_response())
}

/// 从备份恢复：当前层改名留档（pre-restore-*，防"恢复错了想反悔"无解），指定备份转正，热换。
pub(crate) async fn restore_harness(
    State(st): State<AppState>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let name = body.get("backup").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    // 名字防线：只允许 harness.backup-*（拒绝 ../ 等路径游戏）
    if !name.starts_with("harness.backup-") || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(AppError(ApiError::BadRequest("非法备份名".into())));
    }
    let dir = task_exec::harness_dir();
    let backup = dir.with_file_name(&name);
    if !backup.is_dir() {
        return Err(AppError(ApiError::NotFound(format!("备份 {name} 不存在"))));
    }
    if dir.exists() {
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let stash = dir.with_file_name(format!("harness.pre-restore-{ts}"));
        std::fs::rename(&dir, &stash).map_err(|e| ApiError::Internal(format!("当前层留档失败: {e}")))?;
    }
    std::fs::rename(&backup, &dir).map_err(|e| ApiError::Internal(format!("恢复失败: {e}")))?;
    let fresh = task_exec::load_harness()?;
    let version = fresh.manifest.version.clone();
    *st.harness.write().await = fresh;
    info!("[harness] 已从备份 {} 恢复（v{}）", name, version);
    Ok(Json(serde_json::json!({ "success": true, "data": { "version": version, "restoredFrom": name } })).into_response())
}

// ---------- M3-2：指哪打哪——任务创建（上下文已组织好随表单提交；执行引擎 M3-3 接入） ----------
