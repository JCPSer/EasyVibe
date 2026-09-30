//! ai-agent 领域：Supervisor 直调 LLM 的槽位（M2-4 先落地巡检槽位）。
//! 与 easyvibe-session 的分工：session 管"外部 CLI agent"（写路径归纳），
//! ai-agent 管"后端自己调 LLM API"（巡检/审查等槽位，PRD 5.2 的多 LLM 服务配置）。
use easyvibe_common::ApiError;
use easyvibe_db::{FinishPatrolRun, HealthRepository, ModuleHealthRow, NewPatrolRun};
use easyvibe_map::{atomic_write_json, validate_strict};
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::{info, warn};

pub mod compaction;

// ---------- LLM 客户端（trait：Anthropic 兼容实现 + Stub 实现） ----------

pub struct ChatRequest<'a> {
    pub system: &'a str,
    pub user: &'a str,
}

/// LLM 调用结果：文本 + token 用量（§10 #4 成本护栏第一步的原始口径 §11 🟢10）
#[derive(Debug, Clone, Default)]
pub struct ChatOutcome {
    pub text: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

/// 无分词器依赖的 token 估算：CJK 混合文本约 1 token / 2 字符（保守取整）。
/// 用于压缩阈值判断与消息 token 记账；真实用量以 API 返回的 usage 为准。
pub fn estimate_tokens(s: &str) -> i64 {
    ((s.chars().count() as f64) / 2.0).ceil() as i64
}

/// LLM 服务客户端。真实实现走 Anthropic 兼容 API（/v1/messages）；
/// OpenAI 兼容服务通过 base_url 适配（M2-4.x 按需补 messages 格式分叉）。
pub trait LlmClient: Send + Sync {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<ChatOutcome, ApiError>;
    fn model(&self) -> &str;
}

pub struct AnthropicClient {
    base_url: String,
    api_key: String,
    model: String,
    max_tokens: u32,
    /// 附加请求头（某些中转/代理需要自定义头，如 x-opencode-session），JSON map
    extra_headers: Vec<(String, String)>,
}

impl AnthropicClient {
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
        let max_tokens = std::env::var("EASYVIBE_LLM_MAX_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8192);
        // EASYVIBE_LLM_HEADERS='{"x-opencode-session":"easyvibe"}'
        let extra_headers = std::env::var("EASYVIBE_LLM_HEADERS")
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&raw).ok())
            .map(|m| {
                m.into_iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            max_tokens,
            extra_headers,
        }
    }
}

impl LlmClient for AnthropicClient {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<ChatOutcome, ApiError> {
        #[derive(serde::Serialize)]
        struct Msg<'a> {
            role: &'a str,
            content: &'a str,
        }
        #[derive(serde::Serialize)]
        struct Body<'a> {
            model: &'a str,
            max_tokens: u32,
            system: &'a str,
            messages: [Msg<'a>; 1],
        }
        // 无超时会让会话永久 Running（审查 Y2）
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ApiError::Internal(format!("LLM client 构建失败: {e}")))?;
        let mut http_req = client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01");
        for (k, v) in &self.extra_headers {
            http_req = http_req.header(k, v);
        }
        let body = Body { model: &self.model, max_tokens: self.max_tokens, system: req.system, messages: [Msg { role: "user", content: req.user }] };
        let resp = http_req.json(&body).send()
            .await
            .map_err(|e| ApiError::Internal(format!("LLM 请求失败: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Internal(format!("LLM {status}: {body:.300}")));
        }
        let json: Value = resp.json().await.map_err(|e| ApiError::Internal(format!("LLM 响应解析失败: {e}")))?;
        // Anthropic messages 格式：content: [{type:"text", text:"..."}]
        let text = json["content"]
            .as_array()
            .and_then(|arr| arr.iter().find_map(|c| c["text"].as_str()))
            .ok_or_else(|| ApiError::Internal("LLM 响应缺少 content.text".into()))?;
        // usage: {input_tokens, output_tokens}（部分代理可能不返回——记 0，记账不阻断）
        let prompt_tokens = json["usage"]["input_tokens"].as_u64().unwrap_or(0);
        let completion_tokens = json["usage"]["output_tokens"].as_u64().unwrap_or(0);
        Ok(ChatOutcome { text: text.to_string(), prompt_tokens, completion_tokens })
    }

    fn model(&self) -> &str {
        &self.model
    }
}

