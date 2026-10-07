//! 组合根：task-engine 持久化端口 ↔ 具体仓储的适配器（c-arch-7 R3）。
//!
//! 本文件是**唯一**允许同时看见 `task_exec::ports` 与具体仓储（`easyvibe_db`）的地方；
//! 适配器落组合根保证依赖方向仍为 server-api → task-engine（端口在依赖方、实现在编排层，
//! 与 `impl QueueHost for AppState` 同构）。task_exec 生产文件因此零直连、零反向引用。

use crate::task_exec::ports::{AgentSlotResolver, ApprovalRecord, ApprovalStore, SessionAttribution, TaskRecord, TaskStore};
use easyvibe_common::ApiError;
use std::sync::Arc;

/// 持久层任务行 → 切片内本地 DTO（字段逐一同名搬运）。
fn into_record(r: easyvibe_db::TaskRow) -> TaskRecord {
    TaskRecord {
        id: r.id,
        repo: r.repo,
        title: r.title,
        description: r.description,
        modules: r.modules,
        acceptance: r.acceptance,
        source: r.source,
        context: r.context,
        status: r.status,
        trust: r.trust,
        error: r.error,
        session_id: r.session_id,
        gate: r.gate,
        prompt_tokens: r.prompt_tokens,
        completion_tokens: r.completion_tokens,
        result: r.result,
        base_head: r.base_head,
        created_at: r.created_at,
        updated_at: r.updated_at,
        conversation_id: r.conversation_id,
        origin_task_id: r.origin_task_id,
        successor_task_id: r.successor_task_id,
    }
}

/// 任务仓储适配（全方法透传；`TaskRow → TaskRecord` 仅一次字段搬运）。
#[async_trait::async_trait]
impl TaskStore for easyvibe_db::SqliteTaskRepository {
    async fn get(&self, id: &str) -> Result<Option<TaskRecord>, ApiError> {
        Ok(easyvibe_db::TaskRepository::get(self, id).await?.map(into_record))
    }

    async fn list(&self, repo: &str, limit: i64) -> Result<Vec<TaskRecord>, ApiError> {
        Ok(easyvibe_db::TaskRepository::list(self, repo, limit).await?.into_iter().map(into_record).collect())
    }

    async fn update_status(&self, id: &str, status: &str, error: Option<&str>) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::update_status(self, id, status, error).await
    }

    async fn set_gate(&self, id: &str, gate: Option<&str>) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::set_gate(self, id, gate).await
    }

    async fn try_advance_gate(
        &self,
        id: &str,
        expected_gate: Option<&str>,
        new_gate: Option<&str>,
        new_status: Option<&str>,
    ) -> Result<u64, ApiError> {
        easyvibe_db::TaskRepository::try_advance_gate(self, id, expected_gate, new_gate, new_status).await
    }

    async fn set_context(&self, id: &str, context: &str) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::set_context(self, id, context).await
    }

    async fn reset_for_retry(&self, id: &str) -> Result<u64, ApiError> {
        easyvibe_db::TaskRepository::reset_for_retry(self, id).await
    }

    async fn reset_for_remediate(&self, id: &str) -> Result<u64, ApiError> {
        easyvibe_db::TaskRepository::reset_for_remediate(self, id).await
    }

    async fn try_rewind(&self, id: &str, target_gate: &str) -> Result<u64, ApiError> {
        easyvibe_db::TaskRepository::try_rewind(self, id, target_gate).await
    }

    async fn set_session(&self, id: &str, session_id: &str) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::set_session(self, id, session_id).await
    }

    async fn set_base_head(&self, id: &str, head: &str) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::set_base_head(self, id, head).await
    }

    async fn set_result(&self, id: &str, result_json: &str) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::set_result(self, id, result_json).await
    }
}

/// 审批仓储适配：id 与 decided_at 在此补齐（原状态机内的构造职责上提组合根）。
#[async_trait::async_trait]
impl ApprovalStore for easyvibe_db::SqliteApprovalRepository {
    async fn record(&self, approval: &ApprovalRecord) -> Result<(), ApiError> {
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let row = easyvibe_db::ApprovalRow {
            id: format!("ap-{}-{}-{}", approval.task_id, approval.gate, now_ms),
            task_id: approval.task_id.clone(),
            gate: approval.gate.clone(),
            decision: approval.decision.clone(),
            note: approval.note.clone(),
            decided_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                .to_string(),
        };
        easyvibe_db::ApprovalRepository::record(self, &row).await
    }

