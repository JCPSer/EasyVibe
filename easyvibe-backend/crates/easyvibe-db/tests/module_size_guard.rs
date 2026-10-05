//! R12 防回胀守卫（"数据持久 god file 不再复现"的机制保证）。
//!
//! 与 `easyvibe-app/tests/module_size_guard.rs` 同范式：只读源码文本，不 import 符号。
//! 断言组：
//!   1. lib.rs ≤ 250 行，各子模块 ≤ 600 行——god file 复发即失败；
//!   2. src/*.rs 子模块文件集快照（9 个，双向全等）——防新增域文件绕过归位；
//!   3. 迁移清单快照（16 个，双向全等）——少一个=历史丢失，多一个=意外新增；
//!   4. 单元测试数快照（11 个，双向全等）——防迁移时丢测试（尤其两个 P0 安全网）；
//!   5. 域文件禁止横向引用兄弟域（域 → core 允许；域 → 域 禁止）。
//!
//! 阈值口径见方案 §R12：lib.rs 目标 ≈120（250 留 2x 余量）；子模块沿用 app crate
//! 「职责域 ≤600」口径（最大 agent.rs ≈455，留余量）。

use std::path::PathBuf;

fn db_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn read(rel: &str) -> String {
    let p = db_src().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读取 {rel} 失败: {e}"))
}

fn loc(rel: &str) -> usize {
    read(rel).lines().count()
}

/// 全部子模块文件（含 core）。
const MODULES: [&str; 9] = [
    "core", "health", "settings", "task", "approval", "conversation", "event", "agent",
    "session_output",
];

/// 聚合域文件（不含 core）：横向引用检查的対象，域 → 域 一律禁止。
const DOMAIN_FILES: [&str; 8] = [
    "health", "settings", "task", "approval", "conversation", "event", "agent",
    "session_output",
];

/// 16 个迁移文件名（逐字冻结；不含 .sql 后缀，便于排序比较）。
const FROZEN_MIGRATIONS: [&str; 16] = [
    "0001_health_history",
    "0002_settings",
    "0003_tasks",
    "0004_task_session",
    "0005_approvals",
    "0006_conversations_tokens",
    "0007_task_result",
    "0008_task_base_head",
    "0009_conversations_multi",
    "0010_events",
    "0011_task_lineage",
    "0012_task_updated_at_ms",
    "0013_agent_sessions",
    "0014_agent_session_module",
    "0015_session_outputs",
    "0016_patrol_concerns_diff",
];

#[test]
fn god_files_stay_below_size_limits() {
    let n = loc("lib.rs");
    assert!(n <= 250, "lib.rs 超 250 行（god file 复发）: {n}");
    for f in MODULES {
        let n = loc(&format!("{f}.rs"));
        assert!(n <= 600, "{f}.rs 超 600 行: {n}");
    }
}

#[test]
fn submodule_file_set_is_frozen() {
    let dir = db_src();
    let mut actual: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            if p.extension().map(|x| x == "rs").unwrap_or(false) {
                let name = p.file_name().unwrap().to_string_lossy().to_string();
                if name == "lib.rs" {
                    None
                } else {
                    Some(name.trim_end_matches(".rs").to_string())
                }
            } else {
                None
            }
        })
        .collect();
    actual.sort();
    let mut expected: Vec<String> = MODULES.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(actual, expected, "src 子模块文件集漂移（新增域文件须归位到既有子模块）");
}

#[test]
fn migrations_snapshot_is_frozen() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut actual: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            if p.extension().map(|x| x == "sql").unwrap_or(false) {
                Some(p.file_stem().unwrap().to_string_lossy().to_string())
            } else {
                None
            }
        })
        .collect();
    actual.sort();
    let mut expected: Vec<String> = FROZEN_MIGRATIONS.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(actual, expected, "迁移清单漂移：少一个=历史丢失，多一个=意外新增");
}

#[test]
fn test_count_is_frozen() {
    let total: usize = MODULES
        .iter()
        .map(|f| read(&format!("{f}.rs")).matches("#[tokio::test]").count())
        .sum();
    assert_eq!(total, 11, "单元测试数漂移（迁移时可能丢测试，尤其两个 P0 安全网）: {total}");
}

#[test]
fn domain_modules_have_no_horizontal_deps() {
    for f in DOMAIN_FILES {
        let txt = read(&format!("{f}.rs"));
        for other in DOMAIN_FILES {
            if other == f {
                continue;
            }
            let needle = format!("use crate::{other}");
            assert!(
                !txt.contains(&needle),
                "{f}.rs 出现横向引用 `{needle}`（域 → 域 禁止；域 → core 才允许）"
            );
        }
    }
}