/// Stub 客户端：零网络成本端到端验证巡检链路。
/// 从 user prompt 中提取 <CURRENT_MAP>，健康分 +1（cap 100），更新 last_patrol_at。
pub struct StubLlmClient {
    model: String,
}

impl StubLlmClient {
    pub fn new() -> Self {
        Self { model: "stub-patrol".into() }
    }
}

impl LlmClient for StubLlmClient {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<ChatOutcome, ApiError> {
        let start_tag = "<CURRENT_MAP_EMBED>";
        let end_tag = "</CURRENT_MAP_EMBED>";
        let start = req.user.find(start_tag).ok_or_else(|| ApiError::Internal("stub: 找不到 CURRENT_MAP 起始标记".into()))?;
        let rest = &req.user[start + start_tag.len()..];
        let end = rest.find(end_tag).ok_or_else(|| ApiError::Internal("stub: 找不到 CURRENT_MAP 结束标记".into()))?;
        let raw = rest[..end].trim();
        let mut map: Value = serde_json::from_str(raw).map_err(|e| ApiError::Internal(format!("stub: 提取地图失败: {e}")))?;
        if let Some(score) = map["health"]["score"].as_i64() {
            map["health"]["score"] = serde_json::json!((score + 1).min(100));
        }
        map["meta"]["last_patrol_at"] = serde_json::json!(now_iso());
        Ok(ChatOutcome {
            text: serde_json::to_string(&map).unwrap(),
            prompt_tokens: estimate_tokens(req.system) as u64 + estimate_tokens(req.user) as u64,
            completion_tokens: estimate_tokens(&serde_json::to_string(&map).unwrap()) as u64,
        })
    }

    fn model(&self) -> &str {
        &self.model
    }
}

fn now_iso() -> String {
    // 无 chrono 依赖的简易 ISO 时间（秒级，本地时区近似 UTC 展示格式）
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{secs}")
}

// ---------- 巡检槽位 ----------

pub struct PatrolService<R: HealthRepository> {
    repo_health: Arc<R>,
    counter: AtomicU64,
}

impl<R: HealthRepository> PatrolService<R> {
    pub fn new(repo_health: Arc<R>) -> Self {
        Self { repo_health, counter: AtomicU64::new(0) }
    }

    /// 执行一次巡检：LLM 产出新地图 → 校验 → 原子写回 → 健康历史落库
    pub async fn run<L: LlmClient>(
        &self,
        run_id: Option<String>,
        repo_id: &str,
        repo_root: &Path,
        current_map: &Value,
        prompt_template: &str,
        schema_path: &str,
        llm: &L,
    ) -> Result<PatrolSummary, ApiError> {
        let run_id = run_id.unwrap_or_else(|| format!("patrol-{}", self.counter.fetch_add(1, Ordering::SeqCst)));
        self.repo_health
            .create_run(&NewPatrolRun { id: run_id.clone(), repo: repo_id.to_string(), started_at: now_iso(), model: llm.model().to_string() })
            .await?;

        let result = self
            .run_inner(&run_id, repo_id, repo_root, current_map, prompt_template, schema_path, llm)
            .await;

        match result {
            Ok((summary, pt, ct)) => {
                self.repo_health
                    .finish_run(&FinishPatrolRun {
                        id: run_id.clone(),
                        finished_at: now_iso(),
                        status: "succeeded".into(),
                        arch_score: Some(summary.arch_score as i64),
                        error: None,
                        prompt_tokens: Some(pt as i64),
                        completion_tokens: Some(ct as i64),
                    })
                    .await?;
                Ok(summary)
            }
            Err(e) => {
                self.repo_health
                    .finish_run(&FinishPatrolRun {
                        id: run_id.clone(),
                        finished_at: now_iso(),
                        status: "failed".into(),
                        arch_score: None,
                        error: Some(e.to_string()),
                        prompt_tokens: None,
                        completion_tokens: None,
                    })
                    .await?;
                Err(e)
            }
        }
    }

