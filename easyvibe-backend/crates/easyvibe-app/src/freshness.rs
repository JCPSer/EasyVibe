//! S2 地图保鲜：freshness 判定（§13.4——地图可信度是全部下游功能的地基）。
//! 口径：地图 meta.generated_at vs 仓库 git 最新提交——git 有比地图新的提交即"漂移"，
//! 漂移量按天分级（fresh / drifting / stale）；非 git 仓库或地图无时间戳 → unknown（不告警，避免误报）。
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FreshnessStatus {
    Fresh,
    Drifting,
    Stale,
    Unknown,
}

impl FreshnessStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            FreshnessStatus::Fresh => "fresh",
            FreshnessStatus::Drifting => "drifting",
            FreshnessStatus::Stale => "stale",
            FreshnessStatus::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Freshness {
    pub status: FreshnessStatus,
    pub map_generated_at: Option<String>,
    pub latest_commit_at: Option<i64>,
    /// 地图生成之后的提交数（git rev-list --count --since=<map_ts>；拿不到为 None）
    pub commits_since_map: Option<i64>,
}

/// drifting/stale 分界（天）：漂移小于 3 天 = drifting（仍可参考），超过 = stale（建议重归纳）
const STALE_DAYS: i64 = 3;

pub fn assess(repo_root: &Path, map: &Value) -> Freshness {
    let map_ts = map["meta"]["generated_at"].as_str().and_then(parse_ts_like);
    let map_generated_at = map["meta"]["generated_at"].as_str().map(Into::into);
    let latest_commit_at = git_latest_commit_ts(repo_root);
    let (Some(map_ts), Some(latest)) = (map_ts, latest_commit_at) else {
        return Freshness { status: FreshnessStatus::Unknown, map_generated_at, latest_commit_at, commits_since_map: None };
    };
    let commits_since_map = map_generated_at
        .as_deref()
        .and_then(|raw| git_commits_since(repo_root, raw));
    // 口径（§13.4）：git 有比地图新的提交即漂移。时间差只做 drifting/stale 分级，
    // commits_since_map > 0 必须至少 drifting——免疫时钟/时区误差（2026-10-05 实弹修正）。
    let has_new_commits = commits_since_map.map(|n| n > 0).unwrap_or(latest > map_ts);
    let status = if !has_new_commits && latest <= map_ts {
        FreshnessStatus::Fresh
    } else if latest - map_ts < STALE_DAYS * 86_400 {
        FreshnessStatus::Drifting
    } else {
        FreshnessStatus::Stale
    };
    Freshness { status, map_generated_at, latest_commit_at, commits_since_map }
}

fn git_output(repo_root: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).current_dir(repo_root).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn git_latest_commit_ts(repo_root: &Path) -> Option<i64> {
    git_output(repo_root, &["log", "-1", "--format=%ct"]).and_then(|s| s.parse().ok())
}

fn git_commits_since(repo_root: &Path, since_rfc: &str) -> Option<i64> {
    git_output(repo_root, &["rev-list", "--count", "HEAD", &format!("--since={since_rfc}")]).and_then(|s| s.parse().ok())
}

