//! 小步增量归纳（B 方案）应用侧：spawn 前判定（drain 执行时现判，不在入队时判）、
//! git 取材（commit log / numstat / diff 内容）与增量 prompt 预渲染。
//!
//! 判定全部走纯函数（git 取材除外），每个条件的真/假分支都有单测；
//! 任一项不满足或 git 出错 → 转全量（含 rebase 导致锚点不在历史：
//! rev-list range 报错 → None → 全量，全量成功后锚点自然重写）。
use serde_json::Value;
use std::path::Path;

pub const MAX_COMMITS: i64 = 3;
pub const MAX_FILES: usize = 20;
pub const MAX_LINES: i64 = 2000;
pub const MAX_MAP_BYTES: usize = 512 * 1024;

/// 归纳模式：增量携带 spawn 前取好的全部材料（prompt 占位符预渲染，仿 patrol 先例）
#[derive(Debug, Clone)]
pub enum ReinduceMode {
    Full,
    Incremental(Box<IncrementalContext>),
}

#[derive(Debug, Clone)]
pub struct IncrementalContext {
    /// 决策时读到的 HEAD——diff 上界，也是成功后要写入的锚点（读一次、两用）
    pub head: String,
    /// 上一次的归纳锚点（diff 下界）
    pub prev_anchor: String,
    /// sha/date/subject 每行一条
    pub commit_log: String,
    /// 逐文件增删（文本形态，注入 <DIFF_NUMSTAT>）
    pub diff_numstat: String,
    /// diff 全文（注入 <DIFF_CONTENT>）
    pub diff_content: String,
    /// diff 涉及的路径（含二进制；.easyvibe 已排除）——终态合成时的 affected 依据
    pub diff_paths: Vec<String>,
}

fn git_opt(root: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").args(args).current_dir(root).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn head_sha(root: &Path) -> Option<String> {
    git_opt(root, &["rev-parse", "HEAD"])
}

/// 解析 `git diff --numstat -z` 输出：记录以 \0 分隔、字段以 \t 分隔。
/// - 二进制/submodule/typechange：added/deleted 为 `-`，计为 None（阈值跳过，路径仍计入 affected）
/// - rename：`-z` 下记录为 `adds\tdels\t\0` + 紧跟两个记录 old\0new\0——取 new
/// - 防御：`.easyvibe/**` 显式剔除（用户可能把账簿提交进库，否则阈值膨胀且 prompt 困惑）
pub fn parse_numstat_z(text: &str) -> Vec<(String, Option<(i64, i64)>)> {
    let mut out = Vec::new();
    let mut records = text.split('\0');
    while let Some(rec) = records.next() {
        if rec.is_empty() {
            continue;
        }
        let mut parts = rec.splitn(3, '\t');
        let (a, d) = (parts.next(), parts.next());
        let p = parts.next().unwrap_or("");
        let (Some(a), Some(d)) = (a, d) else { continue };
        if p.is_empty() {
            // rename 形态：下两个记录是 old/new，取 new；numstat 本身在 a/d
            let _old = records.next();
            let new = records.next().unwrap_or("");
            let stats = parse_stats(a, d);
            if !new.is_empty() && !new.starts_with(".easyvibe/") {
                out.push((new.to_string(), stats));
            }
            continue;
        }
        if p.starts_with(".easyvibe/") {
            continue;
        }
        out.push((p.to_string(), parse_stats(a, d)));
    }
    out
}

fn parse_stats(a: &str, d: &str) -> Option<(i64, i64)> {
    match (a.parse::<i64>(), d.parse::<i64>()) {
        (Ok(a), Ok(d)) => Some((a, d)),
        _ => None, // 二进制/submodule/typechange 为 "-\t-"
    }
}

/// 阈值判定：文件数 ≤20 且总行数 ≤2000（二进制条目跳过不计）
fn within_bounds(rows: &[(String, Option<(i64, i64)>)]) -> bool {
    let files = rows.len();
    let lines: i64 = rows.iter().filter_map(|(_, s)| *s).map(|(a, d)| a + d).sum();
    files <= MAX_FILES && lines <= MAX_LINES
}

