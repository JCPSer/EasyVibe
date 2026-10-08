//! 应用共享状态、错误与跨域助手（server-api 装配层地基）。

use axum::{
    response::{IntoResponse, Response},
    Json,
};
use easyvibe_common::{ApiError, ErrorResponse};
use easyvibe_event_bus::queue::QueueState;
use easyvibe_event_bus::BusEvent;
use easyvibe_map::MapService;
use easyvibe_session::SessionManager;
use easyvibe_ai_agent::agent_conf;
use std::sync::Arc;
use tokio::sync::broadcast;
use crate::service::git::GitPort;
use crate::service::repo::RepoPipelinePort;
use crate::task_exec;
use easyvibe_db::SettingsRepository as _;

#[derive(Clone)]
pub struct AppState {
    pub map_service: Arc<MapService>,
    pub session_manager: Arc<SessionManager>,
    /// v2.2 提示词模板（含 <REPO_ROOT> 占位）
    pub prompt_template: Arc<String>,
    /// agent CLI 命令与参数（如 claude + ["-p"]）
    pub agent_command: Arc<String>,
    pub agent_args: Arc<Vec<String>>,
    // M2-4：巡检槽位 + 域 2 健康历史
    pub patrol_service: Arc<easyvibe_ai_agent::PatrolService<easyvibe_db::SqliteHealthRepository>>,
    pub health_repo: Arc<easyvibe_db::SqliteHealthRepository>,
    /// M1/U1：agent 会话持久层（运行页历史回放 + 用量页统计的地基）
    pub agent_session_repo: Arc<easyvibe_db::AgentSessionRepo>,
    /// M2：会话输出行持久层（历史回放/断线补拉）
    pub session_output_repo: Arc<easyvibe_db::SessionOutputRepo>,
    pub settings_repo: Arc<easyvibe_db::SqliteSettingsRepository>,
    pub cipher: Arc<easyvibe_common::SecretCipher>,
    pub task_repo: Arc<easyvibe_db::SqliteTaskRepository>,
    pub approval_repo: Arc<easyvibe_db::SqliteApprovalRepository>,
    pub conversation_repo: Arc<easyvibe_db::SqliteConversationRepository>,
    /// R3 D1：使用证据埋点（门控的秤——L3 三道门、Harness 验证指标的数据源）
    pub event_repo: Arc<easyvibe_db::SqliteEventRepository>,
    /// 会话写串行化（审查 🟡1）：append+compact 是 read-modify-write，SQLite 语句原子
    /// 不保证这段复合操作；发送中点"压缩上下文"是 UI 允许的真实并发
    pub chat_lock: Arc<tokio::sync::Mutex<()>>,
    pub executor: Arc<task_exec::TaskExecutor>,
    /// S1-3：harness 单一事实源（插槽内核；恢复默认后热换，chat 与 executor 共用）
    pub harness: Arc<tokio::sync::RwLock<task_exec::Harness>>,
    pub llm_mode: Arc<LlmMode>,
    pub patrol_prompt: Arc<String>,
    /// 子图分析提示词模板（含 <REPO_ROOT>/<MODULE_ID>/<MODULE_JSON> 占位）
    pub submap_prompt: Arc<String>,
    /// 增量归纳 prompt（B 方案；含 <REPO_ROOT>/<CURRENT_MAP>/<COMMIT_LOG>/<DIFF_NUMSTAT>/<DIFF_CONTENT>/<SCHEMA_PATH> 占位）
    pub incremental_prompt: Arc<String>,
    pub schema_path: Arc<String>,
    /// 后端 → 前端事件总线（broadcast；WS handler 订阅）
    pub event_bus: broadcast::Sender<BusEvent>,
    /// 库连接池直持（重审 P1：注销仓库 wipe_repo 跨表清除需要；repos 各自私有池不便借用）
    pub pool: easyvibe_db::sqlx::SqlitePool,
    /// M1 配置体系：agent 探测结果（内存态，启动探测 + POST /api/agent/detect 重探）
    pub agent_detected: Arc<tokio::sync::RwLock<Vec<agent_conf::DetectedAgent>>>,
    /// M1：最近一条测试连接结果（GET /api/agent/status 的 protocolOk 数据源）
    pub agent_test: Arc<tokio::sync::RwLock<Option<agent_conf::AgentTestResult>>>,
    /// M1：测试连接串行化（连点/并发测试互相踩结果）
    pub agent_test_lock: Arc<tokio::sync::Mutex<()>>,
    /// 运行会话排队（需求 v1 §4.1）：key=repo_id 的单槽队列；内存态，重启即失
    /// （S4：重启后首帧 GET 自然清态，无需额外机制）。队列状态机在 easyvibe-event-bus，
    /// 执行入口经 `impl QueueHost for AppState` 回调注入（解环：不再 crate:: 反向引用）。
    pub session_queue: Arc<QueueState>,
    /// c-arch-16 R2：git 域出边端口（适配器 = 装配格 `assembly/ports.rs::GitAdapter`）。
    /// 边界层只见本端口，零 easyvibe-git 字面量 ⇒ `server-api → easyvibe-git` 出边消失。
    pub git: Arc<dyn GitPort>,
    /// c-arch-16 R3：仓库 watcher 管线出边端口（适配器 = 装配格 `assembly/ports.rs::PipelineAdapter`）。
    /// 启动期与运行期**共用同一 `Arc`**（ΔS4）；见 `service::repo::RepoPipelinePort` 注释。
    pub pipeline_port: Arc<dyn RepoPipelinePort>,
}