/// 简易时间戳解析：接受 "YYYY-MM-DDTHH:MM:SS" 带时区后缀（Z / ±HH:MM）。
/// 实现已上移至 easyvibe-common（freshness 与 easyvibe-map::synthesis 共用），此处 re-export 保持调用点不变。
pub use easyvibe_common::parse_ts_like;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ts_like_handles_rfc3339_and_garbage() {
        // 时区必须参与换算：+08:00 比 Z 晚 8 小时（不换算会把本地地图时间当 UTC，漂移被误判 fresh）
        assert_eq!(
            parse_ts_like("2026-09-29T15:30:08+08:00"),
            Some(parse_ts_like("2026-09-29T15:30:08Z").unwrap() - 8 * 3600)
        );
        assert_eq!(parse_ts_like("2026-09-29T15:30:08+0800"), Some(parse_ts_like("2026-09-29T15:30:08Z").unwrap() - 8 * 3600), "无冒号形态也要认");
        assert_eq!(parse_ts_like("2026-09-29T15:30:08-05:00"), Some(parse_ts_like("2026-09-29T15:30:08Z").unwrap() + 5 * 3600), "负偏移");
        assert!(parse_ts_like("2026-09-29T15:30:08Z").unwrap() > 0);
        assert!(parse_ts_like("not-a-date").is_none());
        assert!(parse_ts_like("2026-09-29").is_none(), "日级精度不够，必须拒绝（避免误判 fresh）");
    }

    #[test]
    fn commits_after_map_force_at_least_drifting() {
        // 实弹（2026-10-05）：地图生成后又有提交（commits_since_map=3）就必须至少 drifting，
        // 即使时钟差落在 fresh 侧—— immunity against tz/clock 误差。
        // 提交日期必须显式摆布：rev-list --since 按提交日期过滤，"现在"生成的提交彼此同秒，
        // 不摆布的话 --since 落在未来会数出 0（本测试第一版就踩了这个坑）。
        let dir = std::env::temp_dir().join("ev-freshness-commits");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let commit_at = |msg: &str, iso: &str| {
            std::fs::write(dir.join("a.txt"), msg).unwrap();
            std::process::Command::new("git").args(["add", "."]).current_dir(&dir).output().unwrap();
            std::process::Command::new("git")
                .args(["commit", "-q", "-m", msg])
                .env("GIT_AUTHOR_DATE", iso)
                .env("GIT_COMMITTER_DATE", iso)
                .current_dir(&dir)
                .output()
                .unwrap();
        };
        std::process::Command::new("git").args(["init", "-q"]).current_dir(&dir).output().unwrap();
        std::process::Command::new("git").args(["config", "user.email", "t@t"]).current_dir(&dir).output().unwrap();
        std::process::Command::new("git").args(["config", "user.name", "t"]).current_dir(&dir).output().unwrap();
        commit_at("before-map", "2026-10-01T00:00:00Z");
        let map_iso = "2026-10-01T01:00:00Z"; // 地图生成于第 1 与第 2 个提交之间
        commit_at("c2", "2026-10-01T02:00:00Z");
        commit_at("c3", "2026-10-01T03:00:00Z");
        commit_at("c4", "2026-10-01T04:00:00Z");
        let map = serde_json::json!({"meta": {"generated_at": map_iso}});
        let f = assess(&dir, &map);
        assert!(f.status != FreshnessStatus::Fresh, "地图后有 3 个提交不得判 fresh: {f:?}");
        assert_eq!(f.commits_since_map, Some(3));
    }

    #[test]
    fn status_grades_by_drift_days() {
        let dir = std::env::temp_dir().join("ev-freshness-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(&dir).output().unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(dir.join("a.txt"), "1").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);
        // 手动把提交时间改到 5 天前（GIT_COMMITTER_DATE 作用于下一次提交；这里直接 rev-parse 当前即可——
        // 用 env 变量重做一次提交更稳）
        std::fs::write(dir.join("a.txt"), "2").unwrap();
        git(&["add", "."]);
        let mut cmd = std::process::Command::new("git");
        cmd.args(["commit", "-q", "-m", "old"])
            .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
            .current_dir(&dir);
        cmd.output().unwrap();

        let map_old = serde_json::json!({"meta": {"generated_at": "2025-12-20T00:00:00Z"}}); // 地图比最新提交老 12 天 → stale
        let f = assess(&dir, &map_old);
        assert_eq!(f.status, FreshnessStatus::Stale, "漂移 12 天应 stale: {f:?}");

        let map_mid = serde_json::json!({"meta": {"generated_at": "2026-01-01T12:00:00Z"}}); // 地图晚于提交 → fresh
        let f2 = assess(&dir, &map_mid);
        assert_eq!(f2.status, FreshnessStatus::Fresh, "地图晚于最新提交应 fresh: {f2:?}");
        assert_eq!(f2.commits_since_map, Some(0), "地图之后的提交数应为 0");

        let map_none = serde_json::json!({"meta": {}});
        let f3 = assess(&dir, &map_none);
        assert_eq!(f3.status, FreshnessStatus::Unknown, "地图无时间戳 → unknown 不误报");
    }

    #[test]
    fn non_git_repo_is_unknown_not_stale() {
        let dir = std::env::temp_dir().join("ev-freshness-nogit");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let map = serde_json::json!({"meta": {"generated_at": "2020-01-01T00:00:00Z"}});
        let f = assess(&dir, &map);
        assert_eq!(f.status, FreshnessStatus::Unknown, "非 git 仓库不得判 stale（避免误报骚扰）");
    }
}
