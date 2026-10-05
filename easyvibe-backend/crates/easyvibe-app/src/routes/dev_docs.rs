//! 资源域：development_docs 产物读写删。

use crate::state::*;
use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use easyvibe_common::ApiError;

/// 任务产物文档（方案 v3 §4.4）：扫描 .easyvibe/development_docs/**，按任务时间窗过滤。
/// 右端规则（复审意见落地）：running/pending 任务用 now()——updatedAt 在 running 期间
/// 不刷新，用它当右端会把执行期写出的文档全部漏掉；终态用 updatedAt+10min。
/// excerpt 按 char 边界截 200 字（禁按字节切多字节字符）；*.json 归档与索引类
/// （INDEX-/MEMORY-/operation-）单独归类，不进文档卡。
pub(crate) async fn get_dev_docs(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    use easyvibe_db::TaskRepository as _;
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let tid = q.get("taskId").cloned().unwrap_or_default();
    let task = st.task_repo.get(&tid).await?.ok_or_else(|| ApiError::NotFound(format!("任务 {tid} 不存在")))?;
    let docs_root = repo.root.join(".easyvibe/development_docs");
    if !docs_root.is_dir() {
        return Ok(Json(serde_json::json!({ "success": true, "data": { "docs": [], "indices": [] } })).into_response());
    }
    // 任务时间戳是 epoch 毫秒串（与 toMs/try_advance_gate 同一口径）
    let parse_ms = |s: &str| s.parse::<i64>().ok().unwrap_or(0);
    let left = parse_ms(&task.created_at) - 10 * 60_000;
    let right = if task.status == "running" || task.status == "pending" {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(i64::MAX)
    } else {
        parse_ms(&task.updated_at) + 10 * 60_000
    };
    let mut docs: Vec<serde_json::Value> = vec![];
    let mut indices: Vec<serde_json::Value> = vec![];
    let mut stack: Vec<std::path::PathBuf> = vec![docs_root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let Some(name) = p.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            if !name.ends_with(".md") {
                continue; // *.json 任务归档与 harness 文档混处（task_exec.rs collect），不进文档卡
            }
            let mtime_ms = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            if mtime_ms < left || mtime_ms > right {
                continue;
            }
            let rel = p.strip_prefix(&repo.root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            let is_index = name.starts_with("INDEX-") || name.starts_with("MEMORY-") || name.starts_with("operation-");
            let item = serde_json::json!({ "name": name, "path": rel, "mtime": mtime_ms });
            if is_index {
                indices.push(item);
                continue;
            }
            let excerpt = std::fs::read_to_string(&p)
                .map(|s| s.chars().take(200).collect::<String>())
                .unwrap_or_default();
            docs.push(serde_json::json!({ "name": name, "path": rel, "mtime": mtime_ms, "excerpt": excerpt }));
        }
    }
    docs.sort_by(|a, b| b["mtime"].as_i64().unwrap_or(0).cmp(&a["mtime"].as_i64().unwrap_or(0)));
    Ok(Json(serde_json::json!({ "success": true, "data": { "docs": docs, "indices": indices } })).into_response())
}

/// 产物文档全文（analysis/solution 关评审用——摘要不够，要看全文才能批）。
/// path 必须是 .easyvibe/development_docs/ 下的相对路径（canonicalize 后前缀校验，防目录穿越）。
pub(crate) async fn get_dev_doc(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let rel = q.get("path").cloned().unwrap_or_default();
    // 2026-10-03 实弹 bug：dev-docs 返回的是仓库相对路径（.easyvibe/development_docs/…），
    // 本端点期望 development_docs 相对路径——直接 join 会双前缀 404，评审卡全文永远空。
    // 归一化：两种形态都收（前端无需关心口径差异）。
    let rel = rel
        .strip_prefix(".easyvibe/development_docs/")
        .map(str::to_string)
        .unwrap_or(rel);
    let docs_root = repo.root.join(".easyvibe/development_docs");
    let full = docs_root.join(&rel);
    let (Ok(canonical), Ok(docs_canon)) = (full.canonicalize(), docs_root.canonicalize()) else {
        return Err(AppError(ApiError::NotFound("文档不存在".into())));
    };
    if !canonical.starts_with(&docs_canon) || !canonical.is_file() {
        return Err(AppError(ApiError::NotFound("文档不存在（路径越界或非文件）".into())));
    }
    let content = std::fs::read_to_string(&canonical).map_err(|e| ApiError::Internal(format!("文档读取失败: {e}")))?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "path": rel, "content": content } })).into_response())
}

/// 删除单份归档文档（2026-10-04 审计 P1：development_docs 只进不出）。
/// 防线与 get_dev_doc 同口径（canonicalize + development_docs 前缀越界拒绝），
/// 且拒绝删除"正在运行任务"的产物目录文档由前端两步确认把关——后端不做任务态耦合。
pub(crate) async fn delete_dev_doc(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let rel = body.get("path").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let rel = rel
        .strip_prefix(".easyvibe/development_docs/")
        .map(str::to_string)
        .unwrap_or(rel);
    let docs_root = repo.root.join(".easyvibe/development_docs");
    let full = docs_root.join(&rel);
    let (Ok(canonical), Ok(docs_canon)) = (full.canonicalize(), docs_root.canonicalize()) else {
        return Err(AppError(ApiError::NotFound("文档不存在".into())));
    };
    if !canonical.starts_with(&docs_canon) || !canonical.is_file() {
        return Err(AppError(ApiError::NotFound("文档不存在（路径越界或非文件）".into())));
    }
    std::fs::remove_file(&canonical).map_err(|e| ApiError::Internal(format!("文档删除失败: {e}")))?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "path": rel } })).into_response())
}
