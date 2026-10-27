//! task_exec::review —— 评审门：独立子 agent 审查、阶段产物初审与结论归并（显式入参异步函数）。

use super::*;

/// 审查结论（解析自 [EASYVIBE-REVIEW] 行；verdict 可为 pass / fail / error——
/// error = 审查不可用时的占位结论，summary 携带具体失败原因，前端灰横幅展示）
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReviewVerdict {
    pub verdict: String, // pass / fail / error
    pub summary: String,
}

/// 审查结果三态（2026-10-06 实弹修正）：自建模型 slot 被 claude CLI 拒（stderr
/// unrecognized_model）时初审会话 4 秒异常终态，旧实现返回 None → 仅日志一行 info →
/// result.phaseReviews 无记录 → 前端评审卡完全不显示初审横幅，用户以为没有评审环节。
/// Unavailable 必须携带具体失败原因（截取前 200 字符），由调用方落库（verdict=error）。
#[derive(Debug, Clone)]
pub enum ReviewOutcome {
    Verdict(ReviewVerdict),
    Unavailable(String),
}

impl ReviewOutcome {
    /// 失败原因统一出口：非空兜底 + 前 200 字符截取（stderr 长行/启动错误串不得撑爆留痕）。
    fn unavailable(reason: impl Into<String>) -> Self {
        let r = reason.into();
        let r = if r.trim().is_empty() { "未知原因".to_string() } else { r };
        ReviewOutcome::Unavailable(r.chars().take(200).collect())
    }
}

/// spawn 审查/初审会话：单会话纪律下，执行刚终态的 ≤2s 空档里仓库槽位可能已被
/// 另一排队任务的重试抢占（2026-10-06 实弹：并发任务时评审有概率静默丢失——
/// start_induction 遇 Conflict 被 .ok()? 吞掉 → "审查不可用，照常进 diff 关"）。
/// 对策：Conflict 有界重试（4s 一拍，15 分钟上限——占用方是会话，终态必然释放，
/// 无死锁：占用方自己也在 5s 重试节拍上等这个槽位）；非 Conflict 错误/超时立即降级，
/// 且降级原因落日志（此前静默吞错误，排查只能靠猜）。
async fn start_review_session(
    session_manager: &SessionManager,
    repo_id: &str,
    repo_root: &std::path::Path,
    prompt: &str,
    agent_command: &str,
    agent_args: &[String],
    timeout: std::time::Duration,
    purpose: &str,
) -> Result<easyvibe_api_types::SessionStatusChanged, String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15 * 60);
    loop {
        match session_manager
            .start_induction(repo_id, repo_root, prompt, agent_command, agent_args, Some(timeout))
            .await
        {
            Ok(s) => return Ok(s),
            Err(easyvibe_common::ApiError::Conflict(_)) if tokio::time::Instant::now() < deadline => {
                info!("[task-exec] {} 会话槽位被占用，4s 后重试…", purpose);
                tokio::time::sleep(std::time::Duration::from_secs(4)).await;
            }
            Err(e) => {
                // 降级原因随返回值上抛（调用方落库 verdict=error），日志保留便于排查
                warn!("[task-exec] {} 会话启动失败，按不可用处理（降级不阻断）: {e}", purpose);
                return Err(format!("会话启动失败: {e}"));
            }
        }
    }
}

/// 跑独立审查会话并等终态。25 分钟上限（审查是分钟级任务；超时判不可用不阻断）。
/// custom：自定义层补充（review 槽）；rules：出厂规则内嵌副本（开发/修复两份，直接进 prompt）。
pub(crate) async fn run_subagent_review(
    session_manager: &SessionManager,
    agent_command: &str,
    agent_args: &[String],
    repo_id: &str,
    repo_root: &std::path::Path,
    task: &TaskRecord,
    custom: &HarnessCustom,
    rules: (&str, &str),
) -> ReviewOutcome {
    let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
    let prompt = assemble_review_prompt(task, &user, custom, rules);
    // 审查会话沿用任务槽 CLI 参数（可经 settings `agent.args.review` 单独收紧/换模型）
    let session = match start_review_session(
        session_manager,
        repo_id,
        repo_root,
        &prompt,
        agent_command,
        agent_args,
        std::time::Duration::from_secs(25 * 60),
        "审查",
    )
    .await
    {
        Ok(s) => s,
        Err(reason) => return ReviewOutcome::Unavailable(reason),
    };
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
    task: &TaskRecord,
    phase: u8,
    custom: &HarnessCustom,
) -> ReviewOutcome {
    let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
    let prompt = assemble_phase_review_prompt(task, phase, &user, custom);
    // 初审是读文档+下结论，比实施审查轻——15 分钟上限足够
    let session = match start_review_session(
        session_manager,
        repo_id,
        repo_root,
        &prompt,
        agent_command,
        agent_args,
        std::time::Duration::from_secs(15 * 60),
        "初审",
    )
    .await
    {
        Ok(s) => s,
        Err(reason) => return ReviewOutcome::Unavailable(reason),
    };
    let sid = session.session_id.clone();
    info!("[task-exec] 任务 {} 阶段 {} 初审会话 {} 已启动", task.id, phase, sid);
    session_manager.note_label(&sid, "任务执行·初审".into()).await;
    await_review_verdict(session_manager, &sid, &task.id, std::time::Duration::from_secs(15 * 60)).await
}

