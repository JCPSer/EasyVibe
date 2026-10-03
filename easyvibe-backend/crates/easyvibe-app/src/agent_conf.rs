//! 执行 agent 配置体系（M1，方案 design §2/§3 施工）：
//! 预设表 + settings 优先解析链（env 降级为 fallback）+ 并行探测。
//! 纪律：spawn 前现读（与 resolve_llm 同一哲学）——配置改动对下一次 spawn 生效。

use easyvibe_db::SettingsRepository as _;

/// 单条 agent 预设（后端唯一事实源——前端经 GET /api/agent/status 的 presets 字段消费）
pub struct AgentPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub default_args: &'static [&'static str],
    pub agent_type: &'static str, // claude（stream-json 解析）/ plain（完成态纯文本）
    pub stability: &'static str,  // stable / experimental
}

/// 本期预设三家（拍板 2026-10-03）：claude 稳定；codex/opencode 实验（plain 无流式）
pub const PRESETS: &[AgentPreset] = &[
    AgentPreset {
        id: "claude",
        label: "Claude Code",
        default_args: &["-p", "--bare", "--dangerously-skip-permissions", "--output-format", "stream-json", "--verbose"],
        agent_type: "claude",
        stability: "stable",
    },
    AgentPreset {
        id: "codex",
        label: "Codex CLI",
        default_args: &["exec"],
        agent_type: "plain",
        stability: "experimental",
    },
    AgentPreset {
        id: "opencode",
        label: "OpenCode",
        default_args: &["run"],
        agent_type: "plain",
        stability: "experimental",
    },
];

pub fn preset_by_id(id: &str) -> Option<&'static AgentPreset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// 解析结果：spawn 直接消费
pub struct ResolvedAgent {
    pub command: String,       // 绝对路径（经兜底链解析；未找到时回落为原命令名）
    pub args: Vec<String>,
    pub agent_type: String,    // claude / plain
    pub source: String,        // settings / env / default（命令来源）
    pub preset: String,        // 生效预设 id（解析用；custom 表示用户改参）
}

/// settings 优先解析链（方案 §2.2）：
/// 1. command：settings.agent.command → env_command → "claude"
/// 2. args：槽位参数（整体替换）→ settings.agent.args.global → env_args → 当前 preset 默认 → claude 默认
/// 3. 绝对路径兜底链；4. type：settings → preset 推断 → custom 按 args 含 stream-json 推断
/// 读一条 agent 设置（空值视为未配置）
async fn cfg(settings: &easyvibe_db::SqliteSettingsRepository, key: &str) -> Option<String> {
    settings
        .get("global", key)
        .await
        .ok()
        .flatten()
        .map(|r| r.value)
        .filter(|v| !v.trim().is_empty())
}

