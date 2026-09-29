//! ai-agent 领域：Supervisor 直调 LLM 的槽位（M2-4 先落地巡检槽位）。
//! 与 easyvibe-session 的分工：session 管"外部 CLI agent"（写路径归纳），
//! ai-agent 管"后端自己调 LLM API"（巡检/审查等槽位，PRD 5.2 的多 LLM 服务配置）。
use easyvibe_common::ApiError;
use easyvibe_db::{FinishPatrolRun, HealthRepository, ModuleHealthRow, NewPatrolRun};
use easyvibe_map::{atomic_write_json, validate_minimum};
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::{info, warn};

// ---------- LLM 客户端（trait：Anthropic 兼容实现 + Stub 实现） ----------

pub struct ChatRequest<'a> {
    pub system: &'a str,
    pub user: &'a str,
}

/// LLM 服务客户端。真实实现走 Anthropic 兼容 API（/v1/messages）；
/// OpenAI 兼容服务通过 base_url 适配（M2-4.x 按需补 messages 格式分叉）。
pub trait LlmClient: Send + Sync {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<String, ApiError>;
    fn model(&self) -> &str;
}

pub struct AnthropicClient {
    base_url: String,
    api_key: String,
    model: String,
    max_tokens: u32,
}

impl AnthropicClient {
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            max_tokens: 8192,
        }
    }
}

impl LlmClient for AnthropicClient {
    async fn chat(&self, req: ChatRequest<'_>) -> Result<String, ApiError> {
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
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&Body { model: &self.model, max_tokens: self.max_tokens, system: req.system, messages: [Msg { role: "user", content: req.user }] })
            .send()
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
        Ok(text.to_string())
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
    async fn chat(&self, req: ChatRequest<'_>) -> Result<String, ApiError> {
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
        Ok(serde_json::to_string(&map).unwrap())
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
            Ok(summary) => {
                self.repo_health
                    .finish_run(&FinishPatrolRun {
                        id: run_id.clone(),
                        finished_at: now_iso(),
                        status: "succeeded".into(),
                        arch_score: Some(summary.arch_score as i64),
                        error: None,
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
    ) -> Result<PatrolSummary, ApiError> {
        let user = prompt_template
            .replace("<REPO_ROOT>", &repo_root.to_string_lossy())
            .replace("<SCHEMA_PATH>", schema_path)
            .replace(
                "<CURRENT_MAP>",
                &format!("<CURRENT_MAP_EMBED>\n{}\n</CURRENT_MAP_EMBED>", serde_json::to_string(current_map).unwrap_or_default()),
            );
        let raw = llm.chat(ChatRequest { system: "你是 EasyVibe 巡检 Agent，只输出 JSON 本身。", user: &user }).await?;

        let new_map = extract_json(&raw)?;
        validate_minimum(&new_map).map_err(|e| ApiError::MapInvalid(format!("巡检产物未通过自检: {e}")))?;

        // 健康历史落库（先库后文件：库失败不覆盖合法地图）
        self.persist_module_health(run_id, &new_map).await?;

        atomic_write_json(&repo_root.join(".easyvibe/map/map.json"), &new_map).await?;
        info!("[patrol {run_id}] {repo_id} 地图已更新（watcher 将推送 map.changed）");

        let arch_score = new_map["health"]["score"].as_u64().unwrap_or(0) as u32;
        Ok(PatrolSummary { run_id: run_id.to_string(), modules: module_count(&new_map), arch_score })
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

#[cfg(test)]
mod tests {
    use super::*;
    use easyvibe_db::{Database, SqliteHealthRepository};
    use serde_json::json;

    fn sample_map() -> Value {
        json!({
            "version": "1.0",
            "meta": {"repo": "demo", "generated_at": "t", "generator": "g/test"},
            "layers": [{"id": "l1", "name": "L", "order": 0, "description": "d"}],
            "modules": [{
                "id": "m1", "name": "M", "layer": "l1", "responsibility": "r",
                "files": ["src/**"], "key_entries": [], "dependencies": [],
                "health": {"score": 60, "coupling": "high", "complexity": "high", "decay_flags": ["x"]}
            }],
            "edges": [{"id": "e1", "from": "m1", "to": "m1", "type": "call", "strength": "weak"}],
            "health": {"score": 55, "coupling": "high", "complexity": "high", "decay_flags": []}
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
        assert_eq!(summary.arch_score, 56); // 55 + 1

        // 文件已原子写回
        let written: Value =
            serde_json::from_str(&tokio::fs::read_to_string(dir.join(".easyvibe/map/map.json")).await.unwrap()).unwrap();
        assert_eq!(written["health"]["score"], 56);

        // 历史落库
        let runs = health.list_runs("demo", 10).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "succeeded");
        assert_eq!(runs[0].arch_score, Some(56));
        let history = health.list_module_history("demo", "m1", 10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].score, 60);
    }

    #[test]
    fn extract_json_tolerant() {
        let raw = "前言\n```json\n{\"a\":1}\n```\n后记";
        assert_eq!(extract_json(raw).unwrap()["a"], 1);
        let raw2 = "结果： {\"b\": 2} 完";
        assert_eq!(extract_json(raw2).unwrap()["b"], 2);
    }
}
