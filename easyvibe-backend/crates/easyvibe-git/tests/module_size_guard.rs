//! R10 防回胀 / 方向守卫（easyvibe-git）：文件集快照 + 单文件 LOC + 禁用依赖面 + 导出符号冻结。
//!
//! 与 `easyvibe-app/tests/module_size_guard.rs` 同范式：只读源码文本，不 import 符号。
//! 目的：新领域 crate 不得反向长回 HTTP 边界 / 应用状态 / 持久层 / agent 面。

use std::path::PathBuf;

fn src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rs_files() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(src())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "rs").unwrap_or(false))
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    v.sort();
    v
}

fn read(f: &str) -> String {
    std::fs::read_to_string(src().join(f)).unwrap_or_else(|e| panic!("读取 {f} 失败: {e}"))
}

/// 归属快照（文件集迁移 = 需显式改本表并说明理由）。
const FROZEN_FILES: &[&str] = &["exec.rs", "lib.rs", "model.rs", "ops.rs", "parse.rs", "tests.rs"];

/// 禁止出现的依赖面（边界层符号回流即失败）。
const FORBIDDEN: &[&str] = &[
    "axum",
    "AppState",
    "easyvibe_db",
    "easyvibe_ai_agent",
    "easyvibe_app",
    "easyvibe_map::",
    "crate::service",
];

/// 导出符号冻结（防改名漂移造成调用方静默断链）。
const FROZEN_SYMBOLS: &[&str] = &[
    "pub struct GitFile",
    "pub struct GitStatus",
    "pub struct GitLogRow",
    "pub struct CommitFileStat",
    "pub struct CommitDetail",
    "pub async fn git(",
    "pub fn git_opt(",
    "pub const GIT_TIMEOUT",
    "pub async fn status(",
    "pub async fn log(",
    "pub async fn show_commit(",
    "pub async fn commit_all(",
    "pub async fn pull(",
    "pub async fn push(",
    "pub async fn discard(",
    "pub async fn discard_all(",
    "pub async fn diff(",
    "pub const DIFF_MAX_LINES",
    "pub struct FileDiff",
    "pub fn validate_rel_path(",
    "pub fn parse_porcelain(",
    "pub fn parse_numstat(",
    "pub fn parse_log(",
];

#[test]
fn file_set_is_frozen() {
    let mut expected: Vec<String> = FROZEN_FILES.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(rs_files(), expected, "easyvibe-git 文件集漂移——须登记快照（R10）");
}

#[test]
fn files_stay_below_size_limits() {
    for f in rs_files() {
        let n = read(&f).lines().count();
        let lim = if f == "tests.rs" { 700 } else { 300 };
        assert!(n <= lim, "easyvibe-git/src/{f} 超 {lim} 行: {n}");
    }
}

#[test]
fn no_boundary_dependencies() {
    for f in rs_files() {
        let txt = read(&f);
        for bad in FORBIDDEN {
            assert!(!txt.contains(bad), "easyvibe-git/src/{f} 出现禁用依赖面 `{bad}`——领域 crate 不得回流边界/状态（R10）");
        }
    }
}

#[test]
fn exported_symbols_are_frozen() {
    let mut all = String::new();
    for f in rs_files() {
        all.push_str(&read(&f));
        all.push('\n');
    }
    for s in FROZEN_SYMBOLS {
        assert!(all.contains(s), "easyvibe-git 导出符号 `{s}` 缺失——改名/删除须同步冻结表与调用方（R10）");
    }
}
