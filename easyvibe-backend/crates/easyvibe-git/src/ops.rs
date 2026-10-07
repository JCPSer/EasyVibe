//! git 领域操作：查询（status/log/show_commit）与写操作（commit/pull/push/discard）。

use crate::exec::{git, git_opt};
use crate::model::{CommitDetail, CommitFileStat, FileDiff, GitLogRow, GitStatus};
use crate::parse::{parse_log, parse_numstat, parse_porcelain, validate_rel_path};
use easyvibe_common::ApiError;
use std::path::Path;

/// 差异展示截断保护：统一 diff 超过该值即截断（前端提示"差异过大"），防止超大 diff 撑爆响应与渲染。
pub const DIFF_MAX_LINES: usize = 2000;
/// 未跟踪文件整文件读取上限（防御超大新文件）。
const DIFF_MAX_BYTES: u64 = 1024 * 1024;

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

pub async fn commit_all(repo: &Path, message: &str) -> Result<String, ApiError> {
    let message = message.trim();
    if message.is_empty() {
        return Err(ApiError::BadRequest("提交说明不能为空".into()));
    }
    // R3 C5：产品账簿（地图/归档/会话产物）不进用户提交历史——
    // ① .easyvibe/ 含巡检写回的 map.json，add -A 提交会立刻触发 freshness 自反漂移（越健康越亮警告）
    // ② .claude/ 是 agent STAR 归档，与 .easyvibe/development_docs/ 是同一留痕的两份副本
    // 排除不能用 :(exclude) pathspec 直接点名：被 .gitignore 忽略的目录会触发
    // "The following paths are ignored" 直接失败——改为"全量加（被忽略的目录 git 天然跳过）
    // → 再把账簿目录的暂存撤下"两步，两种口径下行为一致
    git(repo, &["add", "-A", "--", "."]).await?;
    let staged_bookkeeping =
        git(repo, &["diff", "--cached", "--name-only", "-z", "--", ".easyvibe", ".claude"]).await.unwrap_or_default();
    if !staged_bookkeeping.is_empty() {
        // reset 只动暂存区，不碰工作区——账簿文件留在磁盘，改动也不会进本次提交
        git(repo, &["reset", "-q", "--", ".easyvibe", ".claude"]).await?;
    }
    git(repo, &["commit", "-m", message]).await?;
    git(repo, &["rev-parse", "--short", "HEAD"]).await.map(|s| s.trim().to_string())
}

pub async fn pull(repo: &Path) -> Result<(), ApiError> {
    git(repo, &["pull", "--rebase", "--autostash"]).await.map(|_| ())
}

pub async fn push(repo: &Path) -> Result<(), ApiError> {
    git(repo, &["push"]).await.map(|_| ())
}

/// 单文件统一差异（Git 页 diff 抽屉）。
///
/// - `staged=false`：worktree vs index（未暂存改动）；`staged=true`：index vs HEAD。
/// - 未跟踪文件 git diff 天然为空 → 合成「整文件 = 新增」的统一 diff。
/// - 二进制文件只置 `binary` 标记，不返回内容；超行数截断置 `truncated`。
/// - 路径校验沿用 validate_rel_path，并额外拒绝 `-` 开头（双重防注入；命令本就经 `--` 分隔）。
pub async fn diff(repo: &Path, path: &str, staged: bool) -> Result<FileDiff, ApiError> {
    validate_rel_path(path)?;
    if path.starts_with('-') {
        return Err(ApiError::BadRequest(format!("非法路径: {path}")));
    }

    let mut args: Vec<&str> = vec!["diff"];
    if staged {
        args.push("--cached");
    }
    args.push("--");
    args.push(path);
    let out = git(repo, &args).await?;

    // git 对 pathspec 不匹配的路径静默返回空 diff——先确认文件真实存在，区分"无差异"与"文件不存在"
    let tracked = git_opt(repo, &["ls-files", "--error-unmatch", "--", path]).is_some();
    if !tracked && !repo.join(path).is_file() {
        return Err(ApiError::NotFound(format!("文件 {path} 不存在")));
    }

    if is_binary_diff(&out) {
        return Ok(FileDiff { path: path.into(), untracked: false, binary: true, empty: false, truncated: false, total_lines: 0, text: String::new() });
    }
    if !out.is_empty() {
        return Ok(build_file_diff(path, out));
    }
    if staged || tracked {
        // 范围内无差异（已暂存文件的 unstaged 差异即此形态）
        return Ok(FileDiff { path: path.into(), untracked: false, binary: false, empty: true, truncated: false, total_lines: 0, text: String::new() });
    }
    // 未跟踪新文件：合成整文件新增 diff（与 git add -N 后的 diff 同构）
    let content = read_capped(&repo.join(path))?;
    if content.iter().take(8000).any(|b| *b == 0) {
        return Ok(FileDiff { path: path.into(), untracked: true, binary: true, empty: false, truncated: false, total_lines: 0, text: String::new() });
    }
    let text = String::from_utf8_lossy(&content);
    let lines: Vec<&str> = text.lines().collect();
    let (body, truncated) = truncate_lines(&lines, DIFF_MAX_LINES);
    let mut synth = format!("--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n", lines.len());
    for l in body {
        synth.push('+');
        synth.push_str(l);
        synth.push('\n');
    }
    Ok(FileDiff { path: path.into(), untracked: true, binary: false, empty: false, truncated, total_lines: (lines.len() + 3) as i64, text: synth })
}

/// `git diff` 对二进制输出形如 "Binary files a/x and b/x differ"
fn is_binary_diff(out: &str) -> bool {
    out.lines().any(|l| l.starts_with("Binary files ") && l.contains(" differ"))
}

/// 截断到前 max 行，返回 (截取段, 是否截断)
fn truncate_lines<'a>(lines: &'a [&'a str], max: usize) -> (&'a [&'a str], bool) {
    if lines.len() > max {
        (&lines[..max], true)
    } else {
        (lines, false)
    }
}

/// 组装 FileDiff：统一 diff 文本 + 行数截断保护
fn build_file_diff(path: &str, out: String) -> FileDiff {
    let lines: Vec<&str> = out.lines().collect();
    let total = lines.len();
    let (seg, truncated) = truncate_lines(&lines, DIFF_MAX_LINES);
    let mut text = String::new();
    for l in seg {
        text.push_str(l);
        text.push('\n');
    }
    FileDiff { path: path.into(), untracked: false, binary: false, empty: false, truncated, total_lines: total as i64, text }
}

fn read_capped(p: &Path) -> Result<Vec<u8>, ApiError> {
    use std::io::Read as _;
    let f = std::fs::File::open(p).map_err(|e| ApiError::Internal(format!("读取文件失败: {e}")))?;
    let mut r = std::io::BufReader::new(f.take(DIFF_MAX_BYTES));
    let mut buf = Vec::new();
    r.read_to_end(&mut buf).map_err(|e| ApiError::Internal(format!("读取文件失败: {e}")))?;
    Ok(buf)
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