pub async fn resolve_agent(
    settings: &easyvibe_db::SqliteSettingsRepository,
    slot: Option<&str>,
    env_command: &str,
    env_args: &[String],
) -> ResolvedAgent {
    // 1. 命令
    let (command, source) = match cfg(settings, "agent.command").await {
        Some(c) => (c, "settings".to_string()),
        None => {
            if env_command == "claude" {
                ("claude".to_string(), "default".to_string())
            } else {
                (env_command.to_string(), "env".to_string())
            }
        }
    };

    // 2. 参数（槽位整体替换 → 全局 → env → preset 默认）
    let preset_id = cfg(settings, "agent.preset").await.unwrap_or_else(|| "claude".into());
    let default_args = |id: &str| {
        preset_by_id(id)
            .map(|p| p.default_args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
            .unwrap_or_else(|| preset_by_id("claude").unwrap().default_args.iter().map(|s| s.to_string()).collect())
    };
    let args: Vec<String> = if let Some(slot) = slot {
        match cfg(settings, &format!("agent.args.{slot}")).await {
            Some(v) => serde_json::from_str(&v).unwrap_or_else(|_| default_args(&preset_id)),
            None => match cfg(settings, "agent.args.global").await {
                Some(v) => serde_json::from_str(&v).unwrap_or_else(|_| default_args(&preset_id)),
                None => {
                    if !env_args.is_empty() {
                        env_args.to_vec()
                    } else {
                        default_args(&preset_id)
                    }
                }
            },
        }
    } else {
        match cfg(settings, "agent.args.global").await {
            Some(v) => serde_json::from_str(&v).unwrap_or_else(|_| default_args(&preset_id)),
            None => {
                if !env_args.is_empty() {
                    env_args.to_vec()
                } else {
                    default_args(&preset_id)
                }
            }
        }
    };

    // 3. 绝对路径
    let command = resolve_agent_command(&command);

    // 4. 协议类型
    let agent_type = match cfg(settings, "agent.type").await {
        Some(t) => t,
        None => match preset_by_id(&preset_id) {
            Some(p) if preset_id != "custom" => p.agent_type.to_string(),
            // custom（或未知 preset）：按 args 是否含 stream-json 推断（与 session lib 解析探测一致）
            _ => {
                if args.iter().any(|a| a.contains("stream-json")) {
                    "claude".into()
                } else {
                    "plain".into()
                }
            }
        },
    };

    ResolvedAgent { command, args, agent_type, source, preset: preset_id }
}

/// 命令绝对路径解析（GUI PATH 极薄，三层兜底：PATH 直查 → 登录 shell → 常见安装位）
pub fn resolve_agent_command(cmd: &str) -> String {
    let as_path = std::path::Path::new(cmd);
    if as_path.components().count() > 1 && as_path.is_file() {
        return cmd.to_string();
    }
    // Windows 可执行带 .exe/.cmd 后缀；PATH 分隔符平台相关（split_paths 自动处理 ':'/';'）
    let names: Vec<String> = if cfg!(windows) {
        vec![cmd.to_string(), format!("{cmd}.exe"), format!("{cmd}.cmd"), format!("{cmd}.bat")]
    } else {
        vec![cmd.to_string()]
    };
    if let Some(p) = std::env::var("PATH").ok().as_deref().map(|dirs| {
        std::env::split_paths(dirs)
            .filter(|d| !d.as_os_str().is_empty())
            .flat_map(|d| names.iter().map(move |n| format!("{}/{n}", d.display())))
            .find(|p| std::path::Path::new(p).is_file())
    }).flatten() {
        return p;
    }
    #[cfg(unix)]
    if let Ok(out) = std::process::Command::new("bash").args(["-lc", &format!("command -v {cmd}")]).output() {
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if out.status.success() && !p.is_empty() && std::path::Path::new(&p).is_file() {
            return p;
        }
    }
    // Windows 无登录 shell 概念；HOME 缺失时 USERPROFILE 兜底（GUI 启动两者皆薄）
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty())
        .or_else(|| std::env::var("USERPROFILE").ok().filter(|h| !h.is_empty()))
        .unwrap_or_default();
    for cand in [
        format!("{home}/.claude/local/{cmd}"),
        format!("{home}/.local/bin/{cmd}"),
        format!("{home}/.npm-global/bin/{cmd}"),
        format!("/opt/homebrew/bin/{cmd}"),
        format!("/usr/local/bin/{cmd}"),
        format!("{home}/Library/Application Support/kimi-desktop/daimon-share/daimon/npm-global/bin/{cmd}"),
    ] {
        if std::path::Path::new(&cand).is_file() {
            return cand;
        }
    }
    cmd.to_string()
}

/// 探测结果（内存态，不落盘）
#[derive(Debug, Clone, serde::Serialize)]
pub struct DetectedAgent {
    pub command: String,
    pub path: String,
    pub version: Option<String>,
}

/// 按优先级并行探测三家（方案 §3.1：三路并行，最坏 5s；探测失败不阻断启动）
pub async fn detect_agents() -> Vec<DetectedAgent> {
    let (a, b, c) = futures_util::join!(probe("claude"), probe("codex"), probe("opencode"));
    [a, b, c].into_iter().flatten().collect()
}

async fn probe(command: &str) -> Option<DetectedAgent> {
    let path = resolve_agent_command(command);
    if !std::path::Path::new(&path).is_file() {
        return None;
    }
    let version = probe_version(&path).await;
    Some(DetectedAgent { command: command.into(), path, version })
}

/// `--version` 探测（5s 超时；非 0 退出或无输出视为无版本）
async fn probe_version(path: &str) -> Option<String> {
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new(path).arg("--version").output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").trim().to_string();
    if line.is_empty() {
        None
    } else {
        Some(line.chars().take(80).collect())
    }
}

/// 测试连接结果（内存态，最近一条）
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentTestResult {
    pub ok: bool,
    pub latency_ms: u64,
    pub protocol: String, // compatible / incompatible: <原因>
    pub sample: String,
}