/// LLM 客户端来源：stub（零成本验证）或 anthropic（真实 API）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmMode {
    Stub,
    Anthropic,
}

/// ApiError 的新类型包装（绕过孤儿规则；common 层不依赖 axum）
pub(crate) struct AppError(pub(crate) ApiError);

impl From<ApiError> for AppError {
    fn from(e: ApiError) -> Self {
        AppError(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let err = self.0;
        let (status, code) = match &err {
            ApiError::BadRequest(_) => (axum::http::StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            ApiError::NotFound(_) => (axum::http::StatusCode::NOT_FOUND, "NOT_FOUND"),
            ApiError::Conflict(_) => (axum::http::StatusCode::CONFLICT, "CONFLICT"),
            ApiError::MapInvalid(_) => (axum::http::StatusCode::UNPROCESSABLE_ENTITY, "MAP_INVALID"),
            ApiError::Internal(_) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };
        let body = Json(ErrorResponse { success: false, error: err.to_string(), code: code.into() });
        (status, body).into_response()
    }
}

pub(crate) fn data_dir() -> std::path::PathBuf {
    std::env::var("EASYVIBE_DATA_DIR").map(Into::into).unwrap_or_else(|_| {
        // Windows 无 HOME（GUI 启动下 USERPROFILE 也偶有缺失）——逐级兜底，
        // 都缺时落当前目录（独立 exe 场景 = exe 旁，至少可写不闪退）
        let home = std::env::var("HOME").ok().filter(|h| !h.is_empty())
            .or_else(|| std::env::var("USERPROFILE").ok().filter(|h| !h.is_empty()))
            .unwrap_or_else(|| ".".into());
        std::path::PathBuf::from(format!("{home}/.easyvibe"))
    })
}

pub(crate) fn desktop_repos_file() -> std::path::PathBuf {
    data_dir().join("desktop-repos")
}

/// 读 desktop-repos 持久化文件（每行一个仓库根路径；# 开头为注释）
pub(crate) fn read_desktop_repos() -> Vec<std::path::PathBuf> {
    std::fs::read_to_string(desktop_repos_file())
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(Into::into)
        .collect()
}

/// 全量重写 desktop-repos（注销后保持一致）
pub(crate) fn write_desktop_repos(roots: &[std::path::PathBuf]) {
    let content = roots.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n") + "\n";
    if let Err(e) = std::fs::write(desktop_repos_file(), content) {
        tracing::warn!("desktop-repos 持久化失败: {e}");
    }
}

/// M1/U1：会话 kind 从展示 label 推导（app 层 note_label 的文本是已知的稳定词表）
pub(crate) fn session_kind_from_label(label: &str) -> &'static str {
    if label.contains("任务执行") {
        if label.contains("初审") {
            "subagent-review"
        } else if label.contains("审查") {
            "subagent-audit"
        } else {
            "task"
        }
    } else if label.contains("巡检") {
        "patrol"
    } else if label.contains("分析") {
        "submap"
    } else if label.contains("归纳") {
        "induce"
    } else {
        "unknown"
    }
}

/** U2/L1：用量聚合（今天/7天/30天/全部 → days 参数；since 由 started_at >= 计算）。
 *  一次端点拉全五个聚合（总量/按日/按类型/按模型/按模块）——省得前端串行打五个请求。
 *  cost/tokens 的 SUM 天然忽略 NULL：区间无数据时前端拿到 None，诚实显示「—」而非 0。 */

/// CLI agent 命令解析为绝对路径。GUI 启动（Finder/Dock/Tauri sidecar）的进程 PATH 极薄，
/// 裸命令名 spawn 直接 ENOENT——桌面壳实弹三任务全灭于此。三级查找：
/// ① 自身含路径且存在 → 原样；② 当前进程 PATH；③ 登录 shell PATH（用户交互环境才是真相）；
/// ④ 常见安装位兜底（含实测的 Kimi npm-global 位）。
/// 命令绝对路径解析——实现已迁至 agent_conf（M1 配置体系，复用方包括探测层），此处保留薄封装
pub(crate) fn resolve_agent_command(cmd: &str) -> String {
    agent_conf::resolve_agent_command(cmd)
}

/// CLI agent 可执行预检：spawn 路径的鉴权由 claude 自身配置（~/.claude/settings.json 的 env
/// 或进程环境变量）负责，与本进程的 EASYVIBE_LLM_API_KEY 无关——旧守卫把"DB 已配 key"
/// 的合法场景误判为未配置（chat 直调走 DB，patrol/reinduce 走 CLI spawn，两条链路配置源不同）。
/// 方案 R7：跨 patrol/reinduce/submap 复用，落 state（就近取 agent_command，减少横向依赖）。
pub(crate) fn ensure_agent_available(st: &AppState) -> Result<(), ApiError> {
    let cmd = &*st.agent_command;
    let name = cmd.rsplit('/').next().unwrap_or(cmd);
    let on_path = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| d.join(name).is_file()))
        .unwrap_or(false);
    if !on_path && !std::path::Path::new(cmd).is_file() {
        return Err(ApiError::BadRequest(format!("未找到 CLI agent `{cmd}`——请先安装并加入 PATH")));
    }
    Ok(())
}

