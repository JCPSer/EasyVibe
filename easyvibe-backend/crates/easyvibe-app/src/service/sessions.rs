//! sessions 域编排（c-arch-7 R1 新增）：运行会话/用量/健康看板/使用证据埋点的业务编排。
//!
//! 自 `routes/sessions.rs` 原样搬迁（零语义改动）——routes 只保留 HTTP 边界
//! （Path/Query 解析 → 调本域 → 映射响应）。

use crate::db_ports::{EventPort as _, HealthPort as _};
use crate::state::*;
use easyvibe_common::ApiError;
use tracing::info;

/// 健康历史：巡检运行列表 / R3 D1：前端交互埋点入库（dot.case 事件名 + JSON 计数维度）。
pub(crate) async fn ingest_event(st: &AppState, id: &str, body: serde_json::Value) -> Result<serde_json::Value, ApiError> {
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let name = body["name"].as_str().unwrap_or_default().trim();
    if name.is_empty() || name.len() > 64 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-') {
        return Err(ApiError::BadRequest("事件名须为 dot.case（字母数字._-，≤64）".into()));
    }
    let payload = body["payload"].as_object().map(|_| body["payload"].to_string()).unwrap_or_else(|| "{}".into());
    st.event_repo.record(&repo.id, name, &payload).await?;
    Ok(serde_json::json!({ "success": true }))
}

/// R3 D1：门控读数——按事件名计数（L3 三道门、Harness 验证指标的秤）。
pub(crate) async fn events_summary(st: &AppState, id: &str) -> Result<serde_json::Value, ApiError> {
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let rows = st.event_repo.summary(&repo.id).await?;
    Ok(serde_json::json!({ "success": true, "data": rows }))
}

/// 用量聚合（days 缺省 30，clamp(0,36500)；days==0 = 全部）——
/// 一次端点拉全五个聚合 + 会话列表（六路 tokio::join! 并发）。
pub(crate) async fn get_usage(st: &AppState, id: &str, days: Option<i64>) -> Result<serde_json::Value, ApiError> {
    let days = days.unwrap_or(30).clamp(0, 36500);
    let since = if days == 0 {
        "1970-01-01T00:00:00Z".to_string() // 全部
    } else {
        (chrono::Utc::now() - chrono::Duration::days(days.into())).to_rfc3339()
    };
    let (totals, daily, by_kind, by_model, by_module, sessions) = tokio::join!(
        st.agent_session_repo.usage_totals(id, &since),
        st.agent_session_repo.usage_daily(id, &since),
        st.agent_session_repo.usage_by_kind(id, &since),
        st.agent_session_repo.usage_by_model(id, &since),
        st.agent_session_repo.usage_by_module(id, &since),
        st.agent_session_repo.list(id, 50),
    );
    Ok(serde_json::json!({
        "success": true,
        "data": {
            "since": since,
            "totals": totals?,
            "daily": daily?,
            "byKind": by_kind?,
            "byModel": by_model?,
            "byModule": by_module?,
            "sessions": sessions?,
        }
    }))
}

/// M2：会话输出回放/补拉——afterSeq 之后的行（升序，上限 5000）。
pub(crate) async fn get_session_output(
    st: &AppState,
    sid: &str,
    after_seq: Option<i64>,
    limit: Option<i64>,
) -> Result<serde_json::Value, ApiError> {
    let rows = st
        .session_output_repo
        .fetch_after(sid, after_seq.unwrap_or(0), limit.unwrap_or(5000).min(5000))
        .await?;
    Ok(serde_json::json!({ "success": true, "data": rows }))
}

/// M1/U1：会话历史（运行页历史回放 / 用量页统计的数据源）。
pub(crate) async fn list_agent_sessions(st: &AppState, id: &str) -> Result<serde_json::Value, ApiError> {
    let rows = st.agent_session_repo.list(id, 50).await?;
    Ok(serde_json::json!({ "success": true, "data": rows }))
}

/// 巡检运行列表（近 20 次）。
pub(crate) async fn list_patrol_runs(st: &AppState, id: &str) -> Result<serde_json::Value, ApiError> {
    let runs = st.health_repo.list_runs(id, 20).await?;
    Ok(serde_json::json!({ "success": true, "data": runs }))
}

/// 巡检历史清理（重审 P1）：只留最近 N 次已终态巡检，running 的永不进删除集。
/// 默认 keep=10，上限 200（防误传超大值）。
pub(crate) async fn prune_patrol_runs(st: &AppState, id: &str, keep: i64) -> Result<serde_json::Value, ApiError> {
    st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let keep = keep.clamp(0, 200);
    let deleted = st.health_repo.prune_runs(id, keep).await?;
    info!("[patrol-prune] {} 清理历史巡检 {} 条（保留最近 {} 次）", id, deleted, keep);
    Ok(serde_json::json!({ "success": true, "data": { "deleted": deleted, "keep": keep } }))
}

/// M4-3 健康看板数据面：近 20 次巡检（含各自模块平均分）+ 最近一次成功巡检的模块明细。
/// 一次聚合查询代替前端 N×M 次 health-history 轮询（N 模块 × M 次巡检）。
pub(crate) async fn get_health_dashboard(st: &AppState, id: &str) -> Result<serde_json::Value, ApiError> {
    st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let runs = st.health_repo.list_runs(id, 20).await?;
    let avgs = st.health_repo.list_run_averages(id, 20).await?;
    let avg_of: std::collections::HashMap<String, (i64, i64)> = avgs
        .iter()
        .map(|a| (a.run_id.clone(), (a.module_avg, a.module_count)))
        .collect();
    let runs_json: Vec<serde_json::Value> = runs
        .iter()
        .map(|r| {
            let (module_avg, module_count) = avg_of.get(&r.id).copied().unwrap_or((0, 0));
            serde_json::json!({
                "id": r.id,
                "startedAt": r.started_at,
                "finishedAt": r.finished_at,
                "status": r.status,
                "model": r.model,
                "archScore": r.arch_score,
                "moduleAvg": module_avg,
                "moduleCount": module_count,
                // 2026-10-05 问题项新旧对照（仅 succeeded 巡检有值）
                "concernsDiff": r.concerns_diff.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
            })
        })
        .collect();
    let latest_modules = st.health_repo.list_latest_run_modules(id).await?;
    Ok(serde_json::json!({
        "success": true,
        "data": { "runs": runs_json, "latestModules": latest_modules },
    }))
}

/// P0 审查后端#1：终止指定会话（归纳/巡检/子图分析/任务执行同一通道）。
/// 已终态返回 409；外部自注册会话（无终止通道）返回 409。
pub(crate) async fn kill_session(st: &AppState, id: &str, sid: &str) -> Result<(), ApiError> {
    st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    st.session_manager.kill(sid).await?;
    Ok(())
}
