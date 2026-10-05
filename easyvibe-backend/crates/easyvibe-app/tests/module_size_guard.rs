//! R7 防回胀守卫（"god file 不再复现"的机制保证）。
//!
//! 与 `tests/arch_guard.rs` 同范式：只读源码文本，不 import 符号（本 crate 为 bin-only）。
//! 断言组：
//!   1. 主文件 / 子模块 LOC 上限——god file 复发即失败；
//!   2. routes 兄弟域禁止横向 `use crate::routes::<other>`；
//!   3. task_exec 子模块禁止反向引用状态机私有项与 `crate::` 应用层符号；
//!   4. route 清单快照——少一条=404，多一条=意外暴露。
//!
//! 阈值口径（方案 §七）：以行数为权威（主 ≤800 / 其他 ≤700 / 资源域与职责域 ≤600）。

use std::path::{Path, PathBuf};

fn app_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn read(rel: &str) -> String {
    let p = app_src().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读取 {rel} 失败: {e}"))
}

fn loc(rel: &str) -> usize {
    read(rel).lines().count()
}

#[test]
fn god_files_stay_below_size_limits() {
    // 主文件：装配与阶段状态机
    assert!(loc("main.rs") <= 800, "main.rs 超 800 行（god file 复发）: {}", loc("main.rs"));
    assert!(loc("task_exec.rs") <= 800, "task_exec.rs 超 800 行: {}", loc("task_exec.rs"));
    // 装配层其余文件
    for f in ["state.rs", "router.rs", "ws.rs", "bootstrap.rs", "assets.rs", "pipeline.rs", "freshness.rs", "git.rs", "session_queue_routes.rs"] {
        assert!(loc(f) <= 700, "{f} 超 700 行: {}", loc(f));
    }
    // 资源域
    let routes_dir = app_src().join("routes");
    for e in std::fs::read_dir(&routes_dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().map(|x| x == "rs").unwrap_or(false) {
            let name = format!("routes/{}", p.file_name().unwrap().to_string_lossy());
            let n = std::fs::read_to_string(&p).unwrap().lines().count();
            let lim = if p.file_name().unwrap() == "mod.rs" { 300 } else { 600 };
            assert!(n <= lim, "{name} 超 {lim} 行: {n}");
        }
    }
    // 职责域（task_exec 子模块）
    let te_dir = app_src().join("task_exec");
    for e in std::fs::read_dir(&te_dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().map(|x| x == "rs").unwrap_or(false) {
            let name = format!("task_exec/{}", p.file_name().unwrap().to_string_lossy());
            let n = std::fs::read_to_string(&p).unwrap().lines().count();
            assert!(n <= 600, "{name} 超 600 行: {n}");
        }
    }
    // 测试迁移文件
    for f in ["test_support.rs", "tests_a.rs", "tests_b.rs", "tests_c.rs", "tests_d.rs"] {
        assert!(loc(f) <= 600, "{f} 超 600 行: {}", loc(f));
    }
}

#[test]
fn route_modules_have_no_horizontal_deps() {
    let dir = app_src().join("routes");
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        let fname = p.file_name().unwrap().to_string_lossy().to_string();
        if fname == "mod.rs" || !fname.ends_with(".rs") { continue; }
        let txt = std::fs::read_to_string(&p).unwrap();
        assert!(
            !txt.contains("crate::routes::"),
            "routes/{fname} 出现指向兄弟域的横向引用 crate::routes::——共享逻辑应落 crate::state（防横向依赖）"
        );
    }
}

const TASK_ENGINE_FORBIDDEN: &[&str] = &[
    "super::TaskExecutor",
    "crate::BusEvent",
    "crate::publish",
    "crate::agent_conf",
    "crate::AppState",
    "crate::AppError",
    "crate::start_",
    "crate::analyze_submap_inner",
    "crate::session_queue",
];

