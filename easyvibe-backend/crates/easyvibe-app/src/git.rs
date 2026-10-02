//! M4-4 Git 工作树：状态/历史查询 + 写操作（提交/拉取/推送/撤销）。
//! 命令统一走 tokio::process + 超时；解析逻辑全部是纯函数，单测覆盖（含真 git 集成测试）。
//! 安全：写操作只接受相对路径且禁止 `..` 越界；discard 按跟踪状态区分 checkout/clean。
use easyvibe_common::ApiError;
use std::path::Path;
use std::time::Duration;

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

// ---------- 写操作 ----------

pub async fn commit_all(repo: &Path, message: &str) -> Result<String, ApiError> {
    let message = message.trim();
    if message.is_empty() {
        return Err(ApiError::BadRequest("提交说明不能为空".into()));
    }
    git(repo, &["add", "-A"]).await?;
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

/// 路径安全：仅相对路径、禁止 `..` 分量（写操作统一入口）
fn validate_rel_path(path: &str) -> Result<(), ApiError> {
    let p = Path::new(path);
    if p.is_absolute() || path.split('/').any(|s| s == "..") || path.is_empty() {
        return Err(ApiError::BadRequest(format!("非法路径: {path}")));
    }
    Ok(())
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
    async fn non_git_repo_errs() {
        let dir = std::env::temp_dir().join("ev-not-git-status");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(status(&dir).await.is_err());
        assert!(log(&dir, 10).await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