/// 生效配置解析：仓库行覆盖全局行；无设置时回退环境变量（开发期手段）。
pub struct ResolvedLlm {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

pub(crate) async fn service_of(st: &AppState, scope: &str, slot: &str) -> (String, Option<serde_json::Value>, Option<String>) {
    let binding = st.settings_repo.get(scope, &format!("slot.{slot}")).await.ok().flatten()
        .and_then(|r| serde_json::from_str::<String>(&r.value).ok())
        .unwrap_or_else(|| "default".into());
    let base = st.settings_repo.get(scope, &format!("llm.service.{binding}")).await.ok().flatten()
        .and_then(|r| serde_json::from_str::<serde_json::Value>(&r.value).ok());
    let key = st.settings_repo.get(scope, &format!("llm.service.{binding}.apiKey")).await.ok().flatten()
        .and_then(|r| if r.encrypted { st.cipher.decrypt(&r.value).ok() } else { Some(r.value) })
        .and_then(|v| serde_json::from_str::<String>(&v).ok());
    (binding, base, key)
}

/// 生效配置解析：仓库行覆盖全局行；无设置时回退环境变量（开发期手段）。
pub async fn resolve_llm(st: &AppState, repo_id: &str, slot: &str) -> ResolvedLlm {
    // 仓库级优先，全局兜底；key 在仓库级没有时回落全局
    let (mut binding, mut base, mut key) = service_of(st, repo_id, slot).await;
    if base.is_none() {
        let g = service_of(st, "global", slot).await;
        binding = g.0;
        base = g.1;
        if key.is_none() { key = g.2; }
    }
    let _ = binding;
    let obj = base.unwrap_or_default();
    ResolvedLlm {
        base_url: obj.get("baseUrl").and_then(|v| v.as_str()).map(Into::into)
            .or_else(|| std::env::var("EASYVIBE_LLM_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.anthropic.com".into()),
        api_key: key.or_else(|| std::env::var("EASYVIBE_LLM_API_KEY").ok()).unwrap_or_default(),
        model: obj.get("model").and_then(|v| v.as_str()).map(Into::into)
            .or_else(|| std::env::var("EASYVIBE_LLM_MODEL").ok())
            .unwrap_or_else(|| "claude-sonnet-4-5".into()),
    }
}
