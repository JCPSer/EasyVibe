//! M4-4 Git 工作树：状态/历史查询 + 写操作（提交/拉取/推送/撤销）。
//! 命令统一走 tokio::process + 超时；解析逻辑全部是纯函数，单测覆盖（含真 git 集成测试）。
//! 安全：写操作只接受相对路径且禁止 `..` 越界；discard 按跟踪状态区分 checkout/clean。
use axum::{
    extract::{Path as AxumPath, State},
    response::{IntoResponse, Response},
    Json,
};
use easyvibe_common::ApiError;
use std::path::Path;
use std::time::Duration;

use crate::{resolve_llm, AppError, AppState, LlmMode};

const GIT_TIMEOUT: Duration = Duration::from_secs(30);

async fn git(repo: &Path, args: &[&str]) -> Result<String, ApiError> {
    let out = tokio::time::timeout(
        GIT_TIMEOUT,
        tokio::process::Command::new("git").args(args).current_dir(repo).output(),
    )
    .await
    .map_err(|_| ApiError::Internal(format!("git {args:?} 超时（{GIT_TIMEOUT:?}）")))?
    .map_err(|e| ApiError::Internal(format!("git 执行失败: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(ApiError::BadRequest(format!("git {args:?} 失败: {stderr}")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn git_opt(repo: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).current_dir(repo).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

// ---------- 数据形状 ----------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFile {
    /// 归一化状态：M 修改 / A 新增（含暂存）/ D 删除 / R 重命名 / ? 未跟踪
    pub status: char,
    pub path: String,
    /// 重命名来源路径（仅 R）
    pub orig: Option<String>,
    /// 增删行数（未跟踪文件无 diff 数据，为 None）
    pub adds: Option<i64>,
    pub dels: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct GitStatus {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub files: Vec<GitFile>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct GitLogRow {
    pub hash: String,
    pub short: String,
    pub author: String,
    pub email: String,
    pub at: i64,
    pub subject: String,
    /// 本提交触及的文件（--name-only；前端映射模块 chips 用）
    pub files: Vec<String>,
}

// ---------- 解析（纯函数） ----------

/// 解析 `git status --porcelain=v1` 输出（非 -z：路径含特殊字符时带双引号，本期不支持此类路径）。
pub fn parse_porcelain(text: &str) -> Vec<GitFile> {
    let mut out = Vec::new();
    for line in text.lines() {
        let bytes = line.as_bytes();
        if bytes.len() < 4 {
            continue;
        }
        let x = bytes[0] as char;
        let y = bytes[1] as char;
        let rest = &line[3..];
        if x == '?' && y == '?' {
            out.push(GitFile { status: '?', path: rest.to_string(), orig: None, adds: None, dels: None });
            continue;
        }
        let eff = if y != ' ' { y } else { x };
        let status = match eff {
            'M' | 'T' => 'M',
            'A' => 'A',
            'D' => 'D',
            'R' | 'C' => 'R',
            'U' => 'M', // 冲突按修改呈现（细节在 diff 阶段处理）
            _ => continue,
        };
        if status == 'R' {
            // 形如 "R  old -> new"
            if let Some((from, to)) = rest.split_once(" -> ") {
                out.push(GitFile { status, path: to.trim().to_string(), orig: Some(from.trim().to_string()), adds: None, dels: None });
                continue;
            }
        }
        out.push(GitFile { status, path: rest.to_string(), orig: None, adds: None, dels: None });
    }
    out
}

/// 解析 `git diff HEAD --numstat`：合并进文件列表的增删统计（未跟踪文件不在其中，保持 None）。
pub fn parse_numstat(text: &str) -> Vec<(String, Option<i64>, Option<i64>)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(a), Some(d), Some(p)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let (adds, dels) = match (a.parse::<i64>(), d.parse::<i64>()) {
            (Ok(a), Ok(d)) => (Some(a), Some(d)),
            _ => (None, None), // 二进制文件为 "-\t-"
        };
        let path = rename_target(p);
        out.push((path, adds, dels));
    }
    out
}

/// numstat 重命名路径 "src/{old => new}/file" → "src/new/file"
fn rename_target(p: &str) -> String {
    if let Some((left, right)) = p.split_once(" => ") {
        // 前缀在 '{' 之前（"src/{old" → "src/"），右侧首段到 '}' 为止是新目录名
        let prefix = left.split_once('{').map(|(pre, _)| pre).unwrap_or("");
        let right = right.trim_start_matches('{');
        if let Some((first, rest)) = right.split_once('}') {
            return format!("{prefix}{first}{rest}");
        }
        return format!("{prefix}{right}");
    }
    p.to_string()
}

/// 解析 `git log --format=%x1e%H%x1f%h%x1f%an%x1f%ae%x1f%at%x1f%s --name-only` 输出。
/// %x1e 为记录分隔符：每个提交块 = 首行字段（\x1f 分隔）+ 后续非空行的文件清单。
pub fn parse_log(text: &str) -> Vec<GitLogRow> {
    text.split('\u{1e}')
        .filter(|rec| !rec.trim().is_empty())
        .filter_map(|rec| {
            let mut lines = rec.lines();
            let head = lines.next()?;
            let f: Vec<&str> = head.split('\u{1f}').collect();
            if f.len() < 6 {
                return None;
            }
            Some(GitLogRow {
                hash: f[0].to_string(),
                short: f[1].to_string(),
                author: f[2].to_string(),
                email: f[3].to_string(),
                at: f[4].parse().unwrap_or(0),
                subject: f[5].to_string(),
                files: lines.map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect(),
            })
        })
        .collect()
}

// ---------- 查询 ----------

pub async fn status(repo: &Path) -> Result<GitStatus, ApiError> {
    let Some(branch) = git_opt(repo, &["rev-parse", "--abbrev-ref", "HEAD"]) else {
        return Err(ApiError::BadRequest("不是 git 仓库或无提交".into()));
    };
    // 无上游分支 → 未推送语义用本地分支表达（ahead/behind 置 0）
    let upstream = git_opt(repo, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]);
    let (ahead, behind) = upstream
        .as_ref()
        .and_then(|_| git_opt(repo, &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"]))
        .and_then(|s| {
            let mut it = s.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .unwrap_or((0, 0));

    let porcelain = git(repo, &["status", "--porcelain=v1"]).await.unwrap_or_default();
    let mut files = parse_porcelain(&porcelain);
    let numstat = git(repo, &["diff", "HEAD", "--numstat"]).await.unwrap_or_default();
    let stats_list = parse_numstat(&numstat);
    let stats: std::collections::HashMap<&str, (Option<i64>, Option<i64>)> =
        stats_list.iter().map(|(p, a, d)| (p.as_str(), (*a, *d))).collect();
    for f in &mut files {
        if let Some((a, d)) = stats.get(f.path.as_str()) {
            f.adds = *a;
            f.dels = *d;
        }
    }
    Ok(GitStatus { branch, upstream, ahead, behind, files })
}

pub async fn log(repo: &Path, limit: i64) -> Result<Vec<GitLogRow>, ApiError> {
    let out = git(repo, &["log", &format!("-n{limit}"), "--format=%x1e%H%x1f%h%x1f%an%x1f%ae%x1f%at%x1f%s", "--name-only"]).await?;
    Ok(parse_log(&out))
}

// ---------- 提交详情（历史记录可点开——用户反馈：此前纯展示） ----------

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitFileStat {
    pub path: String,
    pub adds: i64,
    pub dels: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetail {
    pub hash: String,
    pub subject: String,
    pub body: String,
    pub files: Vec<CommitFileStat>,
}

/// git show 单提交：%x1f 分隔头部字段，--numstat 逐文件增删行
pub async fn show_commit(repo: &Path, hash: &str) -> Result<CommitDetail, ApiError> {
    if hash.len() < 6 || hash.len() > 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ApiError::BadRequest("非法提交 hash".into()));
    }
    let out = git(repo, &["show", "--format=%H%x1f%s%x1f%b%x1e", "--numstat", hash]).await?;
    let mut parts = out.splitn(2, '\u{1e}');
    let head = parts.next().unwrap_or_default();
    let numstat = parts.next().unwrap_or_default();
    let mut fields = head.split('\u{1f}');
    let hash_full = fields.next().unwrap_or_default().trim().to_string();
    let subject = fields.next().unwrap_or_default().trim().to_string();
    let body = fields.next().unwrap_or_default().trim().to_string();
    let files = numstat
        .lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            let adds = it.next()?.parse::<i64>().ok().unwrap_or(0);
            let dels = it.next()?.parse::<i64>().ok().unwrap_or(0);
            let path = it.next()?.trim().to_string();
            if path.is_empty() { None } else { Some(CommitFileStat { path, adds, dels }) }
        })
        .collect();
    if hash_full.is_empty() {
        return Err(ApiError::NotFound(format!("提交 {hash} 不存在")));
    }
    Ok(CommitDetail { hash: hash_full, subject, body, files })
}

pub async fn get_git_commit(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let hash = q.get("hash").ok_or_else(|| ApiError::BadRequest("缺少 hash 参数".into()))?;
    let detail = show_commit(&repo.root, hash).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": detail })).into_response())
}


// ---------- HTTP handlers：查询 ----------

pub async fn get_git_status(State(st): State<AppState>, AxumPath(id): AxumPath<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let s = status(&repo.root).await?;
    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "branch": s.branch, "upstream": s.upstream, "ahead": s.ahead, "behind": s.behind,
            "files": s.files.iter().map(|f| serde_json::json!({
                "status": f.status.to_string(), "path": f.path, "orig": f.orig, "adds": f.adds, "dels": f.dels,
            })).collect::<Vec<_>>(),
        },
    }))
    .into_response())
}