/// spawn 前判定（drain 到执行时现判）。`old_map` 为 load_map 成功的旧图快照。
/// 全部满足才走增量；任一不满足/git 出错 → Full。纯判定无副作用。
pub fn decide(root: &Path, old_map: Option<&Value>) -> ReinduceMode {
    // ① git 仓库 + HEAD 可读
    let Some(head) = head_sha(root) else { return ReinduceMode::Full };
    // ② 旧图合法（load_map 成功即已过 validate_minimum；调用点保证）
    let Some(map) = old_map else { return ReinduceMode::Full };
    if crate::validate_minimum(map).is_err() {
        return ReinduceMode::Full;
    }
    // ③ 旧图序列化 ≤512KB（大地图喂不动，转全量）
    if serde_json::to_string(map).map(|s| s.len()).unwrap_or(usize::MAX) > MAX_MAP_BYTES {
        return ReinduceMode::Full;
    }
    // ④ 归纳锚点存在且含合法 head_sha（老仓库无此文件 → 全量 → 成功后补锚点，零迁移）
    let Some(state) = crate::synthesis::read_induction_state(root) else {
        return ReinduceMode::Full;
    };
    let range = format!("{}..HEAD", state.head_sha);
    // ⑤ 提交数 ≤3（range 语法；锚点不在历史（rebase）时 rev-list 报错 → None → 全量）
    let Some(count_raw) = git_opt(root, &["rev-list", "--count", &range]) else {
        return ReinduceMode::Full;
    };
    let Ok(count) = count_raw.parse::<i64>() else {
        return ReinduceMode::Full;
    };
    if count > MAX_COMMITS {
        return ReinduceMode::Full;
    }
    // ⑥ 变更阈值（.easyvibe 显式排除在 pathspec 与本解析器两层都做）
    let Some(numstat) = git_opt(root, &["diff", "--numstat", "-z", &range, "--", ".", ":(exclude).easyvibe"]) else {
        return ReinduceMode::Full;
    };
    let rows = parse_numstat_z(&numstat);
    if !within_bounds(&rows) {
        return ReinduceMode::Full;
    }
    // ⑦ 取材：commit log（sha/date/subject 每行一条）+ diff 全文
    let log_range = range.clone();
    let Some(commit_log) = git_opt(root, &["log", "--format=%h%x09%aI%x09%s", &log_range]) else {
        return ReinduceMode::Full;
    };
    let Some(diff_content) = git_opt(root, &["diff", &range, "--", ".", ":(exclude).easyvibe"]) else {
        return ReinduceMode::Full;
    };
    ReinduceMode::Incremental(Box::new(IncrementalContext {
        head,
        prev_anchor: state.head_sha,
        commit_log,
        diff_numstat: rows
            .iter()
            .map(|(p, s)| match s {
                Some((a, d)) => format!("{a}\t{d}\t{p}"),
                None => format!("-\t-\t{p}"),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        diff_content,
        diff_paths: rows.into_iter().map(|(p, _)| p).collect(),
    }))
}

/// 增量 prompt 预渲染：占位符全部在 spawn 前替换（会话层零改动，仿 patrol 预渲染先例）。
/// <REPO_ROOT> 仍留给会话层替换（与全量路径同一纪律）。
pub fn render_incremental_prompt(template: &str, ctx: &IncrementalContext, current_map: &str, schema_path: &str) -> String {
    template
        .replace("<CURRENT_MAP>", current_map)
        .replace("<COMMIT_LOG>", &ctx.commit_log)
        .replace("<DIFF_NUMSTAT>", &ctx.diff_numstat)
        .replace("<DIFF_CONTENT>", &ctx.diff_content)
        .replace("<SCHEMA_PATH>", schema_path)
}

/// 锚点推进裁决（R2 锚点吞提交盲区修复的纯函数核心）：
/// 只有"决策时 HEAD == 当前 HEAD"才把锚点推进到决策 HEAD——
/// 归纳期间用户提交了新 commit 时不推进，下次 range 自然覆盖漏掉的提交。
pub fn should_advance_anchor(decision_head: &Option<String>, current_head: Option<&str>) -> Option<String> {
    match (decision_head, current_head) {
        (Some(d), Some(c)) if d == c => Some(d.clone()),
        _ => None,
    }
}


#[cfg(test)]
mod tests {
    use super::*;

// ---------- numstat -z 解析 ----------

#[test]
fn numstat_z_parses_text_binary_and_easyvibe_exclusion() {
    let text = "12\t3\tsrc/a.rs\0-\t-\tbin/logo.png\05\t5\t.easyvibe/map/map.json\0";
    let rows = parse_numstat_z(text);
    assert_eq!(rows.len(), 2, ".easyvibe 必须显式剔除");
    assert_eq!(rows[0], ("src/a.rs".to_string(), Some((12, 3))));
    assert_eq!(rows[1], ("bin/logo.png".to_string(), None), "二进制跳行计数（None）但路径保留");
}

#[test]
fn numstat_z_parses_rename_taking_new_path() {
    // -z 下 rename：记录体 "adds\tdels\t"（path 空），紧跟 old\0new\0
    let text = "4\t1\t\0src/old.rs\0src/new.rs\0";
    let rows = parse_numstat_z(text);
    assert_eq!(rows, vec![("src/new.rs".to_string(), Some((4, 1)))]);
}

#[test]
fn within_bounds_thresholds() {
    let rows = |n: usize| (0..n).map(|i| (format!("f{i}.rs"), Some((1, 1)))).collect::<Vec<_>>();
    assert!(within_bounds(&rows(20)));
    assert!(!within_bounds(&rows(21)), "21 文件超阈值");
    let many_lines = vec![("big.rs".to_string(), Some((2001, 0)))];
    assert!(!within_bounds(&many_lines), "2001 行超阈值");
    let exactly = vec![("big.rs".to_string(), Some((2000, 0)))];
    assert!(within_bounds(&exactly), "恰好 2000 行合法");
    // 二进制跳过行数计数（文件数配额仍占——阈值是"文件数 AND 行数"双限）
    let mixed = vec![("big.rs".to_string(), Some((1900, 0))), ("bin.png".to_string(), None)];
    assert!(within_bounds(&mixed), "二进制不计行：1900+0 ≤ 2000");
    let bin21 = vec![("bin.png".to_string(), None); 21];
    assert!(!within_bounds(&bin21), "21 个文件超文件数配额（二进制也不例外）");
}

// ---------- 判定（真 git 临时仓库，仿 freshness/git.rs 测试写法） ----------

struct TempRepo {
    dir: std::path::PathBuf,
}
impl TempRepo {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&dir).status().unwrap().success());
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        Self { dir }
    }
    fn commit(&self, msg: &str) -> String {
        std::fs::write(self.dir.join("a.txt"), format!("{msg}\n")).unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&self.dir).status().unwrap().success());
        git(&["add", "."]);
        git(&["commit", "-q", "-m", msg]);
        head_sha(&self.dir).unwrap()
    }
}
impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn test_map() -> Value {
    // 与 easyvibe-map::synthesis::test_map 同形的本地副本（跨 crate 不可见 cfg(test) 项）
    serde_json::json!({
        "version": "1.0",
        "meta": {"generated_at": "2026-01-01T00:00:00Z"},
        "layers": [{"id": "foundation", "order": 0}],
        "modules": [
            {"id": "core", "layer": "foundation", "files": ["src/core/**/*.rs"],
             "dependencies": [], "health": {"coupling": "low", "complexity": "low", "churn": "low"}}
        ],
        "edges": [],
        "health": {}
    })
}

