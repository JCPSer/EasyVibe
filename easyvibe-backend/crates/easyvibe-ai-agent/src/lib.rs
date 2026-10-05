//! ai-agent 领域：Supervisor 直调 LLM 的槽位（M2-4 先落地巡检槽位）。
//! 与 easyvibe-session 的分工：session 管"外部 CLI agent"（写路径归纳），
//! ai-agent 管"后端自己调 LLM API"（巡检/审查等槽位，PRD 5.2 的多 LLM 服务配置）。
//!
//! 门面（2026-10-05 防膨胀拆分）：本文件只留 mod 声明 + re-export + 共享测试夹具；
//! 具体槽位归位到 client / patrol / qa / suggest，规则与 agent_conf / compaction 不动。
//! 对外 `pub` 面（re-export）逐字保持——下游 server-api / task-engine 零改动。

pub mod compaction;

/// 执行 agent 配置体系（预设表 + settings 优先解析链 + 并行探测）。
/// 原位于 easyvibe-app，自 2026-10-05 解环迁移至此：server-api 与 task-engine
/// 各自单向依赖本模块，消除 task-engine → server-api 的反向引用。
pub mod agent_conf;

mod client;
mod patrol;
mod qa;
mod suggest;

// re-export：保持 lib.rs 拆分前的 crate 根 pub 面逐字不变
pub use client::{estimate_tokens, AnthropicClient, ChatOutcome, ChatRequest, LlmClient, StubLlmClient};
pub use patrol::{extract_json, PatrolService, PatrolSummary};
pub use qa::{
    parse_clarify, stub_answer, Clarify, ClarifyOption, LlmQaClient, QaAnswer, QaClient,
    StubQaClient,
};
pub use suggest::{
    extract_json_array, stub_suggest, LlmSuggestClient, SuggestClient, Suggestion, StubSuggestClient,
};

/// 共享测试夹具（拆分前位于单一 `mod tests`；现由各子模块测试按需引用）。
#[cfg(test)]
pub(crate) mod test_fixtures {
    use serde_json::{json, Value};

    pub(crate) fn sample_map() -> Value {
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

    pub(crate) fn prompt() -> String {
        "巡检 <REPO_ROOT> <SCHEMA_PATH>\n<CURRENT_MAP>\n```".to_string()
    }
}