pub async fn get_git_log(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let limit = q.get("limit").and_then(|l| l.parse::<i64>().ok()).unwrap_or(30).clamp(1, 100);
    let rows = log(&repo.root, limit).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": rows })).into_response())
}

// ---------- 写操作 ----------

pub async fn commit_all(repo: &Path, message: &str) -> Result<String, ApiError> {
    let message = message.trim();
    if message.is_empty() {
        return Err(ApiError::BadRequest("提交说明不能为空".into()));
    }
    // R3 C5：产品账簿（地图/归档/会话产物）不进用户提交历史——
    // ① .easyvibe/ 含巡检写回的 map.json，add -A 提交会立刻触发 freshness 自反漂移（越健康越亮警告）
    // ② .claude/ 是 agent STAR 归档，与 .easyvibe/development_docs/ 是同一留痕的两份副本
    // pathspec 魔法符 :(exclude) 需置于最后；无账簿目录的仓库行为与 add -A 等价
    git(repo, &["add", "-A", "--", ".", ":(exclude).easyvibe", ":(exclude).claude"]).await?;
    git(repo, &["commit", "-m", message]).await?;
    git(repo, &["rev-parse", "--short", "HEAD"]).await.map(|s| s.trim().to_string())
}

pub async fn pull(repo: &Path) -> Result<(), ApiError> {
    git(repo, &["pull", "--rebase", "--autostash"]).await.map(|_| ())
}