fn write_anchor(root: &Path, sha: &str) {
    crate::synthesis::write_induction_state(
        root,
        &crate::synthesis::InductionState { head_sha: sha.into(), mode: "full".into(), completed_at: "2026-01-01T00:00:00Z".into() },
    )
    .unwrap();
}

#[test]
fn decide_gates_each_condition() {
    let repo = TempRepo::new("ev-decide-gates");
    let sha1 = repo.commit("init");
    let map = test_map();

    // 非 git → Full（用无 git 目录判）
    let nogit = std::env::temp_dir().join(format!("ev-decide-nogit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&nogit);
    std::fs::create_dir_all(&nogit).unwrap();
    assert!(matches!(decide(&nogit, Some(&map)), ReinduceMode::Full));

    // 无旧图 → Full
    assert!(matches!(decide(&repo.dir, None), ReinduceMode::Full));
    // 无锚点文件 → Full
    assert!(matches!(decide(&repo.dir, Some(&map)), ReinduceMode::Full));
    // 锚点合法 + 0 个新提交 → 增量（取齐材料）
    write_anchor(&repo.dir, &sha1);
    let mode = decide(&repo.dir, Some(&map));
    let ReinduceMode::Incremental(ctx) = mode else { panic!("合法条件齐备必须是增量") };
    assert_eq!(ctx.head, sha1);
    assert_eq!(ctx.prev_anchor, sha1);
    assert!(ctx.diff_paths.is_empty());

    // rebase 盲区：锚点不在历史 → rev-list 报错 → Full
    write_anchor(&repo.dir, "deadbeefdeadbeef");
    assert!(matches!(decide(&repo.dir, Some(&map)), ReinduceMode::Full));

    // 提交数超阈值：3 个新提交仍增量，第 4 个 → Full
    write_anchor(&repo.dir, &sha1);
    repo.commit("c2");
    repo.commit("c3");
    assert!(matches!(decide(&repo.dir, Some(&map)), ReinduceMode::Incremental(_)), "2 个新提交仍增量");
    repo.commit("c4");
    assert!(matches!(decide(&repo.dir, Some(&map)), ReinduceMode::Incremental(_)), "3 个新提交恰在阈值内");
    repo.commit("c5");
    assert!(matches!(decide(&repo.dir, Some(&map)), ReinduceMode::Full), "第 4 个新提交超阈值");
}

#[test]
fn decide_respects_line_and_file_bounds() {
    let repo = TempRepo::new("ev-decide-bounds");
    let sha1 = repo.commit("init");
    let map = test_map();
    write_anchor(&repo.dir, &sha1);
    // 2001 行单文件改动 → Full
    std::fs::write(repo.dir.join("big.txt"), "x\n".repeat(2001)).unwrap();
    let git = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&repo.dir).status().unwrap().success());
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "big"]);
    assert!(matches!(decide(&repo.dir, Some(&map)), ReinduceMode::Full), "超 2000 行必须转全量");
}

