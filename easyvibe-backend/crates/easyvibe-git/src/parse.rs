//! git 输出解析（纯函数）。

use crate::model::{GitFile, GitLogRow};
use easyvibe_common::ApiError;
use std::path::Path;

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

/// 路径安全：仅相对路径、禁止 `..` 分量（写操作统一入口）
pub fn validate_rel_path(path: &str) -> Result<(), ApiError> {
    let p = Path::new(path);
    if p.is_absolute() || path.split('/').any(|s| s == "..") || path.is_empty() {
        return Err(ApiError::BadRequest(format!("非法路径: {path}")));
    }
    Ok(())
}