/// 协议兼容测试（方案 §3.2）：最小 prompt 真实会话，30s 上限。
/// 用独立假仓库 id 注册会话——不与任何真实仓库的单会话纪律冲突。
pub async fn run_agent_test(sessions: &easyvibe_session::SessionManager, resolved: &ResolvedAgent) -> AgentTestResult {
    let started = std::time::Instant::now();
    let repo_id = "__agent_test__";
    let session = match sessions
        .start_induction(
            repo_id,
            &std::env::temp_dir(),
            "只输出 ok 两个字母，不要输出其他任何内容",
            &resolved.command,
            &resolved.args,
            Some(std::time::Duration::from_secs(30)),
        )
        .await
    {
        Ok(s) => s,
        Err(e) => {
            return AgentTestResult {
                ok: false,
                latency_ms: started.elapsed().as_millis() as u64,
                protocol: format!("incompatible: 无法启动（{e}）"),
                sample: String::new(),
            }
        }
    };
    let sid = session.session_id.clone();
    // 等终态（30s 总上限；会话自身超时也会杀）
    let outcome = loop {
        match sessions.status_of_session(&sid).await {
            Some(s)
                if matches!(
                    s.status,
                    easyvibe_api_types::SessionStatus::Succeeded | easyvibe_api_types::SessionStatus::Failed
                ) =>
            {
                break s.status == easyvibe_api_types::SessionStatus::Succeeded
            }
            None => {
                return AgentTestResult {
                    ok: false,
                    latency_ms: started.elapsed().as_millis() as u64,
                    protocol: "incompatible: 会话状态丢失".into(),
                    sample: String::new(),
                }
            }
            _ => {
                if started.elapsed() > std::time::Duration::from_secs(32) {
                    let _ = sessions.kill(&sid).await;
                    return AgentTestResult {
                        ok: false,
                        latency_ms: started.elapsed().as_millis() as u64,
                        protocol: "incompatible: 超时（30s 无终态）".into(),
                        sample: String::new(),
                    };
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    };
    let out = sessions.take_output(&sid).await.unwrap_or_default();
    let sample: String = out.chars().take(200).collect();
    let latency_ms = started.elapsed().as_millis() as u64;
    if !outcome {
        // Failed = 非 0 退出；无输出则按未消费 prompt 说
        let protocol = if out.trim().is_empty() {
            "incompatible: 未消费 prompt/无输出（非 0 退出）".to_string()
        } else {
            "incompatible: 非 0 退出".to_string()
        };
        return AgentTestResult { ok: false, latency_ms, protocol, sample };
    }
    if out.trim().is_empty() {
        return AgentTestResult { ok: false, latency_ms, protocol: "incompatible: 无输出".into(), sample };
    }
    AgentTestResult { ok: true, latency_ms, protocol: "compatible".into(), sample }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_settings() -> easyvibe_db::SqliteSettingsRepository {
        let db = easyvibe_db::Database::connect_memory().await.unwrap();
        easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())
    }

    async fn set(settings: &easyvibe_db::SqliteSettingsRepository, key: &str, value: &str) {
        use easyvibe_db::SettingsRepository as _;
        settings
            .set(&easyvibe_db::SettingRow {
                scope: "global".into(),
                key: key.into(),
                value: value.into(),
                encrypted: false,
                updated_at: "t".into(),
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn resolve_chain_settings_priority_and_fallbacks() {
        let s = mem_settings().await;
        // 全缺省：claude 预设（source=default）
        let r = resolve_agent(&s, None, "claude", &[]).await;
        assert_eq!(r.preset, "claude");
        assert_eq!(r.source, "default");
        assert!(r.args.contains(&"--output-format".into()), "缺省用 claude 预设参数");
        assert_eq!(r.agent_type, "claude");

        // env 命令 → source=env
        let r = resolve_agent(&s, None, "/usr/local/bin/codex", &[]).await;
        assert_eq!(r.source, "env");

        // settings.command 优先于 env
        set(&s, "agent.command", "/bin/echo").await;
        let r = resolve_agent(&s, None, "claude", &[]).await;
        assert_eq!(r.command, "/bin/echo", "settings 命令优先");
        assert_eq!(r.source, "settings");

        // settings.args.global 优先于 env_args
        set(&s, "agent.args.global", r#"["hello"]"#).await;
        let r = resolve_agent(&s, None, "claude", &["ignored".into()]).await;
        assert_eq!(r.args, vec!["hello".to_string()]);

        // 槽位参数整体替换全局
        set(&s, "agent.args.review", r#"["--cheap-model"]"#).await;
        let r = resolve_agent(&s, Some("review"), "claude", &[]).await;
        assert_eq!(r.args, vec!["--cheap-model".to_string()], "槽位参数整体替换");
        let r = resolve_agent(&s, None, "claude", &[]).await;
        assert_eq!(r.args, vec!["hello".to_string()], "无槽位仍用全局");
    }

    #[tokio::test]
    async fn resolve_type_inference_including_custom() {
        let s = mem_settings().await;
        // preset=codex → plain
        set(&s, "agent.preset", "codex").await;
        let r = resolve_agent(&s, None, "claude", &[]).await;
        assert_eq!(r.agent_type, "plain");
        // preset=custom 且 args 含 stream-json → claude（与 session lib 探测一致）
        set(&s, "agent.preset", "custom").await;
        set(&s, "agent.args.global", r#"["--output-format","stream-json"]"#).await;
        let r = resolve_agent(&s, None, "claude", &[]).await;
        assert_eq!(r.agent_type, "claude", "custom 按 stream-json 推断");
        // 不含 → plain
        set(&s, "agent.args.global", r#"["exec"]"#).await;
        let r = resolve_agent(&s, None, "claude", &[]).await;
        assert_eq!(r.agent_type, "plain");
        // settings.agent.type 显式优先
        set(&s, "agent.type", "claude").await;
        let r = resolve_agent(&s, None, "claude", &[]).await;
        assert_eq!(r.agent_type, "claude");
    }

    #[tokio::test]
    async fn probe_version_echo_and_missing() {
        // 存在且 --version 有输出 → Some
        let v = probe_version("/bin/echo").await;
        assert!(v.is_some(), "echo --version 实际输出版本首行: {:?}", v);
        // 不存在 → None（不 panic 不挂起）
        let v = probe_version("/nonexistent/definitely-not-here").await;
        assert!(v.is_none());
    }
}