#[test]
fn decide_excludes_easyvibe_even_when_committed() {
    let repo = TempRepo::new("ev-decide-bookkeeping");
    let sha1 = repo.commit("init");
    let map = test_map();
    write_anchor(&repo.dir, &sha1);
    // 用户把账簿提交进库：阈值不得膨胀
    std::fs::create_dir_all(repo.dir.join(".easyvibe/map")).unwrap();
    std::fs::write(repo.dir.join(".easyvibe/map/map.json"), "x\n".repeat(5000)).unwrap();
    std::fs::write(repo.dir.join("real.rs"), "fn main() {}\n").unwrap();
    let git = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&repo.dir).status().unwrap().success());
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "with bookkeeping"]);
    let mode = decide(&repo.dir, Some(&map));
    let ReinduceMode::Incremental(ctx) = mode else { panic!("账簿不得撑爆阈值") };
    assert!(ctx.diff_paths.iter().all(|p| !p.starts_with(".easyvibe/")), ".easyvibe 必须被排除: {:?}", ctx.diff_paths);
    assert_eq!(ctx.diff_paths, vec!["real.rs".to_string()]);
}

// ---------- prompt 预渲染 ----------

#[test]
fn render_incremental_prompt_fills_placeholders() {
    let ctx = IncrementalContext {
        head: "h".into(),
        prev_anchor: "a".into(),
        commit_log: "abc1234\t2026-10-01T00:00:00+08:00\tfeat: x".into(),
        diff_numstat: "1\t1\tsrc/a.rs".into(),
        diff_content: "diff --git a/src/a.rs b/src/a.rs".into(),
        diff_paths: vec!["src/a.rs".into()],
    };
    let out = render_incremental_prompt(
        "<CURRENT_MAP>\n<COMMIT_LOG>\n<DIFF_NUMSTAT>\n<DIFF_CONTENT>\n<SCHEMA_PATH>\n<REPO_ROOT>",
        &ctx,
        "{\"map\": true}",
        "/schema.json",
    );
    assert!(out.contains("{\"map\": true}"));
    assert!(out.contains("abc1234\t2026-10-01"));
    assert!(out.contains("1\t1\tsrc/a.rs"));
    assert!(out.contains("diff --git a/src/a.rs"));
    assert!(out.contains("/schema.json"));
    assert!(out.contains("<REPO_ROOT>"), "REPO_ROOT 留给会话层替换");
}

// ---------- 锚点推进裁决（R2 测试锁死） ----------

#[test]
fn should_advance_anchor_locks_head_drift_rule() {
    // 常规：决策 HEAD == 当前 HEAD → 推进
    assert_eq!(should_advance_anchor(&Some("sha1".into()), Some("sha1")), Some("sha1".into()));
    // R2：归纳期间用户提交了 → 不推进（下次 range 覆盖漏掉的提交）
    assert_eq!(should_advance_anchor(&Some("sha1".into()), Some("sha2")), None, "HEAD 漂移不得推进锚点");
    // 决策时非 git / 当前读不到 HEAD → 不推进
    assert_eq!(should_advance_anchor(&None, Some("sha1")), None);
    assert_eq!(should_advance_anchor(&Some("sha1".into()), None), None);
}
}