pub async fn push(repo: &Path) -> Result<(), ApiError> {
    git(repo, &["push"]).await.map(|_| ())
}

/// 撤销单个文件的未提交改动：已跟踪 → checkout；未跟踪 → clean
pub async fn discard(repo: &Path, path: &str) -> Result<(), ApiError> {
    validate_rel_path(path)?;
    let st = status(repo).await?;
    let untracked = st.files.iter().any(|f| f.path == path && f.status == '?');
    if untracked {
        git(repo, &["clean", "-f", "--", path]).await.map(|_| ())
    } else {
        git(repo, &["checkout", "--", path]).await.map(|_| ())
    }
}

/// 全部撤销（重审 P2）：tracked 恢复 + untracked 删除（含未跟踪目录）。
/// 前端两步确认后经 discard 端点 path="*" 到达。
pub async fn discard_all(repo: &Path) -> Result<(), ApiError> {
    git(repo, &["checkout", "--", "."]).await?;
    git(repo, &["clean", "-fd"]).await.map(|_| ())
}

/// 路径安全：仅相对路径、禁止 `..` 分量（写操作统一入口）
fn validate_rel_path(path: &str) -> Result<(), ApiError> {
    let p = Path::new(path);
    if p.is_absolute() || path.split('/').any(|s| s == "..") || path.is_empty() {
        return Err(ApiError::BadRequest(format!("非法路径: {path}")));
    }
    Ok(())
}

