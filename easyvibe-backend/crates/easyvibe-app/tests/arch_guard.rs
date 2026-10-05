//! R8 防回归守卫：task-engine 源码不得再出现指向 server-api 符号的 `crate::` 反向引用。
//!
//! 与 `.easyvibe/map/scan_easyvibe.py` 的模块表（`MODS['task-engine']`）同源——扫描器的
//! `task-engine -> server-api` 边由 `task_exec.rs` 中的 `crate::BusEvent` 字面量触发；
//! 本测试以源码事实守卫同一约束（权威口径，先于启发式）。
//! 违反即失败：后续在 task_exec.rs 写回 `crate::BusEvent` 即让 server-api ↔ task-engine 环复发。

use std::path::PathBuf;

/// task-engine 模块文件集合（与 scan_easyvibe.py 的 MODS['task-engine'] 保持一致）：
/// 主文件 + task_exec/ 子模块（拆分为 prompt/review/harness/changes/contract 后必须全覆盖，
/// 否则子模块里的 crate:: 反向引用会被漏报）。
fn task_engine_files() -> Vec<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut v = vec!["src/task_exec.rs".to_string()];
    let dir = root.join("src/task_exec");
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "rs").unwrap_or(false) {
                v.push(format!("src/task_exec/{}", p.file_name().unwrap().to_string_lossy()));
            }
        }
    }
    v.sort();
    v
}

/// 逆向引用禁止子串（server-api / 同 crate 应用层符号）
const FORBIDDEN: &[&str] = &[
    "crate::BusEvent",
    "crate::publish",
    "crate::agent_conf",
    "crate::AppState",
    "crate::AppError",
    "crate::service::map::start_",
    "crate::service::map::analyze_submap_inner",
    "crate::session_queue",
];

#[test]
fn task_engine_has_no_reverse_crate_refs() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for f in task_engine_files() {
        let f = f.as_str();
        let path = root.join(f);
        let txt = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读取 {f} 失败: {e}"));
        for bad in FORBIDDEN {
            assert!(
                !txt.contains(bad),
                "{f} 出现逆向引用 `{bad}`——server-api ↔ task-engine 环复发（R8 守卫）"
            );
        }
    }
}
