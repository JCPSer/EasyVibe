//! 智能优化建议槽位：SuggestClient 契约、Stub 确定性派生、LLM JSON 数组输出解析。
//! 拆自 lib.rs（2026-10-05 防膨胀）。

use crate::client::{ChatRequest, LlmClient};
use easyvibe_common::ApiError;
use serde_json::Value;

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
        let raw = self.llm.chat(ChatRequest { system, user: &user, images: &[] }).await?;
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
mod tests {
    use super::*;
    use crate::test_fixtures::sample_map;

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
}