// ---------- HTTP handlers：写操作 ----------

pub async fn post_git_commit(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let message = body["message"].as_str().unwrap_or_default();
    let short = commit_all(&repo.root, message).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "shortHash": short } })).into_response())
}

pub async fn post_git_pull(State(st): State<AppState>, AxumPath(id): AxumPath<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    pull(&repo.root).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": true })).into_response())
}

pub async fn post_git_push(State(st): State<AppState>, AxumPath(id): AxumPath<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    push(&repo.root).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": true })).into_response())
}

pub async fn post_git_discard(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let path = body["path"].as_str().ok_or_else(|| ApiError::BadRequest("缺少 path".into()))?;
    // 重审 P2：全部撤销（path="*"）——tracked 恢复 + untracked 删除（含未跟踪目录）。
    // 前端两步确认后调用；与逐文件撤销同一端点，语义由 path 值区分。
    if path == "*" {
        discard_all(&repo.root).await?;
        return Ok(Json(serde_json::json!({ "success": true, "data": true })).into_response());
    }
    discard(&repo.root, path).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": true })).into_response())
}

/// M4-4 提交把关台：从任务上下文 + 影响面 AI 生成提交说明（Conventional Commits 单行）。
/// footer（EasyVibe-Task: <id>）由后端一并返回，提交时随说明写入，历史可反查任务。
#[derive(serde::Deserialize)]
pub(crate) struct CommitMessageRequest {
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    modules: Vec<String>,
    #[serde(default)]
    diff_stat: String,
}