    async fn run_inner<L: LlmClient>(
        &self,
        run_id: &str,
        repo_id: &str,
        repo_root: &Path,
        current_map: &Value,
        prompt_template: &str,
        schema_path: &str,
        llm: &L,
    ) -> Result<(PatrolSummary, u64, u64), ApiError> {
        let user = prompt_template
            .replace("<REPO_ROOT>", &repo_root.to_string_lossy())
            .replace("<SCHEMA_PATH>", schema_path)
            .replace(
                "<CURRENT_MAP>",
                &format!("<CURRENT_MAP_EMBED>\n{}\n</CURRENT_MAP_EMBED>", serde_json::to_string(current_map).unwrap_or_default()),
            );
        let outcome = llm.chat(ChatRequest { system: "你是 EasyVibe 巡检 Agent，只输出 JSON 本身。", user: &user }).await?;
        let raw = outcome.text;

        let new_map = extract_json(&raw)?;
        validate_strict(&new_map).map_err(|e| ApiError::MapInvalid(format!("巡检产物未通过严格验收: {e}")))?;

        // 健康历史落库（先库后文件：库失败不覆盖合法地图）
        self.persist_module_health(run_id, &new_map).await?;

        atomic_write_json(&repo_root.join(".easyvibe/map/map.json"), &new_map).await?;
        info!("[patrol {run_id}] {repo_id} 地图已更新（watcher 将推送 map.changed）");

        let arch_score = new_map["health"]["score"].as_u64().unwrap_or(0) as u32;
        Ok((PatrolSummary { run_id: run_id.to_string(), modules: module_count(&new_map), arch_score }, outcome.prompt_tokens, outcome.completion_tokens))
    }

    /// 会话型巡检（session spawn）终态后的健康历史落库：建 run → 插模块行 → 收尾
    pub async fn record_from_map(
        &self,
        run_id: &str,
        repo_id: &str,
        model: &str,
        map: &Value,
        succeeded: bool,
        error: Option<String>,
    ) -> Result<(), ApiError> {
        self.repo_health
            .create_run(&NewPatrolRun { id: run_id.to_string(), repo: repo_id.to_string(), started_at: now_iso(), model: model.to_string() })
            .await?;
        if succeeded {
            self.persist_module_health(run_id, map).await?;
        }
        self.repo_health
            .finish_run(&FinishPatrolRun {
                id: run_id.to_string(),
                finished_at: now_iso(),
                status: if succeeded { "succeeded".into() } else { "failed".into() },
                arch_score: map["health"]["score"].as_i64(),
                error,
                prompt_tokens: None,     // 会话型 CLI agent 无法回报 usage（M3-5 记档）
                completion_tokens: None,
            })
            .await?;
        Ok(())
    }

