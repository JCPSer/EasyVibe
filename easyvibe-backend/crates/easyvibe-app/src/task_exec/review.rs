//! task_exec::review —— 评审门：独立子 agent 审查、阶段产物初审与结论归并（显式入参异步函数）。

use super::*;

/// 审查结论（解析自 [EASYVIBE-REVIEW] 行）
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReviewVerdict {
    pub verdict: String, // pass / fail
    pub summary: String,
}

/// 跑独立审查会话并等终态。25 分钟上限（审查是分钟级任务；超时判不可用不阻断）。
/// custom：自定义层补充（注入点 #3——global + development 块拼进审查 prompt）。
pub(crate) async fn run_subagent_review(
    session_manager: &SessionManager,
    agent_command: &str,
    agent_args: &[String],
    repo_id: &str,
    repo_root: &std::path::Path,
    task: &TaskRow,
    custom: &HarnessCustom,
) -> Option<ReviewVerdict> {
    let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
    let prompt = assemble_review_prompt(task, &user, custom);
    // 审查会话沿用任务槽 CLI 参数（可经 settings `agent.args.review` 单独收紧/换模型）
    let session = session_manager
        .start_induction(repo_id, repo_root, &prompt, agent_command, agent_args, Some(std::time::Duration::from_secs(25 * 60)))
        .await
        .ok()?;
    let sid = session.session_id.clone();
    info!("[task-exec] 任务 {} 审查会话 {} 已启动", task.id, sid);
    session_manager.note_label(&sid, "任务执行·审查".into()).await;
    await_review_verdict(session_manager, &sid, &task.id, std::time::Duration::from_secs(25 * 60)).await
}

/// 阶段产物初审（2026-10-03 用户裁定）：需求矩阵（phase 1）/方案设计（phase 2）
/// 到人工关之前，先派子 agent 预筛一遍——完整性/可测性/一致性。
/// 与实施后审查的关键差异：fail 不自动打回——人是最终裁决，初审只是给审批人
/// 多一双眼睛（结论进 result.phaseReviews，评审卡横幅展示）。
pub(crate) async fn run_phase_doc_review(
    session_manager: &SessionManager,
    agent_command: &str,
    agent_args: &[String],
    repo_id: &str,
    repo_root: &std::path::Path,
    task: &TaskRow,
    phase: u8,
    custom: &HarnessCustom,
) -> Option<ReviewVerdict> {
    let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
    let prompt = assemble_phase_review_prompt(task, phase, &user, custom);
    // 初审是读文档+下结论，比实施审查轻——15 分钟上限足够
    let session = session_manager
        .start_induction(repo_id, repo_root, &prompt, agent_command, agent_args, Some(std::time::Duration::from_secs(15 * 60)))
        .await
        .ok()?;
    let sid = session.session_id.clone();
    info!("[task-exec] 任务 {} 阶段 {} 初审会话 {} 已启动", task.id, phase, sid);
    session_manager.note_label(&sid, "任务执行·初审".into()).await;
    await_review_verdict(session_manager, &sid, &task.id, std::time::Duration::from_secs(15 * 60)).await
}

/// 审查会话终态等待 + [EASYVIBE-REVIEW] 行解析（实施审查与阶段初审共用）。
/// 会话异常终态/超时/结论非法 → None（不可用不阻断，人机审查兜底）。
pub(crate) async fn await_review_verdict(
    session_manager: &SessionManager,
    session_id: &str,
    task_id: &str,
    timeout: std::time::Duration,
) -> Option<ReviewVerdict> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match session_manager.status_of_session(session_id).await {
            Some(s)
                if matches!(
                    s.status,
                    easyvibe_api_types::SessionStatus::Succeeded | easyvibe_api_types::SessionStatus::Failed
                ) =>
            {
                if s.status != easyvibe_api_types::SessionStatus::Succeeded {
                    warn!("[task-exec] 任务 {} 审查会话异常终态：{:?}", task_id, s.status);
                    return None;
                }
                let out = session_manager.take_output(session_id).await.unwrap_or_default();
                let line = out.lines().rev().find(|l| l.contains("[EASYVIBE-REVIEW]"))?;
                let json_str = line.split("[EASYVIBE-REVIEW]").nth(1)?.trim();
                let v: serde_json::Value = serde_json::from_str(json_str).ok()?;
                let verdict = v["verdict"].as_str().unwrap_or("").to_string();
                if verdict != "pass" && verdict != "fail" {
                    warn!("[task-exec] 任务 {} 审查结论 verdict 非法：{}", task_id, verdict);
                    return None;
                }
                let summary = v["summary"].as_str().unwrap_or("（无结论摘要）").to_string();
                info!("[task-exec] 任务 {} 审查结论：{} — {}", task_id, verdict, summary);
                return Some(ReviewVerdict { verdict, summary });
            }
            None => return None,
            _ => {
                if tokio::time::Instant::now() > deadline {
                    warn!("[task-exec] 任务 {} 审查会话超时，按不可用处理", task_id);
                    let _ = session_manager.kill(session_id).await;
                    return None;
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    }
}

/// 实施审查结论并入 tasks.result.review（含 at 毫秒时间戳——前端「人工复审」
/// 轮次完成判定的依据：点击发起后轮询到 at ≥ 点击时刻即知本轮已出结论）。
/// result 可能不存在（复审发生在采集前），此时新建 JSON 骨架。
pub(crate) async fn merge_review_verdict(task_repo: &easyvibe_db::SqliteTaskRepository, task_id: &str, v: &ReviewVerdict) {
    use easyvibe_db::TaskRepository as _;
    let Ok(Some(row)) = task_repo.get(task_id).await else { return };
    let mut rv: serde_json::Value = row
        .result
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    rv["review"] = serde_json::json!({ "verdict": v.verdict, "summary": v.summary, "at": at });
    if let Ok(s) = serde_json::to_string(&rv) {
        let _ = task_repo.set_result(task_id, &s).await;
    }
}

/// 初审结论并入 tasks.result.phaseReviews（key = analysis / solution）——
/// result 可能尚不存在（阶段 1/2 不采集产物），此时新建 JSON 骨架。
pub(crate) async fn merge_phase_review(
    task_repo: &easyvibe_db::SqliteTaskRepository,
    task_id: &str,
    key: &str,
    v: &ReviewVerdict,
) {
    use easyvibe_db::TaskRepository as _;
    let Ok(Some(row)) = task_repo.get(task_id).await else { return };
    let mut rv: serde_json::Value = row
        .result
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    rv["phaseReviews"][key] = serde_json::json!({ "verdict": v.verdict, "summary": v.summary });
    if let Ok(s) = serde_json::to_string(&rv) {
        let _ = task_repo.set_result(task_id, &s).await;
    }
}
