//! LLM 客户端槽位：ChatRequest/ChatOutcome 契约、token 估算、trait 与两种实现
//! （Anthropic 兼容 HTTP 实现 + Stub 无网络实现）。拆自 lib.rs（2026-10-05 防膨胀）。

use easyvibe_common::ApiError;
use serde_json::Value;

// ---------- LLM 客户端（trait：Anthropic 兼容实现 + Stub 实现） ----------

pub struct ChatRequest<'a> {
    pub system: &'a str,
    pub user: &'a str,
    /// S3 附件：dataURL 格式的图片（data:image/png;base64,...）；仅支持图像的模型消费
    pub images: &'a [String],
}

impl<'a> Default for ChatRequest<'a> {
    fn default() -> Self {
        Self { system: "", user: "", images: &[] }
    }
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
        /// content：纯文本为 JSON string；含图片为 block 数组（[text, image...]）
        #[derive(serde::Serialize)]
        struct Msg {
            role: &'static str,
            content: serde_json::Value,
        }
        #[derive(serde::Serialize)]
        struct Body<'a> {
            model: &'a str,
            max_tokens: u32,
            system: &'a str,
            messages: [Msg; 1],
        }
        // 无超时会让会话永久 Running（审查 Y2）
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ApiError::Internal(format!("LLM client 构建失败: {e}")))?;
        // 认证头双发：Anthropic 官方与多数网关认 x-api-key；
        // opencode 系网关（opencode.ai/inference、token-plan 等转发）只认 Authorization: Bearer。
        // 两者并存对官方 API 无副作用（多余头被忽略），对 passthrough 代理原样透传。
        let mut http_req = client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01");
        if !self.api_key.is_empty() {
            http_req = http_req.header("authorization", format!("Bearer {}", self.api_key));
        }
        for (k, v) in &self.extra_headers {
            http_req = http_req.header(k, v);
        }
        // dataURL 解析：data:image/png;base64,<data> → (media_type, data)（不合法的跳过，不阻断对话）
        let mut blocks: Vec<(String, String)> = vec![];
        for url in req.images {
            let Some(rest) = url.strip_prefix("data:") else { continue };
            let Some((meta, data)) = rest.split_once(";base64,") else { continue };
            if !meta.starts_with("image/") {
                continue;
            }
            blocks.push((meta.to_string(), data.to_string()));
        }
        let content = if blocks.is_empty() {
            serde_json::Value::String(req.user.to_string())
        } else {
            let mut arr = vec![serde_json::json!({ "type": "text", "text": req.user })];
            for (media_type, data) in blocks {
                arr.push(serde_json::json!({
                    "type": "image",
                    "source": { "type": "base64", "media_type": media_type, "data": data },
                }));
            }
            serde_json::Value::Array(arr)
        };
        let body = Body {
            model: &self.model,
            max_tokens: self.max_tokens,
            system: req.system,
            messages: [Msg { role: "user", content }],
        };
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

pub(crate) fn now_iso() -> String {
    // 无 chrono 依赖的简易 ISO 时间（秒级，本地时区近似 UTC 展示格式）
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{secs}")
}