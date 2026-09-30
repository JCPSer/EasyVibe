//! 上下文压缩（auto-compact，backend-design §10a）：地图感知三层策略——
//! 1) 近期窗口原文保留；2) 早期历史 → 结构化摘要；3) 工具结果/代码片段丢弃（只留指针）。
//! 压缩摘要必须携带会话状态（§11 🟡6）：未决问题、表单草稿、已拍板决策。
//! 存储分离（§11 🟡5）：运行上下文 = 摘要 + 未压缩窗口；完整原文始终落 SQLite（compacted 标记）。
use crate::{estimate_tokens, ChatRequest, LlmClient};
use easyvibe_common::ApiError;
use easyvibe_db::ConversationMessageRow;

/// 触发判断：未压缩消息总 token 占预算比例 ≥ 阈值（默认 80%）
pub fn needs_compaction(uncompacted_tokens: i64, budget: i64, threshold_pct: i64) -> bool {
    budget > 0 && uncompacted_tokens * 100 >= budget * threshold_pct
}

/// 选择压缩水位：保留最近 keep_recent 条原文不动，其余折叠；不足一个窗口则不压。
pub fn compaction_watermark(messages: &[ConversationMessageRow], keep_recent: usize) -> Option<i64> {
    if messages.len() <= keep_recent.max(1) {
        return None;
    }
    Some(messages[messages.len() - keep_recent].id - 1)
}

/// 压缩执行结果
#[derive(Debug, Clone)]
pub struct CompactionResult {
    /// 新摘要（已叠加既有摘要；含会话状态段）
    pub summary: String,
    /// 水位：rowid ≤ 此值的消息标 compacted（原文保留）
    pub before_id: i64,
    /// 压缩前未压缩 token 占比（留痕消息"82%→34%"的原料，基于预算）
    pub before_pct: i64,
    /// 压缩后估算占比
    pub after_pct: i64,
    /// 压缩调用本身的 token 用量（计入成本护栏）
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
}

/// 生成摘要的共享骨架：增量叠加既有摘要 + 结构化段落要求
fn summary_user_prompt(existing: Option<&str>, compacted: &[ConversationMessageRow]) -> String {
    let transcript: String = compacted
        .iter()
        .map(|m| format!("[{}] {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "## 既有摘要（在其基础上增补，不要丢弃已记录事项）\n{existing}\n\n## 待压缩的早期对话\n{transcript}\n\n\
         输出结构化摘要，必须包含段落：\n\
         1) 已拍板决策（用户明确同意/选择的方案）；\n\
         2) 结论与关键事实（关于代码库/架构）；\n\
         3) 未决问题（拷问中待答、用户尚未确认的事项）；\n\
         4) 进行中的意图（表单草稿、待发起的任务）。\n\
         工具输出与代码片段只留指针（文件路径），不抄原文。",
        existing = existing.unwrap_or("（无）"),
        transcript = if transcript.is_empty() { "（无）".into() } else { transcript },
    )
}

/// Stub 压缩：确定性折叠——逐条截取要点，不做语义摘要（诚实标注 stub）。
/// 仍保留会话状态传递能力（原文要点不丢），供零成本端到端验证。
pub fn compact_stub(existing: Option<&str>, compacted: &[ConversationMessageRow], budget: i64, before_total: i64) -> CompactionResult {
    let keep: String = compacted
        .iter()
        .map(|m| format!("- [{}] {}", m.role, &m.content.chars().take(60).collect::<String>()))
        .collect::<Vec<_>>()
        .join("\n");
    let summary = format!(
        "[stub 压缩]\n{existing}{keep}",
        existing = existing.map(|s| format!("{s}\n")).unwrap_or_default(),
        keep = keep,
    );
    let after_total = estimate_tokens(&summary);
    CompactionResult {
        summary,
        before_id: compacted.last().map(|m| m.id).unwrap_or(0),
        before_pct: pct(before_total, budget),
        after_pct: pct(after_total, budget),
        prompt_tokens: 0,
        completion_tokens: 0,
    }
}

/// LLM 压缩：结构化摘要，会话状态必留（§11 🟡6）
pub async fn compact_with_llm<C: LlmClient>(
    llm: &C,
    existing: Option<&str>,
    compacted: &[ConversationMessageRow],
    budget: i64,
    before_total: i64,
) -> Result<CompactionResult, ApiError> {
    let system = "你是 EasyVibe 的上下文压缩器。把对话早期历史压缩为结构化摘要，供后续对话继续基于它作答。铁律：已拍板决策、未决问题、进行中的任务意图必须保留；宁可保留不可丢失。只输出摘要正文，不输出任何解释。";
    let user = summary_user_prompt(existing, compacted);
    let outcome = llm.chat(ChatRequest { system, user: &user }).await?;
    let after_total = estimate_tokens(&outcome.text);
    Ok(CompactionResult {
        summary: outcome.text,
        before_id: compacted.last().map(|m| m.id).unwrap_or(0),
        before_pct: pct(before_total, budget),
        after_pct: pct(after_total, budget),
        prompt_tokens: outcome.prompt_tokens as i64,
        completion_tokens: outcome.completion_tokens as i64,
    })
}

fn pct(part: i64, whole: i64) -> i64 {
    if whole <= 0 { 0 } else { part * 100 / whole }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: i64, role: &str, content: &str) -> ConversationMessageRow {
        ConversationMessageRow {
            id,
            conversation_id: "chat:demo".into(),
            role: role.into(),
            content: content.into(),
            compacted: false,
            tokens: estimate_tokens(content),
            created_at: "t".into(),
        }
    }

    #[test]
    fn compaction_trigger_at_threshold() {
        assert!(needs_compaction(80, 100, 80));
        assert!(needs_compaction(800, 1000, 80));
        assert!(!needs_compaction(79, 100, 80));
        assert!(!needs_compaction(10, 0, 80), "无预算不触发（防除零）");
    }

    #[test]
    fn watermark_keeps_recent_window() {
        let msgs: Vec<_> = (1..=10).map(|i| msg(i, "user", "问题")).collect();
        let wm = compaction_watermark(&msgs, 4).unwrap();
        assert_eq!(wm, 6, "水位应保住最后 4 条原文（id 7-10 不压，≤6 折叠）");
        assert!(compaction_watermark(&msgs, 20).is_none(), "不足一个窗口不压");
    }

    #[test]
    fn stub_compact_carries_session_state() {
        let msgs = vec![
            msg(1, "user", "我认为应该拆分 order-service，你同意吗？"),
            msg(2, "assistant", "同意，方案 A：先抽接口"),
            msg(3, "user", "好，按方案 A 来，但我还没决定表结构"),
        ];
        let r = compact_stub(None, &msgs, 1000, 900);
        assert!(r.summary.contains("方案 A"), "已拍板决策必须进摘要");
        assert!(r.summary.contains("stub"), "诚实标注 stub 压缩");
        assert_eq!(r.before_id, 3);
        assert!(r.before_pct > r.after_pct, "压缩后占比应下降");
    }

    #[test]
    fn stub_compact_appends_to_existing_summary() {
        let msgs = vec![msg(4, "user", "新问题")];
        let r = compact_stub(Some("既有决策：用方案 B"), &msgs, 1000, 100);
        assert!(r.summary.contains("既有决策：用方案 B"), "增量压缩不得丢既有摘要");
        assert!(r.summary.contains("新问题"));
    }
}
