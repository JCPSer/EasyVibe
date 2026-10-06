//! git 域编排（自 `git.rs` 的 handler 内联逻辑原样下沉，零语义改动）。
//!
//! 领域规则（解析 / 命令 / 安全防线）在 `easyvibe-git`；本层只做
//! 「仓库查找 + 领域调用 + 响应数据装配」，HTTP 边界（入参解析 / 状态码 / Json 包装）留 `routes/repo.rs`。

use crate::state::{resolve_llm, AppState, LlmMode};
use easyvibe_common::ApiError;
use easyvibe_map::Repo;
use serde_json::Value;

/// 提交成功返回（HTTP `data` 形状：`{shortHash}`，与原内联字面量逐字段一致）。
#[derive(serde::Serialize)]
pub(crate) struct CommitResult {
    #[serde(rename = "shortHash")]
    pub(crate) short_hash: String,
}

/// 提交说明生成返回（HTTP `data` 形状：`{message, footer}`）。
#[derive(serde::Serialize)]
pub(crate) struct CommitMessageResult {
    pub(crate) message: String,
    pub(crate) footer: Option<String>,
}

fn to_value<T: serde::Serialize>(v: T) -> Result<Value, ApiError> {
    serde_json::to_value(v).map_err(|e| ApiError::Internal(format!("响应序列化失败: {e}")))
}

/// M4-4 提交把关台请求（从 `git.rs::CommitMessageRequest` 原样搬迁；字段与 serde 语义不变）。
#[derive(serde::Deserialize)]
pub(crate) struct CommitMessageRequest {
    #[serde(default)]
    pub(crate) task_id: Option<String>,
    #[serde(default)]
    pub(crate) modules: Vec<String>,
    #[serde(default)]
    pub(crate) diff_stat: String,
}

async fn find_repo(st: &AppState, id: &str) -> Result<Repo, ApiError> {
    st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))
}

pub(crate) async fn git_status(st: &AppState, id: &str) -> Result<Value, ApiError> {
    let repo = find_repo(st, id).await?;
    to_value(easyvibe_git::status(&repo.root).await?)
}

pub(crate) async fn git_log(st: &AppState, id: &str, limit: i64) -> Result<Value, ApiError> {
    let repo = find_repo(st, id).await?;
    to_value(easyvibe_git::log(&repo.root, limit).await?)
}

pub(crate) async fn git_commit_detail(st: &AppState, id: &str, hash: &str) -> Result<Value, ApiError> {
    let repo = find_repo(st, id).await?;
    to_value(easyvibe_git::show_commit(&repo.root, hash).await?)
}

pub(crate) async fn git_commit(st: &AppState, id: &str, message: &str) -> Result<CommitResult, ApiError> {
    let repo = find_repo(st, id).await?;
    Ok(CommitResult { short_hash: easyvibe_git::commit_all(&repo.root, message).await? })
}

pub(crate) async fn git_pull(st: &AppState, id: &str) -> Result<(), ApiError> {
    let repo = find_repo(st, id).await?;
    easyvibe_git::pull(&repo.root).await
}

pub(crate) async fn git_push(st: &AppState, id: &str) -> Result<(), ApiError> {
    let repo = find_repo(st, id).await?;
    easyvibe_git::push(&repo.root).await
}

/// 撤销：path == "*" → 全部撤销（tracked 恢复 + untracked 删除），否则逐文件。
pub(crate) async fn git_discard(st: &AppState, id: &str, path: &str) -> Result<(), ApiError> {
    let repo = find_repo(st, id).await?;
    if path == "*" {
        return easyvibe_git::discard_all(&repo.root).await;
    }
    easyvibe_git::discard(&repo.root, path).await
}

/// M4-4 提交把关台：从任务上下文 + 影响面 AI 生成提交说明（Conventional Commits 单行）。
/// footer（EasyVibe-Task: <id>）由后端一并返回，提交时随说明写入，历史可反查任务。
pub(crate) async fn git_commit_message(
    st: &AppState,
    id: &str,
    body: CommitMessageRequest,
) -> Result<CommitMessageResult, ApiError> {
    use easyvibe_db::TaskRepository as _;
    let _repo = find_repo(st, id).await?;

    let (task_ctx, footer) = match &body.task_id {
        Some(tid) => {
            let t = st.task_repo.get(tid).await?.ok_or_else(|| ApiError::NotFound(format!("任务 {tid} 不存在")))?;
            let summary = t.result.as_deref().and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok())
                .and_then(|v| v["result"]["summary"].as_str().map(str::to_string));
            let ctx = format!("任务 {}：{}\n需求描述：{}\n执行总结：{}", t.id, t.title, t.description, summary.unwrap_or_else(|| "（无）".into()));
            (ctx, Some(format!("EasyVibe-Task: {}", t.id)))
        }
        None => (String::new(), None),
    };

    let message = match *st.llm_mode {
        LlmMode::Stub => format!("chore({}): EasyVibe 汇总提交", if body.modules.is_empty() { "repo" } else { &body.modules[0] }),
        LlmMode::Anthropic => {
            let cfg = resolve_llm(st, id, "chat").await;
            if cfg.api_key.is_empty() {
                return Err(ApiError::BadRequest("未配置 LLM API key（设置面板或 EASYVIBE_LLM_API_KEY）".into()));
            }
            let system = "你是提交说明撰写助手。根据给定上下文输出一条符合 Conventional Commits 的中文提交说明：仅一行 subject（≤60 字），格式 type(scope): 描述，type 取 fix/feat/refactor/chore/docs 之一，scope 取主要模块名。只输出这一行，不要任何解释、引号或多余内容。";
            let user = format!(
                "改动涉及模块：{}\n任务上下文：\n{}\n变更统计（git diff --stat）：\n{}\n\n提交说明：",
                body.modules.join("、"),
                if task_ctx.is_empty() { "（无关联任务）" } else { &task_ctx },
                body.diff_stat
            );
            let llm = easyvibe_ai_agent::AnthropicClient::new(&cfg.base_url, &cfg.api_key, &cfg.model);
            let out = easyvibe_ai_agent::LlmClient::chat(&llm, easyvibe_ai_agent::ChatRequest { system, user: &user, images: &[] }).await?;
            out.text.trim().lines().next().unwrap_or_default().trim().to_string()
        }
    };
    if message.is_empty() {
        return Err(ApiError::Internal("LLM 未产出提交说明".into()));
    }
    Ok(CommitMessageResult { message, footer })
}
