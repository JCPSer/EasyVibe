//! 巡检槽位：PatrolService 编排（LLM 产出地图 → 校验 → 原子写回 → 健康历史落库）
//! + JSON 提取容错工具。拆自 lib.rs（2026-10-05 防膨胀）。

use crate::client::{now_iso, ChatRequest, LlmClient};
use easyvibe_common::ApiError;
use easyvibe_db::{FinishPatrolRun, HealthRepository, ModuleHealthRow, NewPatrolRun};
use easyvibe_map::{atomic_write_json, validate_strict};
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::{info, warn};

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
                        concerns_diff: None, // Stub 路径无新旧对照（评审#B2：通道在 finish_run）
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
                        concerns_diff: None,
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
        let outcome = llm.chat(ChatRequest { system: "你是 EasyVibe 巡检 Agent，只输出 JSON 本身。", user: &user, images: &[] }).await?;
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
        concerns_diff: Option<String>,
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
                concerns_diff,
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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::StubLlmClient;
    use crate::test_fixtures::{prompt, sample_map};
    use easyvibe_db::{Database, SqliteHealthRepository};

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
    fn extract_json_tolerant() {
        let raw = "前言\n```json\n{\"a\":1}\n```\n后记";
        assert_eq!(extract_json(raw).unwrap()["a"], 1);
        let raw2 = "结果： {\"b\": 2} 完";
        assert_eq!(extract_json(raw2).unwrap()["b"], 2);
    }
}