    async fn persist_module_health(&self, run_id: &str, map: &Value) -> Result<(), ApiError> {
        let Some(mods) = map["modules"].as_array() else { return Ok(()) };
        for m in mods {
            let health = &m["health"];
            let row = ModuleHealthRow {
                run_id: run_id.to_string(),
                module_id: m["id"].as_str().unwrap_or("?").to_string(),
                name: m["name"].as_str().map(Into::into),
                score: health["score"].as_i64().unwrap_or(0),
                coupling: health["coupling"].as_str().map(Into::into),
                complexity: health["complexity"].as_str().map(Into::into),
                churn: health["churn"].as_str().map(Into::into),
                decay_flags: serde_json::to_string(&health["decay_flags"]).unwrap_or_else(|_| "[]".into()),
                review_note: health["review_note"].as_str().map(Into::into),
                concerns: serde_json::to_string(&health["concerns"]).unwrap_or_else(|_| "[]".into()),
            };
            self.repo_health.insert_module_health(&row).await?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PatrolSummary {
    pub run_id: String,
    pub modules: usize,
    pub arch_score: u32,
}

fn module_count(map: &Value) -> usize {
    map["modules"].as_array().map(|a| a.len()).unwrap_or(0)
}

/// 从 LLM 输出中提取 JSON（容忍 ```json 围栏与前后缀文字）
pub fn extract_json(raw: &str) -> Result<Value, ApiError> {
    let trimmed = raw.trim();
    let candidate = if trimmed.starts_with('{') {
        trimmed
    } else {
        // 找第一个 '{' 开始的平衡括号片段（容忍前导文字）
        let start = trimmed.find('{').ok_or_else(|| ApiError::MapInvalid("输出中找不到 JSON 起始".into()))?;
        &trimmed[start..]
    };
    // 围栏剥离
    let candidate = candidate.trim_start_matches("```json").trim_start_matches("```").trim();
    match serde_json::from_str::<Value>(candidate) {
        Ok(v) => Ok(v),
        Err(e) => {
            warn!("extract_json 直接解析失败，尝试括号截取: {e}");
            bracket_extract(candidate).ok_or_else(|| ApiError::MapInvalid(format!("JSON 解析失败: {e}")))
        }
    }
}

fn bracket_extract(s: &str) -> Option<Value> {
    let bytes = s.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        match b {
            b'"' if !esc => in_str = !in_str,
            b'\\' if in_str => esc = !esc,
            b'{' if !in_str => depth += 1,
            b'}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&s[start..=i]).ok();
                }
            }
            _ => esc = false,
        }
        if b != b'\\' { esc = false; }
    }
    None
}

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
    async fn ask(&self, map: &Value, question: &str, history: &[(String, String)]) -> Result<QaAnswer, ApiError>;
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
    async fn ask(&self, map: &Value, question: &str, _history: &[(String, String)]) -> Result<QaAnswer, ApiError> {
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
    async fn ask(&self, map: &Value, question: &str, history: &[(String, String)]) -> Result<QaAnswer, ApiError> {
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
        let raw = self.llm.chat(ChatRequest { system: &system, user: &user }).await?;
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

// ---------- 智能优化建议（M3-2 补充：AI 主动发现优化机会，一键转修复任务） ----------

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub title: String,
    pub description: String,
    pub modules: Vec<String>,
    pub priority: String, // critical / high / medium
    pub rationale: String,
}

pub trait SuggestClient: Send + Sync {
    async fn suggest(&self, map: &Value) -> Result<Vec<Suggestion>, ApiError>;
}

/// 确定性建议（Stub）：从地图数据派生优化机会——concerns、腐化标记、违规热点聚类
pub struct StubSuggestClient;

impl SuggestClient for StubSuggestClient {
    async fn suggest(&self, map: &Value) -> Result<Vec<Suggestion>, ApiError> {
        Ok(stub_suggest(map))
    }
}

pub fn stub_suggest(map: &Value) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = vec![];
    let Some(mods) = map["modules"].as_array() else { return out };

    // 1) 模块级 concerns → 建议
    for m in mods {
        for c in m["health"]["concerns"].as_array().into_iter().flatten().take(2) {
            out.push(Suggestion {
                title: format!("{}：{}", m["name"].as_str().unwrap_or("?"), &c["finding"].as_str().unwrap_or("").chars().take(36).collect::<String>()),
                description: format!("{}
建议：{}", c["finding"].as_str().unwrap_or(""), c["suggestion"].as_str().unwrap_or("")),
                modules: vec![m["id"].as_str().unwrap_or("?").to_string()],
                priority: c["severity"].as_str().unwrap_or("high").to_string(),
                rationale: "来自巡检问题提名".to_string(),
            });
        }
    }
    // 2) 违规热点：涉及逆向依赖最多的模块 → 治理建议
    let mut violation_count: std::collections::HashMap<String, usize> = Default::default();
    for e in map["edges"].as_array().into_iter().flatten() {
        if e["direction_violation"].as_bool() == Some(true) {
            for ep in ["from", "to"] {
                if let Some(id) = e[ep].as_str() {
                    *violation_count.entry(id.to_string()).or_default() += 1;
                }
            }
        }
    }
    let mut hot: Vec<_> = violation_count.into_iter().collect();
    hot.sort_by(|a, b| b.1.cmp(&a.1));
    for (id, n) in hot.into_iter().take(2) {
        if n < 2 { continue }
        if let Some(m) = mods.iter().find(|m| m["id"].as_str() == Some(id.as_str())) {
            out.push(Suggestion {
                title: format!("{}：消除 {} 条逆向依赖", m["name"].as_str().unwrap_or("?"), n),
                description: format!("该模块处于 {} 条逆向依赖（direction_violation）上，是方向约束的主要破坏点。建议按模块详情中的 review_note 收敛依赖方向。", n),
                modules: vec![id],
                priority: "high".into(),
                rationale: "违规热点聚类（确定性统计）".into(),
            });
        }
    }
    // 3) 架构级 concerns → 建议（影响面最大）
    for c in map["health"]["concerns"].as_array().into_iter().flatten().take(1) {
        let affected: Vec<String> = mods.iter().filter(|m| m["health"]["score"].as_i64().unwrap_or(100) < 70).take(3).map(|m| m["id"].as_str().unwrap_or("?").to_string()).collect();
        out.push(Suggestion {
            title: format!("架构级：{}", &c["finding"].as_str().unwrap_or("").chars().take(40).collect::<String>()),
            description: format!("{}
建议：{}", c["finding"].as_str().unwrap_or(""), c["suggestion"].as_str().unwrap_or("")),
            modules: affected,
            priority: c["severity"].as_str().unwrap_or("critical").to_string(),
            rationale: "架构级问题（LLM 独立评估，优先级天然最高）".into(),
        });
    }
    // 优先级排序 + 限量
    let rank = |p: &str| match p { "critical" => 0, "high" => 1, _ => 2 };
    out.sort_by_key(|x| rank(&x.priority));
    out.truncate(8);
    out
}

/// LLM 建议实现：地图注入，要求输出 JSON 数组 [{title, description, modules, priority, rationale}]
pub struct LlmSuggestClient<C: LlmClient> {
    llm: C,
}

impl<C: LlmClient> LlmSuggestClient<C> {
    pub fn new(llm: C) -> Self {
        Self { llm }
    }
}

impl<C: LlmClient> SuggestClient for LlmSuggestClient<C> {
    async fn suggest(&self, map: &Value) -> Result<Vec<Suggestion>, ApiError> {
        let system = "你是 EasyVibe 优化顾问。基于语义代码地图，主动发现最值得做的架构优化（不局限于已有 concerns：也看职责重叠、分层错位、健康度洼地）。只输出 JSON 数组本身，每项 {title, description, modules, priority(critical/high/medium), rationale}，至多 6 项，按优先级排序。";
        let user = format!("## 语义代码地图
```json
{}
```

给出你的优化建议列表。", serde_json::to_string(map).unwrap_or_default());
        let raw = self.llm.chat(ChatRequest { system, user: &user }).await?;
        let arr = extract_json_array(&raw.text)?;
        let mut out = vec![];
        for it in arr.iter().filter_map(|x| x.as_object()) {
            out.push(Suggestion {
                title: it["title"].as_str().unwrap_or("未命名建议").to_string(),
                description: it["description"].as_str().unwrap_or_default().to_string(),
                modules: it["modules"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(Into::into)).collect()).unwrap_or_default(),
                priority: it["priority"].as_str().unwrap_or("medium").to_string(),
                rationale: it["rationale"].as_str().unwrap_or_default().to_string(),
            });
        }
        Ok(out)
    }
}

/// 从 LLM 输出提取 JSON 数组（容忍围栏/前后缀）
pub fn extract_json_array(raw: &str) -> Result<Vec<Value>, ApiError> {
    let t = raw.trim();
    let candidate = if t.starts_with('[') { t } else { &t[t.find('[').ok_or_else(|| ApiError::MapInvalid("输出中找不到 JSON 数组起始".into()))?..] };
    let candidate = candidate.trim_start_matches("```json").trim_start_matches("```").trim();
    if let Ok(v) = serde_json::from_str::<Vec<Value>>(candidate) {
        return Ok(v);
    }
    // 括号截取
    let bytes = candidate.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    let start = bytes.iter().position(|&b| b == b'[').unwrap_or(0);
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        match b {
            b'"' if !esc => in_str = !in_str,
            b'[' if !in_str => depth += 1,
            b']' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&candidate[start..=i]).map_err(|e| ApiError::MapInvalid(format!("JSON 数组解析失败: {e}")));
                }
            }
            _ => {}
        }
        esc = b == b'\\' && !esc;
        if b != b'\\' { esc = false; }
    }
    Err(ApiError::MapInvalid("JSON 数组括号不配对".into()))
}

