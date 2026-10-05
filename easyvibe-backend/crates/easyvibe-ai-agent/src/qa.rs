//! 对话问答槽位（F2）：QaClient 契约、Stub 确定性地图检索、LLM 地图上下文注入、
//! 澄清卡协议解析。拆自 lib.rs（2026-10-05 防膨胀）。

use crate::client::{estimate_tokens, ChatRequest, LlmClient};
use crate::extract_json;
use easyvibe_common::ApiError;
use serde_json::Value;

// ---------- 对话问答槽位（F2，M2-5） ----------

/// 问答结果：回复文本 + 引用到的模块 id（供"存为视图"与画布定位消费）+ token 用量（M3-5）
/// + 澄清卡（S1：grill-me 选择题协议——需求有歧义时的结构化拷问）
#[derive(Debug, Clone)]
pub struct QaAnswer {
    pub reply: String,
    pub refs: Vec<String>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub clarify: Option<Clarify>,
}

impl QaAnswer {
    fn plain(reply: String, refs: Vec<String>) -> Self {
        Self { reply, refs, prompt_tokens: 0, completion_tokens: 0, clarify: None }
    }
}

/// 澄清卡：选择题形态（inject-prompt.md"提问时尽量使用选择题"的产品化）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Clarify {
    pub question: String,
    pub options: Vec<ClarifyOption>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ClarifyOption {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
}

