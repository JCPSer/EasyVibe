//! task_exec::changes —— git 变更采集、启动基线快照与终态结果收集（显式入参异步函数）。

use super::*;

/// 任务启动后变更的文件（越界校验候选集）：
/// - 已跟踪：`git diff --name-only <base>`（base 缺省 HEAD）——只算基线之后的改动
/// - 未跟踪：porcelain -uall 的 `??` 行（diff 系命令漏未跟踪，新建文件恰是最常见越界形态）
pub async fn changed_files(repo_root: &std::path::Path, base: Option<&str>) -> Vec<String> {
    let run = |args: &[&str]| {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio::process::Command::new("git").args(args).current_dir(repo_root).output(),
        )
    };
    let mut out: Vec<String> = vec![];
    let diff_args: Vec<&str> = match base {
        Some(b) => vec!["diff", "--name-only", b],
        None => vec!["diff", "--name-only", "HEAD"],
    };
    if let Ok(Ok(o)) = run(&diff_args).await {
        if o.status.success() {
            out.extend(String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()));
        }
    }
    if let Ok(Ok(o)) = run(&["status", "--porcelain=v1", "-uall"]).await {
        if o.status.success() {
            out.extend(
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| l.starts_with("?? "))
                    .filter_map(|l| l.get(3..).map(str::trim).map(str::to_string))
                    .filter(|l| !l.is_empty()),
            );
        }
    }
    out.sort();
    out.dedup();
    out
}

/// 启动基线快照：任务开始时工作区的全部脏文件（已跟踪改动 + 未跟踪）。
/// 采集时从越界候选集中扣减——任务之前就在那儿的陈年脏文件不进冤案（R2 审查裂缝#1）。
/// 存内存（spawn 与采集同进程；重启会把 running 任务标 interrupted，基线随之失效）。
///
/// 2026-10-03 实弹修订（hover-client 644 越界冤案）：基线的未跟踪部分必须**无视
/// .gitignore**——上一轮回被打回的实施改了 .gitignore（把 docs/ 加了忽略），基线
/// 快照时这批文件"被消失"，该轮 agent 恢复 .gitignore 后它们在采集时首次进入 git
/// 视野，644 个文件全部误判为"任务新增越界"。修复 = 基线并集
/// `git ls-files --others --ignored`（被忽略但存在的文件也是"早就有的"）。
/// 上限 5 万条防巨型 ignored 目录（node_modules 类）内存爆炸——超限则放弃该部分
/// （退回旧行为，宁可漏排不炸内存）。
pub async fn dirty_files(repo_root: &std::path::Path) -> Vec<String> {
    let Ok(Ok(o)) = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new("git").args(["status", "--porcelain=v1", "-uall"]).current_dir(repo_root).output(),
    )
    .await
    else {
        return vec![]
    };
    if !o.status.success() {
        return vec![];
    }
    let mut v: Vec<String> = String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| l.get(3..).map(str::trim).map(str::to_string))
        .filter(|l| !l.is_empty())
        .map(|p| p.split_once(" -> ").map(|(_, to)| to.trim().to_string()).unwrap_or(p))
        .collect();
    v.sort();
    v.dedup();
    // 被忽略但存在的文件：gitignore 游戏免疫（见上方注释）
    if let Ok(Ok(ig)) = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new("git")
            .args(["ls-files", "--others", "--ignored", "--exclude-standard", "-z"])
            .current_dir(repo_root)
            .output(),
    )
    .await
    {
        if ig.status.success() {
            let ignored: Vec<String> = String::from_utf8_lossy(&ig.stdout)
                .split('\0')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            if ignored.len() <= 50_000 {
                v.extend(ignored);
                v.sort();
                v.dedup();
            }
        }
    }
    v
}

/// git 变更摘要（diff 关供料）：`git diff --stat`（已跟踪改动）+ `git status --porcelain`
/// （未跟踪新文件）。非 git 仓库返回 None——git 是增强项不是硬依赖（设计定稿）。
pub async fn git_change_summary(repo_root: &std::path::Path, base: Option<&str>) -> Option<String> {
    let run = |args: &[&str]| {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio::process::Command::new("git").args(args).current_dir(repo_root).output(),
        )
    };
    let mut parts: Vec<String> = vec![];
    let stat_args: Vec<&str> = match base {
        Some(b) => vec!["diff", "--stat", b],
        None => vec!["diff", "--stat", "HEAD"],
    };
    match run(&stat_args).await {
        Ok(Ok(out)) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                parts.push(s);
            }
        }
        _ => return None, // 非 git 仓库或 git 不可用：无摘要可给
    }
    if let Ok(Ok(out)) = run(&["status", "--porcelain"]).await {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                parts.push(format!("工作区状态：\n{s}"));
            }
        }
    }
    if parts.is_empty() { None } else { Some(parts.join("\n")) }
}

/// 完整 diff（M4-3 diff 可视化）：`git diff HEAD`，256KB 封顶（超帽截断并标注）。
/// 非 git 仓库返回 None。落归档文件供 GET task-diff 读取，tasks.result 只带 stat 摘要不带全文。
pub async fn git_full_diff(repo_root: &std::path::Path, base: Option<&str>) -> Option<String> {
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        tokio::process::Command::new("git").args(match base {
            Some(b) => vec!["diff", b],
            None => vec!["diff", "HEAD"],
        }).current_dir(repo_root).output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None; // 非 git 仓库
    }
    let mut s = String::from_utf8_lossy(&out.stdout).to_string();
    if s.is_empty() {
        return None;
    }
    const CAP: usize = 262_144;
    if s.len() > CAP {
        s.truncate(CAP);
        s.push_str("\n…（diff 超 256KB 已截断）");
    }
    Some(s)
}

