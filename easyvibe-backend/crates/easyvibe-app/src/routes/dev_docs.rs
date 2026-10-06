//! 资源域：development_docs 产物读写删。
//!
//! c-arch-7 R1：按任务时间窗列举文档的编排（含任务读库）已下沉 `crate::service::task`；
//! 纯 FS 的全文读取/删除留本文件（HTTP 边界）。

use crate::state::*;
use axum::{
    extract::{Path, State},
    routing::get,
    response::{IntoResponse, Response},
    Json, Router,
};
use easyvibe_common::ApiError;

/// 本域路由（R1 自注册）：development_docs 产物读写删。
pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/repos/{id}/dev-docs", get(get_dev_docs))
        .route("/repos/{id}/dev-doc", get(get_dev_doc).delete(delete_dev_doc))
}

/// 任务产物文档（方案 v3 §4.4）：扫描 .easyvibe/development_docs/**，按任务时间窗过滤。
/// 时间窗规则与 excerpt 截断见 `service::task::list_task_dev_docs`（原样搬迁）。
pub(crate) async fn get_dev_docs(
    State(st): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let tid = q.get("taskId").cloned().unwrap_or_default();
    let body = crate::service::task::list_task_dev_docs(&st, &id, &tid).await?;
    Ok(Json(body).into_response())
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
