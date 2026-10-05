//! task_exec::contract —— 影响面合约守护、风险预评估与 [EASYVIBE-RESULT] 解析（纯函数）。

use super::*;

/// 改进#7 supervised 风险预评估（v1 确定性规则——LLM 评估为记档增强）。
/// 高危信号：大范围改动（>3 模块）/ 高危关键词 / 动低分模块（<50 分，手术风险高）。
/// 返回 (是否高危, 理由)。
pub fn risk_assess(task: &TaskRow) -> (bool, String) {
    let modules: Vec<String> = serde_json::from_str(&task.modules).unwrap_or_default();
    let mut reasons: Vec<String> = vec![];
    if modules.len() > 3 {
        reasons.push(format!("影响 {} 个模块（>3，大范围改动）", modules.len()));
    }
    for kw in ["重构", "架构", "整体", "全部", "删除", "迁移"] {
        if task.description.contains(kw) {
            reasons.push(format!("描述含高危关键词「{kw}」"));
            break;
        }
    }
    // 低分模块由调用方上下文难以获取——用 description 长度代理复杂度（长描述=大需求）
    if task.description.chars().count() > 200 {
        reasons.push("需求描述超长（>200 字，需求可能未收敛）".to_string());
    }
    if reasons.is_empty() {
        (false, format!("影响 {} 个模块，无高危信号", modules.len()))
    } else {
        (true, reasons.join("；"))
    }
}

/// 解析 agent stdout 的 `[EASYVIBE-RESULT] {json}` 归档行
/// （assemble_task_prompt 要求 agent 最后一行输出；从尾部找，容忍前后缀文字）
pub fn parse_result_line(output: &str) -> Option<serde_json::Value> {
    let line = output.lines().rev().find(|l| l.contains("[EASYVIBE-RESULT]"))?;
    let start = line.find("[EASYVIBE-RESULT]")? + "[EASYVIBE-RESULT]".len();
    let payload = line[start..].trim();
    serde_json::from_str(payload)
        .ok()
        .or_else(|| easyvibe_ai_agent::extract_json(payload).ok())
}

/// L2 哨兵：从当前越界候选中剔出**未上报过**的新文件（幂等——同一文件只预警一次，
/// 已上报集合随任务生命周期累计）。纯函数便于测试。
pub fn new_violators(candidates: &[String], reported: &mut std::collections::HashSet<String>) -> Vec<String> {
    let fresh: Vec<String> = candidates.iter().filter(|p| !reported.contains(*p)).cloned().collect();
    reported.extend(fresh.iter().cloned());
    fresh
}

/// L2 哨兵巡检间隔（秒）：默认 15s——够快能拦住"越界写一大片"的趋势，
/// 又不至于让 git 调用频率喧宾夺主（env 可调）。
pub(crate) fn sentry_interval() -> std::time::Duration {
    std::env::var("EASYVIBE_CONTRACT_SENTRY_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&s| s > 0)
        .map(std::time::Duration::from_secs)
        .unwrap_or(std::time::Duration::from_secs(15))
}

/// N26：任务槽会话超时（env 可调）——任务执行是自由 coding，40-60 分钟常态；
/// 透明槽位（归纳/巡检/子图）仍用 SessionManager 默认 30 分钟。
pub fn task_session_timeout() -> std::time::Duration {
    std::env::var("EASYVIBE_TASK_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&s| s > 0)
        .map(std::time::Duration::from_secs)
        .unwrap_or(std::time::Duration::from_secs(90 * 60))
}

// ---------- 影响面合约（战略审查第一 P0：任务声明的模块从注释升级为确定性边界） ----------

/// 路径是否落在合约 glob 范围内——与前端 moduleOfFile 同口径且**带路径段边界**：
/// `**` 前前缀去尾斜杠后，命中条件：路径恰为 base（精确文件型 glob，如 "src/main.rs"）、
/// 以 `base/` 开头（目录前缀）、或整段包含 `/base/`——
/// R2 审查实锤：starts_with("src/core") 会放过 src/coreography/，前缀必须有边界；
/// 自托管实弹（dogfood）：漏掉 path == base 会让精确文件型 glob 永不命中（回归锁死）。
pub fn path_within_contract(path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|g| {
        let base = match g.find("**") {
            Some(i) => &g[..i],
            None => g.as_str(),
        };
        let base = base.trim_end_matches('/');
        if base.is_empty() {
            return false;
        }
        path == base || path.starts_with(&format!("{base}/")) || path.contains(&format!("/{base}/"))
    })
}

/// 从任务 context 提取影响面合约（创建任务时由模块展开写入；无合约返回空 = 不约束）
pub fn contract_patterns_from_context(context_json: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(context_json)
        .ok()
        .and_then(|v| v["contract"]["patterns"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|p| p.as_str().map(str::to_string))
        .collect()
}