/// 变更归因：读任务行拿 base_head（spawn 时已落库）；读不到（竞态/旧库）回退 None=HEAD 口径
pub(crate) async fn base_head_from_task(task_repo: &dyn TaskStore, task_id: &str) -> Option<String> {
    task_repo.get(task_id).await.ok().flatten().and_then(|t| t.base_head)
}

/// M4-1 终态采集：stdout 的 RESULT 行 + git 变更摘要 → `.easyvibe/development_docs/` 归档
/// （§9 #2：git 可见、随 PR 评审）→ tasks.result JSON（diff 关审批的展示原料）。
/// 两者皆无（agent 无输出且非 git 仓库）返回 None。
pub async fn collect_task_result(
    session_manager: &SessionManager,
    session_id: &str,
    repo_root: &std::path::Path,
    task_id: &str,
    task_repo: &dyn TaskStore,
    baseline: &[String],
) -> Option<String> {
    let output = session_manager.take_output(session_id).await.unwrap_or_default();
    let parsed = parse_result_line(&output);
    // 变更归因：优先用任务 base_head，缺省回退 HEAD（兼容旧任务与无 git）
    let base_owned = base_head_from_task(task_repo, task_id).await;
    let diff_stat = git_change_summary(repo_root, base_owned.as_deref()).await;
    let diff_full = git_full_diff(repo_root, base_owned.as_deref()).await;
    if parsed.is_none() && diff_stat.is_none() && diff_full.is_none() {
        return None;
    }
    // 实弹#3 防线：会话判成功但 agent 未输出 [EASYVIBE-RESULT] 行（可能被带偏/模型未遵从）——
    // 不阻断终态（与归纳产物核验同款哲学），但给审批人亮警告，diff 关须警惕"空执行"
    let mut warnings: Vec<String> = vec![];
    if parsed.is_none() {
        warnings.push("agent 未输出 [EASYVIBE-RESULT] 归档行——执行可能未按协议完成，审批时请核对 diff 是否为本任务改动".into());
    }
    // 影响面合约：越界写文件 = 红线（注入式护栏哲学——不阻断，但审批人必须看见）。
    // 确定性校验：基线之后的变更文件（扣启动时已有的脏文件，防冤案）× 创建时展开的模块 glob，零 LLM。
    let task_ctx = task_repo.get(task_id).await.ok().flatten().map(|t| t.context).unwrap_or_default();
    let contract = contract_patterns_from_context(&task_ctx);
    let baseline_set: std::collections::HashSet<&str> = baseline.iter().map(String::as_str).collect();
    // 产品自身 bookkeeping（.easyvibe/ 归档/地图产物）不属于 agent 改动——排除出合约校验
    let contract_violations: Vec<String> = if contract.is_empty() {
        vec![]
    } else {
        changed_files(repo_root, base_owned.as_deref())
            .await
            .into_iter()
            .filter(|p| !baseline_set.contains(p.as_str()))
            // 产品自身与 agent 脚手架的 bookkeeping（.easyvibe/ 归档/地图、.claude/ STAR 记忆）
            // 不属于任务改动——排除出合约校验（自托管实弹：harness STAR 归档路径约定待统一，见债务登记）
            .filter(|p| !p.starts_with(".easyvibe/") && !p.starts_with(".claude/"))
            .filter(|p| !path_within_contract(p, &contract))
            .collect()
    };
    if !contract_violations.is_empty() {
        let preview: Vec<String> = contract_violations.iter().take(5).cloned().collect();
        warnings.push(format!(
            "影响面合约：{} 个文件越出任务声明的模块边界——{}（diff 关须逐条确认或驳回）",
            contract_violations.len(),
            preview.join("、")
        ));
    }
    let mut archive = serde_json::json!({
        "taskId": task_id,
        "sessionId": session_id,
        "collectedAt": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string(),
        "result": parsed,
        "diffStat": diff_stat,
        "diffFull": diff_full,
        "warnings": warnings,
        "contractViolations": contract_violations,
        "archivedPath": serde_json::Value::Null,
    });
    // 阶段初审结论（phaseReviews）在终态采集时保留——实施阶段的 set_result 是整体重写，
    // 不携带会把矩阵/方案的初审记录冲掉（评审轮回缺一环）
    if let Ok(Some(row)) = task_repo.get(task_id).await {
        if let Some(res) = &row.result {
            if let Ok(old) = serde_json::from_str::<serde_json::Value>(res) {
                if let Some(pr) = old.get("phaseReviews") {
                    archive["phaseReviews"] = pr.clone();
                }
            }
        }
    }
    let dir = repo_root.join(".easyvibe/development_docs");
    if tokio::fs::create_dir_all(&dir).await.is_ok() {
        let path = dir.join(format!("{task_id}.json"));
        if easyvibe_map::atomic_write_json(&path, &archive).await.is_ok() {
            archive["archivedPath"] = serde_json::json!(path.to_string_lossy());
        }
    }
    // tasks.result 只带 stat 与归档路径（任务列表载荷可控）；diff 全文只进归档文件，
    // 由 GET /repos/{id}/tasks/{tid}/diff 按需读取（M4-3）
    let mut slim = archive;
    slim.as_object_mut()?.remove("diffFull");
    serde_json::to_string(&slim).ok()
}