    async fn list_by_task(&self, task_id: &str) -> Result<Vec<ApprovalRecord>, ApiError> {
        Ok(easyvibe_db::ApprovalRepository::list_by_task(self, task_id)
            .await?
            .into_iter()
            .map(|a| ApprovalRecord {
                task_id: a.task_id,
                gate: a.gate,
                decision: a.decision,
                note: a.note,
            })
            .collect())
    }
}

/// 会话归属适配（tasks.modules 首个模块 → 会话行归因）。
#[async_trait::async_trait]
impl SessionAttribution for easyvibe_db::AgentSessionRepo {
    async fn set_module_id(&self, session_id: &str, module_id: &str) -> Result<(), ApiError> {
        easyvibe_db::AgentSessionRepo::set_module_id(self, session_id, module_id).await
    }
}

/// 槽位 agent 解析适配：settings 优先解析链（agent_conf::resolve_agent）落组合根，
/// 切片只看见「按槽位解析」的端口语义。
pub(crate) struct AgentSlotAdapter {
    settings: Arc<easyvibe_db::SqliteSettingsRepository>,
    command: Arc<String>,
    args: Arc<Vec<String>>,
}

impl AgentSlotAdapter {
    pub(crate) fn new(
        settings: Arc<easyvibe_db::SqliteSettingsRepository>,
        command: Arc<String>,
        args: Arc<Vec<String>>,
    ) -> Self {
        Self { settings, command, args }
    }
}

#[async_trait::async_trait]
impl AgentSlotResolver for AgentSlotAdapter {
    async fn resolve(&self, slot: Option<&str>) -> easyvibe_ai_agent::agent_conf::ResolvedAgent {
        easyvibe_ai_agent::agent_conf::resolve_agent(&self.settings, slot, &self.command, &self.args).await
    }
}

// ============================================================================
// c-arch-10 R4：service/** 编排层消费的持久化端口（**扩展 trait**）与本地 DTO
// ----------------------------------------------------------------------------
// 为什么是「扩展 trait + 具体类型字段」而非 `Arc<dyn …>`：`agent_conf::resolve_agent(
// &SqliteSettingsRepository, …)` 与 `PatrolService<R: HealthRepository>` 都接受**具体/泛型具体**类型，
// AppState 字段改 dyn 会连带破坏这 6 处组合 ⇒ 唯一零 `dyn` 路径是「impl XPort for 具体仓储」。
// service/** 只见 `crate::db_ports::*Port` 与本地 DTO，不再直连 `easyvibe_db`。
// 组合根（state.rs / db_ports.rs / assembly/** / 测试面）仍是 `easyvibe_db` 的唯一合法落点。
// ============================================================================

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

// ---------------------------- 设置端口 ----------------------------

#[async_trait::async_trait]
pub(crate) trait SettingsPort {
    async fn get(&self, scope: &str, key: &str) -> Result<Option<SettingValue>, ApiError>;
    async fn list(&self, scope: &str) -> Result<Vec<SettingValue>, ApiError>;
    async fn set(&self, scope: &str, key: &str, value: String, encrypted: bool, updated_at: String) -> Result<(), ApiError>;
    async fn delete(&self, scope: &str, key: &str) -> Result<(), ApiError>;
}

#[async_trait::async_trait]
impl SettingsPort for easyvibe_db::SqliteSettingsRepository {
    async fn get(&self, scope: &str, key: &str) -> Result<Option<SettingValue>, ApiError> {
        Ok(easyvibe_db::SettingsRepository::get(self, scope, key)
            .await?
            .map(|r| SettingValue { scope: r.scope, key: r.key, value: r.value, encrypted: r.encrypted, updated_at: r.updated_at }))
    }
    async fn list(&self, scope: &str) -> Result<Vec<SettingValue>, ApiError> {
        Ok(easyvibe_db::SettingsRepository::list(self, scope)
            .await?
            .into_iter()
            .map(|r| SettingValue { scope: r.scope, key: r.key, value: r.value, encrypted: r.encrypted, updated_at: r.updated_at })
            .collect())
    }
    async fn set(&self, scope: &str, key: &str, value: String, encrypted: bool, updated_at: String) -> Result<(), ApiError> {
        let row = easyvibe_db::SettingRow { scope: scope.to_string(), key: key.to_string(), value, encrypted, updated_at };
        easyvibe_db::SettingsRepository::set(self, &row).await
    }
    async fn delete(&self, scope: &str, key: &str) -> Result<(), ApiError> {
        easyvibe_db::SettingsRepository::delete(self, scope, key).await
    }
}