#[test]
fn task_exec_submodules_have_no_reverse_refs() {
    let dir = app_src().join("task_exec");
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if !p.extension().map(|x| x == "rs").unwrap_or(false) { continue; }
        let fname = p.file_name().unwrap().to_string_lossy().to_string();
        let txt = std::fs::read_to_string(&p).unwrap();
        for bad in TASK_ENGINE_FORBIDDEN {
            assert!(
                !txt.contains(bad),
                "task_exec/{fname} 出现 `{bad}`——子模块反向引用状态机/应用层，环复发（R7 守卫）"
            );
        }
    }
}

/// 冻结的 route 清单（拆分前 build_router 的全部 `.route("…")` 字面量）。
/// 拆分是纯重组：少一条=线上 404，多一条=意外暴露。
const FROZEN_ROUTES: &[&str] = &[
    "/agent/detect",
    "/agent/status",
    "/agent/test",
    "/diagnostics",
    "/harness",
    "/harness/backups",
    "/harness/file",
    "/harness/files",
    "/harness/reset",
    "/harness/restore",
    "/health",
    "/llm/test",
    "/repos",
    "/repos/{id}",
    "/repos/{id}/agent-sessions",
    "/repos/{id}/chat",
    "/repos/{id}/chat/compact",
    "/repos/{id}/chat/reset",
    "/repos/{id}/conversations",
    "/repos/{id}/conversations/{cid}",
    "/repos/{id}/dev-doc",
    "/repos/{id}/dev-docs",
    "/repos/{id}/events",
    "/repos/{id}/events/summary",
    "/repos/{id}/freshness",
    "/repos/{id}/git/commit",
    "/repos/{id}/git/commit-message",
    "/repos/{id}/git/discard",
    "/repos/{id}/git/log",
    "/repos/{id}/git/pull",
    "/repos/{id}/git/push",
    "/repos/{id}/git/status",
    "/repos/{id}/growth",
    "/repos/{id}/health-dashboard",
    "/repos/{id}/map",
    "/repos/{id}/modules/{module_id}",
    "/repos/{id}/modules/{module_id}/analyze-submap",
    "/repos/{id}/modules/{module_id}/health-history",
    "/repos/{id}/patrol",
    "/repos/{id}/patrol-runs",
    "/repos/{id}/progress",
    "/repos/{id}/reinduce",
    "/repos/{id}/sessions/{sid}/kill",
    "/repos/{id}/sessions/{sid}/output",
    "/repos/{id}/suggest",
    "/repos/{id}/tasks",
    "/repos/{id}/tasks/{tid}",
    "/repos/{id}/tasks/{tid}/approvals",
    "/repos/{id}/tasks/{tid}/decide",
    "/repos/{id}/tasks/{tid}/diff",
    "/repos/{id}/tasks/{tid}/kill",
    "/repos/{id}/tasks/{tid}/remediate",
    "/repos/{id}/tasks/{tid}/retry",
    "/repos/{id}/tasks/{tid}/review",
    "/repos/{id}/tasks/{tid}/rewind",
    "/repos/{id}/usage",
    "/repos/{id}/views",
    "/repos/{id}/views/{slug}",
    "/sessions/overview",
    "/settings",
    "/settings/set",
    "/settings/{scope}/{key}",
    "/ws"
];

#[test]
fn router_route_list_is_frozen() {
    let txt = read("router.rs");
    let mut actual: Vec<String> = Vec::new();
    for part in txt.split(".route(\"").skip(1) {
        if let Some(end) = part.find('"') {
            actual.push(part[..end].to_string());
        }
    }
    actual.sort();
    actual.dedup();
    let mut frozen: Vec<String> = FROZEN_ROUTES.iter().map(|s| s.to_string()).collect();
    frozen.sort();
    frozen.dedup();
    let missing: Vec<_> = frozen.iter().filter(|r| !actual.contains(r)).collect();
    let extra: Vec<_> = actual.iter().filter(|r| !frozen.contains(r)).collect();
    assert!(missing.is_empty() && extra.is_empty(),
        "route 清单漂移——缺失(404风险): {missing:?}；新增(意外暴露): {extra:?}");
    assert_eq!(actual.len(), frozen.len(), "route 数量不一致");
}