#[cfg(test)]
mod qa_tests {
    use super::*;
    use easyvibe_db::{Database, SqliteHealthRepository};
    use serde_json::json;

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
        let a = qa.ask(&map, "我想给系统新增支付功能", &[]).await.unwrap();
        assert!(a.clarify.is_some(), "改动意图应触发澄清卡");
        assert!(!a.clarify.unwrap().options.is_empty());
        // 普通提问 → 无澄清卡
        let b = qa.ask(&map, "评测提交流程是谁负责的？", &[]).await.unwrap();
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

    fn sample_map() -> Value {
        json!({
            "version": "1.0",
            "meta": {"repo": "demo", "generated_at": "t", "generator": "g/test"},
            "layers": [
                {"id": "app-entry", "name": "应用装配层", "order": 0, "description": "d"},
                {"id": "application", "name": "应用服务层", "order": 2, "description": "d"}
            ],
            "modules": [{
                "id": "exam-core", "name": "考试与评测核心", "layer": "application",
                "responsibility": "考试会话编排、答题流程与评测提交管线",
                "files": ["lib/application/exam/**"], "key_entries": [],
                "dependencies": [],
                "health": {"score": 64, "coupling": "high", "complexity": "high",
                           "churn": "medium", "decay_flags": ["circular_dep"],
                           "review_note": "与 reporting 双向耦合",
                           "concerns": [{"severity": "critical", "finding": "双向耦合", "suggestion": "改单向"}]}
            }],
            "edges": [],
            "health": {"score": 58, "coupling": "high", "complexity": "high",
                       "churn": "high", "decay_flags": ["layer_violation"], "review_note": "三类系统性问题",
                       "concerns": []}
        })
    }