// ---------------------------- 任务端口 ----------------------------

#[async_trait::async_trait]
pub(crate) trait TaskPort {
    async fn get(&self, id: &str) -> Result<Option<TaskInfo>, ApiError>;
    async fn list(&self, repo: &str, limit: i64) -> Result<Vec<TaskInfo>, ApiError>;
    async fn list_by_conversation(&self, conversation_id: &str) -> Result<Vec<TaskInfo>, ApiError>;
    async fn create(&self, t: &TaskDraft) -> Result<(), ApiError>;
    async fn set_successor(&self, id: &str, successor: &str) -> Result<(), ApiError>;
    async fn delete(&self, id: &str) -> Result<(), ApiError>;
}

fn into_task_info(r: easyvibe_db::TaskRow) -> TaskInfo {
    TaskInfo {
        id: r.id, title: r.title, description: r.description, modules: r.modules,
        acceptance: r.acceptance, source: r.source, status: r.status, trust: r.trust, error: r.error,
        session_id: r.session_id, gate: r.gate, result: r.result, created_at: r.created_at,
        updated_at: r.updated_at, conversation_id: r.conversation_id, origin_task_id: r.origin_task_id,
        successor_task_id: r.successor_task_id,
    }
}

#[async_trait::async_trait]
impl TaskPort for easyvibe_db::SqliteTaskRepository {
    async fn get(&self, id: &str) -> Result<Option<TaskInfo>, ApiError> {
        Ok(easyvibe_db::TaskRepository::get(self, id).await?.map(into_task_info))
    }
    async fn list(&self, repo: &str, limit: i64) -> Result<Vec<TaskInfo>, ApiError> {
        Ok(easyvibe_db::TaskRepository::list(self, repo, limit).await?.into_iter().map(into_task_info).collect())
    }
    async fn list_by_conversation(&self, conversation_id: &str) -> Result<Vec<TaskInfo>, ApiError> {
        Ok(easyvibe_db::TaskRepository::list_by_conversation(self, conversation_id).await?.into_iter().map(into_task_info).collect())
    }
    async fn create(&self, t: &TaskDraft) -> Result<(), ApiError> {
        let row = easyvibe_db::TaskRow {
            id: t.id.clone(), repo: t.repo.clone(), title: t.title.clone(), description: t.description.clone(),
            modules: t.modules.clone(), acceptance: t.acceptance.clone(), source: t.source.clone(),
            context: t.context.clone(), status: t.status.clone(), trust: t.trust.clone(), error: t.error.clone(),
            session_id: t.session_id.clone(), gate: t.gate.clone(), prompt_tokens: t.prompt_tokens,
            completion_tokens: t.completion_tokens, result: t.result.clone(), base_head: t.base_head.clone(),
            created_at: t.created_at.clone(), updated_at: t.updated_at.clone(),
            conversation_id: t.conversation_id.clone(), origin_task_id: t.origin_task_id.clone(),
            successor_task_id: t.successor_task_id.clone(),
        };
        easyvibe_db::TaskRepository::create(self, &row).await
    }
    async fn set_successor(&self, id: &str, successor: &str) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::set_successor(self, id, successor).await
    }
    async fn delete(&self, id: &str) -> Result<(), ApiError> {
        easyvibe_db::TaskRepository::delete(self, id).await
    }
}

// ---------------------------- 审批端口 ----------------------------

/// 审批端口（返回具体行类型不构成类型穿透——service 侧不写名，且 JSON 形状零改动）。
#[async_trait::async_trait]
pub(crate) trait ApprovalPort {
    async fn list_by_task(&self, task_id: &str) -> Result<Vec<easyvibe_db::ApprovalRow>, ApiError>;
}

#[async_trait::async_trait]
impl ApprovalPort for easyvibe_db::SqliteApprovalRepository {
    async fn list_by_task(&self, task_id: &str) -> Result<Vec<easyvibe_db::ApprovalRow>, ApiError> {
        easyvibe_db::ApprovalRepository::list_by_task(self, task_id).await
    }
}

