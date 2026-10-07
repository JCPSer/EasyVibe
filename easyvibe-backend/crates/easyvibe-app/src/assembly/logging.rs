//! 装配格 · 启动前置：日志落盘、仓库注册、地图预热、agent 解析与受管资产解析。
//!
//! c-arch-10 R2/R3：自 `bootstrap.rs` **纯搬运**（顺序与语义逐字不变）。

use crate::assets;
use crate::state::{data_dir, read_desktop_repos, resolve_agent_command};
use easyvibe_map::{repo_from_root, MapService, Repo};
use tracing::info;

/// 日志落盘初始化（数据目录 logs/ 按天滚动，双写 stderr）。
/// 返回的 Guard 须由调用方持有到进程结束——否则非阻塞写线程提前 drop、日志丢失。
pub(crate) fn init() -> tracing_appender::non_blocking::WorkerGuard {
    let dir = data_dir().to_string_lossy().into_owned();
    let dir = format!("{dir}/logs");
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| eprintln!("日志目录创建失败 {dir}: {e}"));
    let appender = tracing_appender::rolling::daily(&dir, "easyvibe.log");
    let (nb, guard) = tracing_appender::non_blocking(appender);
    // 双写：文件（按天滚动，排障事实源）+ stderr（D5-2 桌面壳 pipe_child_logs 转发 sidecar
    // 日志用——壳日志与后端日志汇流到一处，只看一个流）
    use tracing_subscriber::prelude::*;
    let file_layer = tracing_subscriber::fmt::layer().with_writer(nb).with_ansi(false);
    let stderr_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr).with_ansi(false);
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new("info"))
        .with(file_layer)
        .with(stderr_layer)
        .init();
    guard
}

/// 仓库注册：`EASYVIBE_REPO`（开发期手段）+ `~/.easyvibe/desktop-repos` 持久化文件。
/// 合并去重、跳过不存在目录；两者皆空 = 零仓库起步（前端引导添加）。
pub(crate) fn load_repos() -> Vec<Repo> {
    let mut repo_roots: Vec<std::path::PathBuf> = std::env::var("EASYVIBE_REPO")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(Into::into)
        .collect();
    for p in read_desktop_repos() {
        if !repo_roots.contains(&p) {
            repo_roots.push(p);
        }
    }
    let repo_roots: Vec<_> = repo_roots.into_iter().filter(|p| p.is_dir()).collect();
    let repos: Vec<_> = repo_roots.iter().map(|p| repo_from_root(p)).collect();
    for r in &repos {
        info!("注册仓库 {} -> {}", r.id, r.root.display());
    }
    repos
}

/// MapService 预热缓存（不出残图：加载失败仅告警，不阻断启动）。
pub(crate) async fn warm_map(map_service: &MapService, repos: &[Repo]) {
    for r in repos {
        if let Err(e) = map_service.load_map(r).await {
            tracing::warn!("预热 {} 失败: {e}", r.id);
        }
    }
}

/// agent CLI 配置（命令/参数）+ 主提示词模板（env 可覆盖）。
pub(crate) struct AgentCfg {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) prompt_template: String,
}

/// 解析 agent 命令/参数与主提示词模板（含启动日志，顺序与原 `bootstrap.rs` 一致）。
pub(crate) fn resolve_agent_cfg() -> AgentCfg {
    let agent_command = std::env::var("EASYVIBE_AGENT_CMD").unwrap_or_else(|_| "claude".into());
    // GUI 启动的进程 PATH 极薄（launchd 只有 /usr/bin:/bin:...），裸命令名 spawn 必败——
    // 启动时把命令解析成绝对路径：PATH 直查 → 登录 shell PATH → 常见安装位兜底。
    let agent_command = resolve_agent_command(&agent_command);
    info!("[boot] agent CLI 解析为: {agent_command}");
    let agent_args: Vec<String> = std::env::var("EASYVIBE_AGENT_ARGS")
        .unwrap_or_else(|_| "-p --bare --dangerously-skip-permissions --output-format stream-json --verbose".into())
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    let map_spec = assets::spec("map_prompt");
    let (prompt_template, prompt_path) =
        assets::resolve_text_asset(map_spec.env, map_spec.name, map_spec.embedded, map_spec.persist);
    info!("agent={} args={:?} prompt={}", agent_command, agent_args, prompt_path);
    AgentCfg { command: agent_command, args: agent_args, prompt_template }
}

/// 提示词/schema 统一走解析链（env → exe 旁 → cwd → 编译期内嵌落盘），不再因缺文件直接 panic。
pub(crate) struct ResolvedAssets {
    pub(crate) patrol_prompt: String,
    pub(crate) schema_path: String,
    pub(crate) submap_prompt: String,
    pub(crate) incremental_prompt: String,
}

/// 解析巡检/schema/子图/增量四类受管文本资产（顺序与原 `bootstrap.rs` 一致）。
pub(crate) fn resolve_assets() -> ResolvedAssets {
    let patrol_spec = assets::spec("patrol");
    let (patrol_prompt, _) =
        assets::resolve_text_asset(patrol_spec.env, patrol_spec.name, patrol_spec.embedded, patrol_spec.persist);
    let schema_spec = assets::spec("schema");
    let (_, schema_path) =
        assets::resolve_text_asset(schema_spec.env, schema_spec.name, schema_spec.embedded, schema_spec.persist);
    let submap_spec = assets::spec("submap");
    let (submap_prompt, _) =
        assets::resolve_text_asset(submap_spec.env, submap_spec.name, submap_spec.embedded, submap_spec.persist);
    // 增量归纳 prompt（B 方案）：独立 env 名，不干扰主 prompt 的 env
    let incremental_spec = assets::spec("incremental");
    let (incremental_prompt, _) = assets::resolve_text_asset(
        incremental_spec.env,
        incremental_spec.name,
        incremental_spec.embedded,
        incremental_spec.persist,
    );
    ResolvedAssets { patrol_prompt, schema_path, submap_prompt, incremental_prompt }
}
