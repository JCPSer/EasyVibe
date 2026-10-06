//! git 领域操作：查询（status/log/show_commit）与写操作（commit/pull/push/discard）。

use crate::exec::{git, git_opt};
use crate::model::{CommitDetail, CommitFileStat, GitLogRow, GitStatus};
use crate::parse::{parse_log, parse_numstat, parse_porcelain, validate_rel_path};
use easyvibe_common::ApiError;
use std::path::Path;

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