pub async fn post_git_commit_message(
    State(st): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<CommitMessageRequest>,
) -> Result<Response, AppError> {
    use easyvibe_db::TaskRepository as _;
    let _repo = st.map_service.find_repo(&id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;

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
            let cfg = resolve_llm(&st, &id, "chat").await;
            if cfg.api_key.is_empty() {
                return Err(AppError(ApiError::BadRequest("未配置 LLM API key（设置面板或 EASYVIBE_LLM_API_KEY）".into())));
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
        return Err(AppError(ApiError::Internal("LLM 未产出提交说明".into())));
    }
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": message, "footer": footer } })).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_basic_kinds() {
        let text = " M src/a.java\nA  src/b.java\n?? new/file.ts\n D gone.sql\nR  old.yml -> new.yml\n M \"quoted path.java\"\n";
        let fs = parse_porcelain(text);
        assert_eq!(fs.len(), 6);
        assert_eq!(fs[0], GitFile { status: 'M', path: "src/a.java".into(), orig: None, adds: None, dels: None });
        assert_eq!(fs[1].status, 'A');
        assert_eq!(fs[2], GitFile { status: '?', path: "new/file.ts".into(), orig: None, adds: None, dels: None });
        assert_eq!(fs[3].status, 'D');
        assert_eq!(fs[4].orig.as_deref(), Some("old.yml"));
        assert_eq!(fs[4].path, "new.yml");
        assert_eq!(fs[5].path, "\"quoted path.java\"", "带引号路径原样保留（前端展示）");
    }

    #[test]
    fn numstat_merge_and_rename() {
        let text = "12\t3\tsrc/a.java\n-\t-\tbin/logo.png\n1\t1\tsrc/{old => new}/c.java\n";
        let rows = parse_numstat(text);
        assert_eq!(rows[0], ("src/a.java".into(), Some(12), Some(3)));
        assert_eq!(rows[1], ("bin/logo.png".into(), None, None), "二进制无统计");
        assert_eq!(rows[2].0, "src/new/c.java", "重命名取目标路径");
    }

    #[test]
    fn log_parse_roundtrip() {
        // git 实际格式：%x1e 记录分隔 + 字段全用 \x1f 分隔 + --name-only 文件行
        let text = "\u{1e}abcdef123456\u{1f}abcdef1\u{1f}张伟\u{1f}zw@ex.com\u{1f}1790871442\u{1f}fix: x\nsrc/a.java\nsrc/b.java\n\u{1e}fedcba654321\u{1f}fedcba6\u{1f}李雷\u{1f}ll@ex.com\u{1f}1790870000\u{1f}feat: y\n\n";
        let rows = parse_log(text);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].short, "abcdef1");
        assert_eq!(rows[0].at, 1790871442);
        assert_eq!(rows[0].subject, "fix: x");
        assert_eq!(rows[0].files, vec!["src/a.java".to_string(), "src/b.java".to_string()]);
        assert_eq!(rows[1].files.len(), 0, "空行不产生文件项");
    }

    #[test]
    fn rel_path_guard() {
        assert!(validate_rel_path("src/a.java").is_ok());
        assert!(validate_rel_path("../etc/passwd").is_err());
        assert!(validate_rel_path("a/../../b").is_err());
        assert!(validate_rel_path("/abs/path").is_err());
        assert!(validate_rel_path("").is_err());
    }

    #[tokio::test]
    async fn status_commit_discard_e2e() {
        let repo = std::env::temp_dir().join("ev-git-status-test");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        let git_sync = |args: &[&str]| {
            std::process::Command::new("git").args(args).current_dir(&repo).output().expect("git 执行失败")
        };
        git_sync(&["init", "-q"]);
        git_sync(&["config", "user.email", "t@t"]);
        git_sync(&["config", "user.name", "t"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        git_sync(&["add", "."]);
        git_sync(&["commit", "-q", "-m", "init"]);

        // 修改已跟踪 + 新增未跟踪
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(repo.join("b_new.txt"), "new\n").unwrap();
        let st = status(&repo).await.unwrap();
        assert_eq!(st.branch, "main", "默认分支（init 默认 main）");
        assert!(st.upstream.is_none(), "无远程 → upstream None");
        let a = st.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!(a.status, 'M');
        assert_eq!(a.adds, Some(1), "numstat 合并出增行");
        assert_eq!(a.dels, Some(0));
        let b = st.files.iter().find(|f| f.path == "b_new.txt").unwrap();
        assert_eq!(b.status, '?');
        assert_eq!(b.adds, None, "未跟踪文件无 diff 统计");

        // 提交（add -A 含未跟踪）
        let short = commit_all(&repo, "second").await.unwrap();
        assert_eq!(short.len(), 7);
        let st = status(&repo).await.unwrap();
        assert!(st.files.is_empty(), "提交后工作树干净");
        let rows = log(&repo, 10).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].subject, "second");
        assert!(rows[0].files.contains(&"a.txt".to_string()) && rows[0].files.contains(&"b_new.txt".to_string()), "--name-only 带出文件清单: {:?}", rows[0].files);

        // 撤销已跟踪修改 → 内容还原；撤销未跟踪 → 文件删除
        std::fs::write(repo.join("a.txt"), "changed\n").unwrap();
        std::fs::write(repo.join("c_un.txt"), "x\n").unwrap();
        discard(&repo, "a.txt").await.unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "one\ntwo\n");
        discard(&repo, "c_un.txt").await.unwrap();
        assert!(!repo.join("c_un.txt").exists());

        // 空提交说明被拒
        assert!(commit_all(&repo, "   ").await.is_err());
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[tokio::test]
    async fn commit_all_excludes_bookkeeping_dirs() {
        // R3 C5 回归：把关台提交不得带入 .easyvibe/（巡检写回的 map.json——否则 freshness 立刻自反漂移）
        // 与 .claude/（agent STAR 归档）——产品账簿不进用户提交历史
        let dir = std::env::temp_dir().join("ev-commit-exclude-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".easyvibe/map")).unwrap();
        std::fs::create_dir_all(dir.join(".claude/development_docs")).unwrap();
        // 故意不写 .gitignore：排除必须来自 commit_all 的 pathspec，而非 gitignore 的副作用
        let git_sync = |args: &[&str]| {
            assert!(std::process::Command::new("git").args(args).current_dir(&dir).status().unwrap().success())
        };
        git_sync(&["init", "-q"]);
        git_sync(&["config", "user.email", "t@t"]);
        git_sync(&["config", "user.name", "t"]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        git_sync(&["add", "."]);
        git_sync(&["commit", "-q", "-m", "init"]);

        // 用户改动 + 账簿变动（等价于巡检写回 map.json 与 STAR 归档）
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(dir.join(".easyvibe/map/map.json"), "{}").unwrap();
        std::fs::write(dir.join(".claude/development_docs/t1.json"), "{}").unwrap();

        let short = commit_all(&dir, "feature").await.unwrap();
        assert_eq!(short.len(), 7);
        let rows = log(&dir, 5).await.unwrap();
        assert!(rows[0].files.contains(&"a.txt".to_string()), "用户改动必须提交: {:?}", rows[0].files);
        assert!(
            !rows[0].files.iter().any(|f| f.starts_with(".easyvibe") || f.starts_with(".claude")),
            "产品账簿不得进用户提交历史: {:?}",
            rows[0].files
        );
        assert!(dir.join(".easyvibe/map/map.json").exists() && dir.join(".claude/development_docs/t1.json").exists(), "账簿文件必须留在磁盘（不被提交也不被吞掉）");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn show_commit_parses_header_and_numstat() {
        // 历史记录可点开（用户反馈）：提交详情 = 主题 + 正文 + 逐文件增删
        let dir = std::env::temp_dir().join("ev-show-commit-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git_sync = |args: &[&str]| {
            assert!(std::process::Command::new("git").args(args).current_dir(&dir).status().unwrap().success())
        };
        git_sync(&["init", "-q"]);
        git_sync(&["config", "user.email", "t@t"]);
        git_sync(&["config", "user.name", "t"]);
        std::fs::write(dir.join("a.txt"), "1\n").unwrap();
        git_sync(&["add", "."]);
        git_sync(&["commit", "-q", "-m", "标题行", "-m", "正文段落一\n正文段落二"]);
        let head = std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&dir).output().unwrap();
        let hash = String::from_utf8_lossy(&head.stdout).trim().to_string();

        let d = show_commit(&dir, &hash).await.unwrap();
        assert_eq!(d.hash, hash);
        assert_eq!(d.subject, "标题行");
        assert!(d.body.contains("正文段落一"), "正文必须带出: {:?}", d.body);
        assert_eq!(d.files.len(), 1);
        assert_eq!(d.files[0].path, "a.txt");
        assert_eq!(d.files[0].adds, 1);

        // 非法 hash 必须 400 而不是进 git
        assert!(show_commit(&dir, "zzzz!!").await.is_err());
        assert!(show_commit(&dir, "abc").await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn non_git_repo_errs() {
        let dir = std::env::temp_dir().join("ev-not-git-status");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(status(&dir).await.is_err());
        assert!(log(&dir, 10).await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn discard_all_restores_tracked_and_removes_untracked() {
        // 重审 P2：全部撤销——tracked 恢复 + untracked（含目录）删除
        let dir = std::env::temp_dir().join(format!("ev-discard-all-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]).await.unwrap();
        std::fs::write(dir.join("tracked.txt"), "v1").unwrap();
        git(&dir, &["add", "."]).await.unwrap();
        git(&dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]).await.unwrap();
        // 造改动：改 tracked + 新增 untracked 文件与目录
        std::fs::write(dir.join("tracked.txt"), "v2-changed").unwrap();
        std::fs::create_dir_all(dir.join("untracked-dir")).unwrap();
        std::fs::write(dir.join("untracked-dir/new.txt"), "new").unwrap();
        std::fs::write(dir.join("untracked.txt"), "new").unwrap();

        discard_all(&dir).await.unwrap();

        assert_eq!(std::fs::read_to_string(dir.join("tracked.txt")).unwrap(), "v1", "tracked 必须恢复");
        assert!(!dir.join("untracked.txt").exists(), "untracked 文件必须删除");
        assert!(!dir.join("untracked-dir").exists(), "untracked 目录必须删除");
        let st = status(&dir).await.unwrap();
        assert!(st.files.is_empty(), "清理后工作树必须干净: {:?}", st.files);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