// ---------------------------- 会话端口 ----------------------------

#[async_trait::async_trait]
pub(crate) trait ConversationPort {
    async fn get_or_create(&self, repo: &str) -> Result<Conversation, ApiError>;
    async fn list_by_repo(&self, repo: &str) -> Result<Vec<Conversation>, ApiError>;
    async fn create(&self, id: &str, repo: &str, title: Option<&str>) -> Result<Conversation, ApiError>;
    async fn rename(&self, conversation_id: &str, title: &str) -> Result<(), ApiError>;
    async fn delete(&self, conversation_id: &str) -> Result<(), ApiError>;
    async fn reset(&self, conversation_id: &str) -> Result<(), ApiError>;
    async fn list_messages(&self, conversation_id: &str, before_id: Option<i64>, limit: i64) -> Result<Vec<Message>, ApiError>;
    async fn count_messages(&self, conversation_id: &str) -> Result<i64, ApiError>;
    async fn list_uncompacted(&self, conversation_id: &str) -> Result<Vec<Message>, ApiError>;
    async fn append_message(&self, conversation_id: &str, role: &str, content: &str, tokens: i64) -> Result<i64, ApiError>;
    async fn add_tokens(&self, conversation_id: &str, prompt_tokens: i64, completion_tokens: i64) -> Result<(), ApiError>;
    async fn apply_compaction(&self, conversation_id: &str, before_id: i64, summary: &str, prompt_tokens: i64, completion_tokens: i64) -> Result<(), ApiError>;
}

fn into_conversation(r: easyvibe_db::ConversationRow) -> Conversation {
    Conversation {
        id: r.id, repo: r.repo, summary: r.summary, prompt_tokens: r.prompt_tokens,
        completion_tokens: r.completion_tokens, created_at: r.created_at, updated_at: r.updated_at, title: r.title,
    }
}

fn into_message(r: easyvibe_db::ConversationMessageRow) -> Message {
    Message {
        id: r.id, conversation_id: r.conversation_id, role: r.role, content: r.content,
        compacted: r.compacted, tokens: r.tokens, created_at: r.created_at,
    }
}

#[async_trait::async_trait]
impl ConversationPort for easyvibe_db::SqliteConversationRepository {
    async fn get_or_create(&self, repo: &str) -> Result<Conversation, ApiError> {
        Ok(into_conversation(easyvibe_db::ConversationRepository::get_or_create(self, repo).await?))
    }
    async fn list_by_repo(&self, repo: &str) -> Result<Vec<Conversation>, ApiError> {
        Ok(easyvibe_db::ConversationRepository::list_by_repo(self, repo).await?.into_iter().map(into_conversation).collect())
    }
    async fn create(&self, id: &str, repo: &str, title: Option<&str>) -> Result<Conversation, ApiError> {
        Ok(into_conversation(easyvibe_db::ConversationRepository::create(self, id, repo, title).await?))
    }
    async fn rename(&self, conversation_id: &str, title: &str) -> Result<(), ApiError> {
        easyvibe_db::ConversationRepository::rename(self, conversation_id, title).await
    }
    async fn delete(&self, conversation_id: &str) -> Result<(), ApiError> {
        easyvibe_db::ConversationRepository::delete(self, conversation_id).await
    }
    async fn reset(&self, conversation_id: &str) -> Result<(), ApiError> {
        easyvibe_db::ConversationRepository::reset(self, conversation_id).await
    }
    async fn list_messages(&self, conversation_id: &str, before_id: Option<i64>, limit: i64) -> Result<Vec<Message>, ApiError> {
        Ok(easyvibe_db::ConversationRepository::list_messages(self, conversation_id, before_id, limit).await?.into_iter().map(into_message).collect())
    }
    async fn count_messages(&self, conversation_id: &str) -> Result<i64, ApiError> {
        easyvibe_db::ConversationRepository::count_messages(self, conversation_id).await
    }
    async fn list_uncompacted(&self, conversation_id: &str) -> Result<Vec<Message>, ApiError> {
        Ok(easyvibe_db::ConversationRepository::list_uncompacted(self, conversation_id).await?.into_iter().map(into_message).collect())
    }
    async fn append_message(&self, conversation_id: &str, role: &str, content: &str, tokens: i64) -> Result<i64, ApiError> {
        easyvibe_db::ConversationRepository::append_message(self, conversation_id, role, content, tokens).await
    }
    async fn add_tokens(&self, conversation_id: &str, prompt_tokens: i64, completion_tokens: i64) -> Result<(), ApiError> {
        easyvibe_db::ConversationRepository::add_tokens(self, conversation_id, prompt_tokens, completion_tokens).await
    }
    async fn apply_compaction(&self, conversation_id: &str, before_id: i64, summary: &str, prompt_tokens: i64, completion_tokens: i64) -> Result<(), ApiError> {
        easyvibe_db::ConversationRepository::apply_compaction(self, conversation_id, before_id, summary, prompt_tokens, completion_tokens).await
    }
}