    fn prompt() -> String {
        "巡检 <REPO_ROOT> <SCHEMA_PATH>\n<CURRENT_MAP>\n```".to_string()
    }

    #[tokio::test]
    async fn stub_patrol_full_pipeline() {
        let db = Database::connect_memory().await.unwrap();
        let health = Arc::new(SqliteHealthRepository::new(db.pool().clone()));
        let svc = PatrolService::new(health.clone());

        let dir = std::env::temp_dir().join("ev-patrol-test");
        tokio::fs::create_dir_all(dir.join(".easyvibe/map")).await.unwrap();
        let current = sample_map();
        tokio::fs::write(dir.join(".easyvibe/map/map.json"), serde_json::to_string(&current).unwrap()).await.unwrap();

        let llm = StubLlmClient::new();
        let summary = svc
            .run(None, "demo", &dir, &current, &prompt(), "/tmp/schema.json", &llm)
            .await
            .unwrap();

        assert_eq!(summary.modules, 1);
        assert_eq!(summary.arch_score, 59); // 58 + 1

        // 文件已原子写回
        let written: Value =
            serde_json::from_str(&tokio::fs::read_to_string(dir.join(".easyvibe/map/map.json")).await.unwrap()).unwrap();
        assert_eq!(written["health"]["score"], 59);

        // 历史落库
        let runs = health.list_runs("demo", 10).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "succeeded");
        assert_eq!(runs[0].arch_score, Some(59));
        let history = health.list_module_history("demo", "exam-core", 10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].score, 64);
    }

    #[test]
    fn stub_suggest_prioritizes() {
        let map = sample_map();
        let sug = stub_suggest(&map);
        assert!(!sug.is_empty());
        // 样例含 1 个模块 concern（critical）+ 架构 concern，critical 应排最前
        assert_eq!(sug[0].priority, "critical");
        assert!(sug.iter().any(|s| s.rationale.contains("违规") || s.rationale.contains("提名") || s.rationale.contains("架构")));
    }

    #[test]
    fn extract_json_array_tolerant() {
        let raw = "前言\n```json\n[{\"title\":\"t\",\"modules\":[\"m1\"]}]\n```";
        let arr = extract_json_array(raw).unwrap();
        assert_eq!(arr[0]["title"], "t");
    }

    #[test]
    fn extract_json_tolerant() {
        let raw = "前言\n```json\n{\"a\":1}\n```\n后记";
        assert_eq!(extract_json(raw).unwrap()["a"], 1);
        let raw2 = "结果： {\"b\": 2} 完";
        assert_eq!(extract_json(raw2).unwrap()["b"], 2);
    }
}
