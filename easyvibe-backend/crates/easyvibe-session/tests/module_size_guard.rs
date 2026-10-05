//! 防回胀守卫（R14，session；范式照抄 easyvibe-db/tests/module_size_guard.rs 的 R12）：
//! 只读源码文本，不 import 被守卫符号。
//!
//! 断言组：
//!   1. lib.rs ≤ 250 行，各子模块 ≤ 600 行——god file 复发即失败；
//!   2. src/*.rs 子模块文件集快照（4 个，双向全等）——防新增域文件绕过归位；
//!   3. 单元测试数快照（13 个）——防拆分时丢测试；
//!   4. 子模块禁止横向引用兄弟子模块（→ lib.rs 门面 允许）。
//!
//! 阈值口径：lib.rs 是门面（SessionManager 结构 + 构造器 + re-export），目标 ≈89
//! （250 留近 3x 余量）；子模块沿用全仓「职责域 ≤600」口径（拆后最大 lifecycle.rs ≈449）。
//! LOC 口径：`lines().count()`（与 `wc -l` 一致；无尾换行时相差 1，勿与前端 `split('\n')` 混淆）。
//! 横向引用口径：四路子串 `use crate::x::` / `use crate::x;` / 完全限定 `crate::x::` /
//! 别名 `crate::x as `（R14 返工加固——旧两路可被完全限定路径与 `as` 别名绕过）。

use std::path::PathBuf;

fn session_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn read(rel: &str) -> String {
    let p = session_src().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读取 {rel} 失败: {e}"))
}

fn loc(rel: &str) -> usize {
    read(rel).lines().count()
}

/// 全部子模块文件（不含 lib.rs 门面）。
const MODULES: [&str; 4] = ["types", "output", "pty", "lifecycle"];

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
    let dir = session_src();
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
fn test_count_is_frozen() {
    let total: usize = MODULES
        .iter()
        .map(|f| {
            let txt = read(&format!("{f}.rs"));
            txt.matches("#[test]").count() + txt.matches("#[tokio::test]").count()
        })
        .sum();
    assert_eq!(total, 13, "单元测试数漂移（拆分时可能丢测试）: {total}");
}

#[test]
fn submodules_have_no_horizontal_deps() {
    for f in MODULES {
        let txt = read(&format!("{f}.rs"));
        for other in MODULES {
            if other == f {
                continue;
            }
            for needle in [
                format!("use crate::{other}::"),
                format!("use crate::{other};"),
                format!("crate::{other}::"),
                format!("crate::{other} as "),
            ] {
                assert!(
                    !txt.contains(&needle),
                    "{f}.rs 出现横向引用 `{needle}`（子模块 → 兄弟子模块 禁止；→ lib.rs 门面 允许）"
                );
            }
        }
    }
}