/// 解析回复尾部的 `[clarify: {...}]` 标记（与 [refs:] 同构的协议；返回剥离后的文本）
/// JSON 体内可能含 `]`（options 数组），所以不能找第一个 `]` 收尾——用平衡括号扫描。
pub fn parse_clarify(raw: &str) -> (String, Option<Clarify>) {
    let Some(pos) = raw.rfind("[clarify:") else { return (raw.trim().to_string(), None) };
    let start = pos + "[clarify:".len();
    let Some(obj_start) = raw[start..].find('{').map(|o| start + o) else {
        return (raw.trim().to_string(), None);
    };
    let bytes = raw.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    let mut obj_end = None;
    for i in obj_start..bytes.len() {
        match bytes[i] {
            b'"' if !esc => in_str = !in_str,
            b'\\' if in_str => esc = !esc,
            b'{' if !in_str => depth += 1,
            b'}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    obj_end = Some(i);
                    break;
                }
            }
            _ => {}
        }
        if bytes[i] != b'\\' {
            esc = false;
        }
    }
    let Some(end) = obj_end else { return (raw.trim().to_string(), None) };
    let payload = &raw[obj_start..=end];
    let Ok(v) = serde_json::from_str::<Value>(payload).or_else(|_| extract_json(payload)) else {
        return (raw.trim().to_string(), None);
    };
    let question = v["question"].as_str().unwrap_or_default().trim().to_string();
    if question.is_empty() {
        return (raw.trim().to_string(), None);
    }
    let options: Vec<ClarifyOption> = v["options"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    let label = o["label"].as_str()?.trim().to_string();
                    if label.is_empty() { None } else {
                        Some(ClarifyOption { label, desc: o["desc"].as_str().map(Into::into) })
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    if options.is_empty() {
        return (raw.trim().to_string(), None);
    }
    let clarify = Clarify { question, options, why: v["why"].as_str().map(Into::into) };
    (raw[..pos].trim().to_string(), Some(clarify))
}

/// 对话槽位客户端。Stub = 确定性地图检索（零成本、回答真实基于地图）；
/// LLM 实现 = 地图上下文注入（M2-5 范围：基于地图回答；代码级追问留待 M3 工具能力）。
pub trait QaClient: Send + Sync {
    async fn ask(&self, map: &Value, question: &str, history: &[(String, String)], images: &[String]) -> Result<QaAnswer, ApiError>;
    fn model(&self) -> &str;
}

pub struct StubQaClient {
    model: String,
}

impl StubQaClient {
    pub fn new() -> Self {
        Self { model: "stub-qa".into() }
    }
}

impl QaClient for StubQaClient {
    async fn ask(&self, map: &Value, question: &str, _history: &[(String, String)], _images: &[String]) -> Result<QaAnswer, ApiError> {
        let mut ans = stub_answer(map, question);
        // stub 无真实 API usage，用估算值记账（让成本护栏链路在零成本模式下也可验证）
        ans.prompt_tokens = (estimate_tokens(question) + estimate_tokens(&serde_json::to_string(map).unwrap_or_default())) as u64;
        ans.completion_tokens = estimate_tokens(&ans.reply) as u64;
        // S1：确定性澄清触发——含改动意图关键词即返回选择题卡（零成本验证澄清链路）
        if ["新增", "加功能", "改造", "重构"].iter().any(|k| question.contains(k)) {
            ans.clarify = Some(Clarify {
                question: "改动范围如何界定？".into(),
                options: vec![
                    ClarifyOption { label: "仅涉及地图命中的模块".into(), desc: Some("最小改动面，风险可控".into()) },
                    ClarifyOption { label: "允许跨模块调整".into(), desc: Some("架构级改动，建议走手动三道关".into()) },
                ],
                why: Some("stub 模式按关键词确定性触发".into()),
            });
        }
        Ok(ans)
    }

    fn model(&self) -> &str {
        &self.model
    }
}

/// 确定性地图问答：按问题中的关键词（模块 id/名称/职责/key_entries 命中）检索，
/// 生成基于地图事实的回答；命中"架构/健康/分层"等词时输出架构级摘要。
pub fn stub_answer(map: &Value, question: &str) -> QaAnswer {
    let q = question.to_lowercase();
    let Some(mods) = map["modules"].as_array() else {
        return QaAnswer::plain("地图中没有模块信息。".into(), vec![]);
    };

    // 架构级意图
    let arch_intent = ["架构", "分层", "整体", "健康", "评分", "违规", "逆向", "耦合", "最"]
        .iter()
        .any(|k| q.contains(k));

    let mut scored: Vec<(usize, &Value)> = mods
        .iter()
        .map(|m| {
            let mut score = 0usize;
            let id = m["id"].as_str().unwrap_or("");
            let name = m["name"].as_str().unwrap_or("");
            let resp = m["responsibility"].as_str().unwrap_or("");
            if !id.is_empty() && q.contains(&id.to_lowercase()) { score += 3; }
            for seg in name.chars().collect::<Vec<_>>().chunks(2) {
                let bigram: String = seg.iter().collect();
                if bigram.chars().count() == 2 && q.contains(&bigram) { score += 2; }
            }
            for kw in ["提交", "评测", "考试", "报告", "账户", "练习", "发音", "题库", "音频", "录音", "网关", "存储", "更新", "登录", "支付"] {
                if q.contains(kw) && (resp.contains(kw) || name.contains(kw)) { score += 2; }
            }
            for ke in m["key_entries"].as_array().into_iter().flatten() {
                if let Some(sym) = ke["symbol"].as_str() {
                    let sym_l = sym.to_lowercase();
                    if sym_l.len() > 3 && q.contains(&sym_l) { score += 3; }
                }
            }
            (score, m)
        })
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.truncate(3);

    let layer_name = |id: &str| {
        map["layers"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|l| l["id"].as_str() == Some(id))
            .and_then(|l| l["name"].as_str())
            .unwrap_or(id)
            .to_string()
    };

    if scored.is_empty() && !arch_intent {
        return QaAnswer::plain(
            "根据现有语义地图没有找到直接相关的模块。可以换个问法（提及模块名或职责关键词），或先对仓库重新归纳以获得更完整的地图。".into(),
            vec![],
        );
    }

    let mut parts: Vec<String> = vec![];
    let mut refs: Vec<String> = vec![];
    for (_, m) in &scored {
        let id = m["id"].as_str().unwrap_or("?");
        let name = m["name"].as_str().unwrap_or(id);
        let resp = m["responsibility"].as_str().unwrap_or("");
        let score = m["health"]["score"].as_i64().unwrap_or(0);
        let flags = m["health"]["decay_flags"].as_array().map(|a| a.len()).unwrap_or(0);
        let layer = layer_name(m["layer"].as_str().unwrap_or(""));
        let concern = m["health"]["concerns"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|c| c["finding"].as_str())
            .unwrap_or("");
        let mut line = format!("**{name}**（{layer}）：{resp}。健康度 {score} 分");
        if flags > 0 { line.push_str(&format!("，有 {flags} 个腐化标记")); }
        line.push('。');
        if !concern.is_empty() { line.push_str(&format!("值得关注：{concern}。")); }
        parts.push(line);
        refs.push(id.to_string());
    }

    if arch_intent {
        let arch = &map["health"];
        let arch_score = arch["score"].as_i64().unwrap_or(0);
        let arch_note = arch["review_note"].as_str().unwrap_or("");
        parts.insert(0, format!("整体架构健康 **{arch_score} 分**（耦合 {})：{}。", arch["coupling"].as_str().unwrap_or("?"), arch_note));
    }

    if refs.is_empty() && arch_intent {
        refs.clear();
    }

    QaAnswer::plain(parts.join("\n\n"), refs)
}

/// LLM 问答实现：地图上下文注入 + 引用模块 id 的要求（M2-5：基于地图回答）
pub struct LlmQaClient<C: LlmClient> {
    llm: C,
    max_history: usize,
    /// S1-3：system 前缀（user_entry 插槽 skill 注入通道——§9 #4 仅用户入口对话）
    system_prefix: String,
}

impl<C: LlmClient> LlmQaClient<C> {
    pub fn new(llm: C) -> Self {
        Self { llm, max_history: 10, system_prefix: String::new() }
    }

    pub fn new_with_prefix(llm: C, system_prefix: &str) -> Self {
        Self { llm, max_history: 10, system_prefix: system_prefix.to_string() }
    }
}

impl<C: LlmClient> QaClient for LlmQaClient<C> {
    async fn ask(&self, map: &Value, question: &str, history: &[(String, String)], images: &[String]) -> Result<QaAnswer, ApiError> {
        let hist: Vec<String> = history
            .iter()
            .rev()
            .take(self.max_history)
            .rev()
            .map(|(q, a)| format!("Q: {q}\nA: {a}"))
            .collect();
        let system = format!(
            "{prefix}你是 EasyVibe 入口对话 Agent。基于给定的语义代码地图回答用户关于代码库的问题：模块职责、依赖关系、健康问题、架构分层。规则：1) 只依据地图事实回答，不确定就说不知道；2) 回答末尾用 [refs: 模块id1, 模块id2] 标出引用到的模块（最多 3 个，没有则写 none）；3) 简明直接，先给结论；4) 若需求意图明显（用户想改代码/加功能）且关键信息缺失，先出一道选择题澄清（给出建议答案，单题、最多两题），澄清标记格式：回复末尾另起一行输出 [clarify: {{\"question\": \"问题\", \"options\": [{{\"label\": \"选项\", \"desc\": \"建议理由\"}}], \"why\": \"为什么问\"}}]——只输出 JSON 本体在标记内；信息已足够时直接回答并在结尾建议'可点上方'转为任务'发起修复'。",
            prefix = self.system_prefix
        );
        let user = format!(
            "## 语义代码地图\n```json\n{}\n```\n\n## 最近对话\n{}\n\n## 用户问题\n{}",
            serde_json::to_string(map).unwrap_or_default(),
            if hist.is_empty() { "（无）".into() } else { hist.join("\n\n") },
            question
        );
        let raw = self.llm.chat(ChatRequest { system: &system, user: &user, images }).await?;
        // S1：先剥澄清卡标记，再剥 refs（两标记同存时互不干扰）
        let (text, clarify) = parse_clarify(&raw.text);
        let (reply, refs) = match text.rfind("[refs:") {
            Some(pos) => {
                let end = text[pos..].find(']').map(|e| pos + e).unwrap_or(text.len());
                let tag = &text[pos + 6..end];
                let refs: Vec<String> = tag
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty() && s != "none")
                    .collect();
                (text[..pos].trim().to_string(), refs)
            }
            None => (text.trim().to_string(), vec![]),
        };
        Ok(QaAnswer { reply, refs, prompt_tokens: raw.prompt_tokens, completion_tokens: raw.completion_tokens, clarify })
    }

    fn model(&self) -> &str {
        self.llm.model()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::sample_map;

    #[test]
    fn parse_clarify_marker_protocol() {
        // 合法标记：剥离 + 结构化
        let (text, c) = parse_clarify("先给一段分析。\n[clarify: {\"question\": \"改动范围？\", \"options\": [{\"label\": \"仅本模块\", \"desc\": \"最小改动\"}], \"why\": \"影响验收\"}]");
        assert_eq!(text, "先给一段分析。");
        let c = c.unwrap();
        assert_eq!(c.question, "改动范围？");
        assert_eq!(c.options.len(), 1);
        assert_eq!(c.options[0].label, "仅本模块");
        // 与 refs 共存：先 clarify 后 refs 的剥离顺序由调用方保证，parse_clarify 本身只认自己的标记
        let (t2, _) = parse_clarify("[refs: none]\n[clarify: {\"question\":\"q\",\"options\":[{\"label\":\"a\"}]}]");
        assert_eq!(t2, "[refs: none]");
        // 缺 options → 不算合法澄清（降级为普通文本，用户看到的是回复而非坏卡）
        let (t3, c3) = parse_clarify("回复[clarify: {\"question\": \"只有问题没有选项\"}]");
        assert!(c3.is_none());
        assert!(t3.contains("[clarify:"), "不合法标记保留原文（容错展示）");
        // 无标记
        let (t4, c4) = parse_clarify("普通回复");
        assert_eq!(t4, "普通回复");
        assert!(c4.is_none());
    }

    #[tokio::test]
    async fn stub_qa_triggers_clarify_on_change_intent() {
        let map = sample_map();
        let qa = StubQaClient::new();
        // 改动意图 → 澄清卡
        let a = qa.ask(&map, "我想给系统新增支付功能", &[], &[]).await.unwrap();
        assert!(a.clarify.is_some(), "改动意图应触发澄清卡");
        assert!(!a.clarify.unwrap().options.is_empty());
        // 普通提问 → 无澄清卡
        let b = qa.ask(&map, "评测提交流程是谁负责的？", &[], &[]).await.unwrap();
        assert!(b.clarify.is_none());
    }

    #[test]
    fn stub_qa_matches_modules_and_arch() {
        let map = sample_map();
        // 关键词命中模块
        let a = stub_answer(&map, "评测提交流程是谁负责的？");
        assert!(a.refs.contains(&"exam-core".to_string()), "应命中考试与评测核心, refs={:?}", a.refs);
        assert!(a.reply.contains("考试与评测核心"));
        // 架构级意图
        let b = stub_answer(&map, "整体架构健康怎么样？");
        assert!(b.reply.contains("架构健康"));
        assert!(b.reply.contains("58"));
        // 无命中
        let c = stub_answer(&map, "量子涨落是什么意思");
        assert!(c.refs.is_empty());
    }
}
