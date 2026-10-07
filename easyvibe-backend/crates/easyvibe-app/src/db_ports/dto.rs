//! 本地 DTO（切断 `easyvibe_db` 行类型向 service/** 的穿透；c-arch-13 R1 自 db_ports.rs 纯搬运）。

use serde::Serialize;

// ---------------------------- 本地 DTO ----------------------------

/// 设置行（切断 `SettingRow` 穿透）。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SettingValue {
    pub(crate) scope: String,
    pub(crate) key: String,
    pub(crate) value: String,
    pub(crate) encrypted: bool,
    pub(crate) updated_at: String,
}

/// 任务行（切断 `TaskRow` 穿透；字段逐一同名搬运）。
#[derive(Debug, Clone)]
pub(crate) struct TaskInfo {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) modules: String,
    pub(crate) acceptance: String,
    pub(crate) source: String,
    pub(crate) status: String,
    pub(crate) trust: String,
    pub(crate) error: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) gate: Option<String>,
    pub(crate) result: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) conversation_id: Option<String>,
    pub(crate) origin_task_id: Option<String>,
    pub(crate) successor_task_id: Option<String>,
}

/// 建任务草稿（切断 `TaskRow` 穿透；`create` 内部转具体行类型）。
#[derive(Debug, Clone)]
pub(crate) struct TaskDraft {
    pub(crate) id: String,
    pub(crate) repo: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) modules: String,
    pub(crate) acceptance: String,
    pub(crate) source: String,
    pub(crate) context: String,
    pub(crate) status: String,
    pub(crate) trust: String,
    pub(crate) error: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) gate: Option<String>,
    pub(crate) prompt_tokens: Option<i64>,
    pub(crate) completion_tokens: Option<i64>,
    pub(crate) result: Option<String>,
    pub(crate) base_head: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) conversation_id: Option<String>,
    pub(crate) origin_task_id: Option<String>,
    pub(crate) successor_task_id: Option<String>,
}

/// 会话行（切断 `ConversationRow` 穿透）。
#[derive(Debug, Clone)]
pub(crate) struct Conversation {
    pub(crate) id: String,
    pub(crate) repo: String,
    pub(crate) summary: Option<String>,
    pub(crate) prompt_tokens: i64,
    pub(crate) completion_tokens: i64,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) title: Option<String>,
}

/// 会话消息（切断 `ConversationMessageRow` 穿透；序列化形状与原行类型**逐字等价**）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Message {
    pub(crate) id: i64,
    pub(crate) conversation_id: String,
    pub(crate) role: String,
    pub(crate) content: String,
    pub(crate) compacted: bool,
    pub(crate) tokens: i64,
    pub(crate) created_at: String,
}

/// 本地 DTO → 具体消息行（仅用于喂 `easyvibe_ai_agent::compaction` 的既有窄接口）。
pub(crate) fn to_message_rows(msgs: &[Message]) -> Vec<easyvibe_db::ConversationMessageRow> {
    msgs.iter()
        .map(|m| easyvibe_db::ConversationMessageRow {
            id: m.id,
            conversation_id: m.conversation_id.clone(),
            role: m.role.clone(),
            content: m.content.clone(),
            compacted: m.compacted,
            tokens: m.tokens,
            created_at: m.created_at.clone(),
        })
        .collect()
}