// ---------------------------- 巡检健康端口 ----------------------------

#[async_trait::async_trait]
pub(crate) trait HealthPort {
    async fn list_runs(&self, repo: &str, limit: i64) -> Result<Vec<easyvibe_db::PatrolRunRow>, ApiError>;
    async fn list_run_averages(&self, repo: &str, limit: i64) -> Result<Vec<easyvibe_db::RunModuleAvg>, ApiError>;
    async fn list_latest_run_modules(&self, repo: &str) -> Result<Vec<easyvibe_db::ModuleHealthRow>, ApiError>;
    async fn list_module_history(&self, repo: &str, module_id: &str, limit: i64) -> Result<Vec<easyvibe_db::ModuleHealthRow>, ApiError>;
    async fn prune_runs(&self, repo: &str, keep: i64) -> Result<u64, ApiError>;
}

#[async_trait::async_trait]
impl HealthPort for easyvibe_db::SqliteHealthRepository {
    async fn list_runs(&self, repo: &str, limit: i64) -> Result<Vec<easyvibe_db::PatrolRunRow>, ApiError> {
        easyvibe_db::HealthRepository::list_runs(self, repo, limit).await
    }
    async fn list_run_averages(&self, repo: &str, limit: i64) -> Result<Vec<easyvibe_db::RunModuleAvg>, ApiError> {
        easyvibe_db::HealthRepository::list_run_averages(self, repo, limit).await
    }
    async fn list_latest_run_modules(&self, repo: &str) -> Result<Vec<easyvibe_db::ModuleHealthRow>, ApiError> {
        easyvibe_db::HealthRepository::list_latest_run_modules(self, repo).await
    }
    async fn list_module_history(&self, repo: &str, module_id: &str, limit: i64) -> Result<Vec<easyvibe_db::ModuleHealthRow>, ApiError> {
        easyvibe_db::HealthRepository::list_module_history(self, repo, module_id, limit).await
    }
    async fn prune_runs(&self, repo: &str, keep: i64) -> Result<u64, ApiError> {
        easyvibe_db::HealthRepository::prune_runs(self, repo, keep).await
    }
}

// ---------------------------- 事件端口 ----------------------------

#[async_trait::async_trait]
pub(crate) trait EventPort {
    async fn record(&self, repo: &str, name: &str, payload: &str) -> Result<(), ApiError>;
    async fn summary(&self, repo: &str) -> Result<Vec<easyvibe_db::EventCountRow>, ApiError>;
}

#[async_trait::async_trait]
impl EventPort for easyvibe_db::SqliteEventRepository {
    async fn record(&self, repo: &str, name: &str, payload: &str) -> Result<(), ApiError> {
        easyvibe_db::EventRepository::record(self, repo, name, payload).await
    }
    async fn summary(&self, repo: &str) -> Result<Vec<easyvibe_db::EventCountRow>, ApiError> {
        easyvibe_db::EventRepository::summary(self, repo).await
    }
}

// ---------------------------- 仓库维护端口 ----------------------------

/// 注销仓库的跨表清除（原 `easyvibe_db::wipe_repo(&st.pool, id)`）。
#[async_trait::async_trait]
pub(crate) trait RepoMaintenancePort {
    async fn wipe(&self, repo: &str) -> Result<u64, ApiError>;
}

#[async_trait::async_trait]
impl RepoMaintenancePort for easyvibe_db::sqlx::SqlitePool {
    async fn wipe(&self, repo: &str) -> Result<u64, ApiError> {
        easyvibe_db::wipe_repo(self, repo).await
    }
}
