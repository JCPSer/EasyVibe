//! git 域编排（自 `git.rs` 的 handler 内联逻辑原样下沉，零语义改动）。
//!
//! 领域规则（解析 / 命令 / 安全防线）在 `easyvibe-git`；本层只做
//! 「仓库查找 + 领域调用 + 响应数据装配」，HTTP 边界（入参解析 / 状态码 / Json 包装）留 `routes/repo.rs`。

use crate::db_ports::TaskPort as _;
use crate::state::{resolve_llm, AppState, LlmMode};
use easyvibe_common::ApiError;
use easyvibe_map::Repo;
use serde_json::Value;
use std::path::Path;

/// git 域出边端口（c-arch-16 R2）：把 easyvibe-git crate 的 9 个纯函数调用收成端口，
/// 适配器落装配格（`assembly/ports.rs`）——本 trait 是 server-api 侧的**唯一**消费面，
/// 故本文件与整个 server-api 零 easyvibe-git 路径字面量，`server-api → easyvibe-git` 出边由此消失。
///
/// **为何此处可 `dyn`（而 `db_ports/mod.rs` 记的路径不可）**：
/// 本端口 9 个方法都是**纯函数**——入参只有 `&Path` / `&str` / `bool` / `i64`，出参为
/// `serde_json::Value` / `String` / `()`，无泛型具体类型约束、无跨 crate 具体类型共享，
/// 故 trait 对象安全、可用 `Arc<dyn GitPort>`。而 `db_ports` 家族之所以走「扩展 trait + 具体类型字段」
/// 的零 dyn 路径，是因为那个组合里有 6 处接受**具体/泛型具体**类型（设置仓储的 agent 解析链、
/// 巡检服务的泛型仓储参数等），字段改 dyn 会连带破坏它们。
///
/// **为何读操作返回 `Value`**：若签名写 easyvibe-git 的 `GitStatus` / `FileDiff` / … 领域类型，
/// server-api 侧仍会出现 easyvibe-git 的路径字面量 ⇒ 边不消失。返回 `serde_json::Value` 使端口面
/// 零领域类型（与改造前经 `to_value` 包装的响应形状逐字节等价——序列化移入适配器）。
#[async_trait::async_trait]
pub trait GitPort: Send + Sync + 'static {
    async fn status(&self, root: &Path) -> Result<Value, ApiError>;
    async fn log(&self, root: &Path, limit: i64) -> Result<Value, ApiError>;
    async fn show_commit(&self, root: &Path, hash: &str) -> Result<Value, ApiError>;
    async fn diff(&self, root: &Path, path: &str, staged: bool) -> Result<Value, ApiError>;
    /// 提交成功返回短 hash（HTTP `data.shortHash` 的原始来源，形态不变）。
    async fn commit_all(&self, root: &Path, message: &str) -> Result<String, ApiError>;
    async fn pull(&self, root: &Path) -> Result<(), ApiError>;
    async fn push(&self, root: &Path) -> Result<(), ApiError>;
    async fn discard(&self, root: &Path, path: &str) -> Result<(), ApiError>;
    async fn discard_all(&self, root: &Path) -> Result<(), ApiError>;
}

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

/// git 写操作（提交/pull 改本地历史）后重估保鲜并推送 freshness.changed——
/// 否则头部"落后提示"要等 30 分钟定时器才刷新（2026-10-06 实弹：软件内提交后
/// 主页无提示，漂移洞察自己刷新所以看得见——两个视图不同步）。
/// 与归纳/巡检终态的即时重估同一口径（service/map.rs）。
async fn refresh_freshness(st: &AppState, repo: &Repo) {
    if let Ok(snap) = st.map_service.load_map(repo).await {
        let f = easyvibe_map::freshness::assess(&repo.root, &snap.json);
        easyvibe_event_bus::publish(&st.event_bus, easyvibe_event_bus::BusEvent::Freshness {
            repo: repo.id.clone(),
            status: f.status.as_str().to_string(),
            latest_commit_at: f.latest_commit_at,
            commits_since_map: f.commits_since_map,
        });
    }
}

pub(crate) async fn git_status(st: &AppState, id: &str) -> Result<Value, ApiError> {
    let repo = find_repo(st, id).await?;
    st.git.status(&repo.root).await
}

pub(crate) async fn git_log(st: &AppState, id: &str, limit: i64) -> Result<Value, ApiError> {
    let repo = find_repo(st, id).await?;
    st.git.log(&repo.root, limit).await
}

pub(crate) async fn git_commit_detail(st: &AppState, id: &str, hash: &str) -> Result<Value, ApiError> {
    let repo = find_repo(st, id).await?;
    st.git.show_commit(&repo.root, hash).await
}

/// 单文件差异（diff 抽屉）：staged=false 工作区 vs 暂存区，staged=true 暂存区 vs HEAD。
/// 路径防注入在 easyvibe-git crate 的 diff 内（validate_rel_path + 禁 `-` 开头），本层不重复。
pub(crate) async fn git_diff(st: &AppState, id: &str, path: &str, staged: bool) -> Result<Value, ApiError> {
    let repo = find_repo(st, id).await?;
    st.git.diff(&repo.root, path, staged).await
}

pub(crate) async fn git_commit(st: &AppState, id: &str, message: &str) -> Result<CommitResult, ApiError> {
    let repo = find_repo(st, id).await?;
    let short_hash = st.git.commit_all(&repo.root, message).await?;
    refresh_freshness(st, &repo).await;
    Ok(CommitResult { short_hash })
}

pub(crate) async fn git_pull(st: &AppState, id: &str) -> Result<(), ApiError> {
    let repo = find_repo(st, id).await?;
    st.git.pull(&repo.root).await?;
    refresh_freshness(st, &repo).await;
    Ok(())
}

pub(crate) async fn git_push(st: &AppState, id: &str) -> Result<(), ApiError> {
    let repo = find_repo(st, id).await?;
    st.git.push(&repo.root).await
}

/// 撤销：path == "*" → 全部撤销（tracked 恢复 + untracked 删除），否则逐文件。
pub(crate) async fn git_discard(st: &AppState, id: &str, path: &str) -> Result<(), ApiError> {
    let repo = find_repo(st, id).await?;
    if path == "*" {
        return st.git.discard_all(&repo.root).await;
    }
    st.git.discard(&repo.root, path).await
}

/// M4-4 提交把关台：从任务上下文 + 影响面 AI 生成提交说明（Conventional Commits 单行）。
/// footer（EasyVibe-Task: <id>）由后端一并返回，提交时随说明写入，历史可反查任务。
pub(crate) async fn git_commit_message(
    st: &AppState,
    id: &str,
    body: CommitMessageRequest,
) -> Result<CommitMessageResult, ApiError> {
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