/// 审查会话终态等待 + [EASYVIBE-REVIEW] 行解析（实施审查与阶段初审共用）。
/// 会话异常终态/超时/结论非法 → Unavailable(原因)（不可用不阻断，人机审查兜底，
/// 原因由调用方落库 verdict=error，前端横幅可见——2026-10-06 实弹修正）。
pub(crate) async fn await_review_verdict(
    session_manager: &SessionManager,
    session_id: &str,
    task_id: &str,
    timeout: std::time::Duration,
) -> ReviewOutcome {
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
                    // 失败原因取证：捕获缓冲已并入 stderr（[err] 前缀，与过程直播同格式）——
                    // 实弹：claude CLI 拒模型只写 stderr（unrecognized_model），stdout 为空。
                    // 取最后一条 stderr 行；一行都没有则退化为通用文案。
                    let out = session_manager.take_output(session_id).await.unwrap_or_default();
                    let reason = out
                        .lines()
                        .rev()
                        .find_map(|l| l.strip_prefix("[err] "))
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .unwrap_or("会话异常终态");
                    return ReviewOutcome::unavailable(reason);
                }
                let out = session_manager.take_output(session_id).await.unwrap_or_default();
                let Some(line) = out.lines().rev().find(|l| l.contains("[EASYVIBE-REVIEW]")) else {
                    return ReviewOutcome::unavailable("审查结论行缺失（无 [EASYVIBE-REVIEW] 输出）");
                };
                let Some(json_str) = line.split("[EASYVIBE-REVIEW]").nth(1) else {
                    return ReviewOutcome::unavailable("审查结论行缺失（无 [EASYVIBE-REVIEW] 输出）");
                };
                let json_str = json_str.trim();
                let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) else {
                    return ReviewOutcome::unavailable(format!("审查结论 JSON 非法: {json_str}"));
                };
                let verdict = v["verdict"].as_str().unwrap_or("").to_string();
                if verdict != "pass" && verdict != "fail" {
                    warn!("[task-exec] 任务 {} 审查结论 verdict 非法：{}", task_id, verdict);
                    return ReviewOutcome::unavailable(format!("审查结论 verdict 非法: {verdict}"));
                }
                let summary = v["summary"].as_str().unwrap_or("（无结论摘要）").to_string();
                info!("[task-exec] 任务 {} 审查结论：{} — {}", task_id, verdict, summary);
                return ReviewOutcome::Verdict(ReviewVerdict { verdict, summary });
            }
            None => return ReviewOutcome::unavailable("会话状态丢失"),
            _ => {
                if tokio::time::Instant::now() > deadline {
                    warn!("[task-exec] 任务 {} 审查会话超时，按不可用处理", task_id);
                    let _ = session_manager.kill(session_id).await;
                    return ReviewOutcome::unavailable("审查会话超时");
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    }
}

/// 实施审查结论并入 tasks.result.review（含 at 毫秒时间戳——前端「人工复审」
/// 轮次完成判定的依据：点击发起后轮询到 at ≥ 点击时刻即知本轮已出结论）。
/// verdict 可为 pass / fail / error（error = 审查不可用的占位结论，summary 携带失败原因）。
/// result 可能不存在（复审发生在采集前），此时新建 JSON 骨架。
pub(crate) async fn merge_review_verdict(task_repo: &dyn TaskStore, task_id: &str, v: &ReviewVerdict) {
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
/// verdict 可为 pass / fail / error（error = 初审不可用的占位结论，summary 携带失败原因）。
pub(crate) async fn merge_phase_review(
    task_repo: &dyn TaskStore,
    task_id: &str,
    key: &str,
    v: &ReviewVerdict,
) {
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
