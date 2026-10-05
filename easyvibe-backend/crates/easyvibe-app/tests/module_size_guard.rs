//! R7 防回胀守卫（"god file 不再复现"的机制保证）。
//!
//! 与 `tests/arch_guard.rs` 同范式：只读源码文本，不 import 符号（本 crate 为 bin-only）。
//! 断言组：
//!   1. 主文件 / 子模块 LOC 上限——god file 复发即失败；
//!   2. routes 兄弟域禁止横向引用（`crate::routes::` 与 `super::` 两种写法都堵）；
//!   3. task_exec 生产子模块禁止反向引用状态机（按实际符号判定，glob 引入不可绕过）；
//!   4. route 清单快照——少一条=404，多一条=意外暴露（扫描容忍跨行写法）。
//!
//! 阈值口径（方案 §七；N4 整改）：以行数为权威。
//!   主文件 ≤ 900：方案原定 800，实测阶段状态机 task_exec.rs 796 行无缓冲，
//!     经**显式豁免**上调至 900（仍远低于地图观测的 god 阈值：ai-agent 1013 / db 1373 被判 god）。
//!   其余装配 ≤ 700；资源域 / 职责域（含测试迁移文件）≤ 600（routes/mod.rs ≤ 300）。

use std::path::PathBuf;

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

/// 抓取源码中全部 `.route("<path>")` 字面量，容忍跨行写法（`.route(\n  "path",`）。
/// N1 整改：旧实现用 `split(".route(\"")` 会漏掉多行形式的 `/repos/{id}/session-queue`。
fn extract_routes(txt: &str) -> Vec<String> {
    let bytes = txt.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while let Some(pos) = txt[i..].find(".route(") {
        let mut j = i + pos + ".route(".len();
        while j < bytes.len() && (bytes[j] as char).is_ascii_whitespace() {
            j += 1;
        }
        if j < bytes.len() && bytes[j] == b'"' {
            let start = j + 1;
            let mut k = start;
            while k < bytes.len() && bytes[k] != b'"' {
                k += 1;
            }
            out.push(txt[start..k].to_string());
            i = k;
        } else {
            i = j.max(i + pos + 1);
        }
    }
    out
}

#[test]
fn god_files_stay_below_size_limits() {
    // 主文件：装配与阶段状态机（≤900，见文件头 N4 豁免说明）
    assert!(loc("main.rs") <= 900, "main.rs 超 900 行（god file 复发）: {}", loc("main.rs"));
    assert!(loc("task_exec.rs") <= 900, "task_exec.rs 超 900 行: {}", loc("task_exec.rs"));
    // 装配层其余文件
    for f in ["state.rs", "map_concerns.rs", "router.rs", "ws.rs", "bootstrap.rs", "assets.rs", "pipeline.rs", "freshness.rs", "git.rs", "session_queue_routes.rs"] {
        assert!(loc(f) <= 700, "{f} 超 700 行: {}", loc(f));
    }
    // 服务编排层（方案 R1：service/{mod,chat,task,map}.rs）
    let service_dir = app_src().join("service");
    for e in std::fs::read_dir(&service_dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().map(|x| x == "rs").unwrap_or(false) {
            let name = format!("service/{}", p.file_name().unwrap().to_string_lossy());
            let n = std::fs::read_to_string(&p).unwrap().lines().count();
            let lim = if p.file_name().unwrap() == "mod.rs" { 300 } else { 600 };
            assert!(n <= lim, "{name} 超 {lim} 行: {n}");
        }
    }
    // 资源域：方案 R8 单轨下调 600 → 400（存量最大 settings 334，安全；map 下沉后远低于）
    let routes_dir = app_src().join("routes");
    for e in std::fs::read_dir(&routes_dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().map(|x| x == "rs").unwrap_or(false) {
            let name = format!("routes/{}", p.file_name().unwrap().to_string_lossy());
            let n = std::fs::read_to_string(&p).unwrap().lines().count();
            let lim = if p.file_name().unwrap() == "mod.rs" { 300 } else { 400 };
            assert!(n <= lim, "{name} 超 {lim} 行: {n}");
        }
    }
    // 职责域（task_exec 子模块，含测试迁移文件）
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
        // 写法一：显式 crate::routes::<兄弟域>
        assert!(
            !txt.contains("crate::routes::"),
            "routes/{fname} 出现指向兄弟域的横向引用 crate::routes::——共享逻辑应落 crate::state / crate::service（防横向依赖）"
        );
        // 写法二：super::——在 routes/<域>.rs 中 super = routes 模块，同样能拿到兄弟域（N3 加固）
        assert!(
            !txt.contains("super::"),
            "routes/{fname} 出现 super:: —— routes 域文件不得经父模块横向取兄弟域，共享逻辑应落 crate::state / crate::service"
        );
    }
}

/// task_exec 子模块的逆向引用禁止子串（应用层/事件出口符号）。
/// 方案 R7：map 编排下沉后 `crate::start_*` / `crate::analyze_submap_inner` 旧禁串失效，
/// 改指新路径 `crate::service::map::…`（与 tests/arch_guard.rs 两处同步）。
const TASK_ENGINE_FORBIDDEN: &[&str] = &[
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
fn task_exec_submodules_have_no_reverse_refs() {
    let dir = app_src().join("task_exec");
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if !p.extension().map(|x| x == "rs").unwrap_or(false) { continue; }
        let fname = p.file_name().unwrap().to_string_lossy().to_string();
        let raw = std::fs::read_to_string(&p).unwrap();
        // N3 加固：子模块普遍 `use super::*;`，glob 会把父模块符号（含 TaskExecutor）引入，
        // 字面 `super::TaskExecutor` 永不可能出现——故先剔除 glob 导入行，再按真实符号判定。
        let txt: String = raw
            .lines()
            .filter(|l| l.trim() != "use super::*;")
            .collect::<Vec<_>>()
            .join("\n");
        for bad in TASK_ENGINE_FORBIDDEN {
            assert!(
                !txt.contains(bad),
                "task_exec/{fname} 出现 `{bad}`——子模块反向引用状态机/应用层，环复发（R7 守卫）"
            );
        }
        // 生产职责子模块（非写状态机的测试文件）不得引用状态机类型本身
        let is_state_machine_test = fname == "test_util.rs" || fname.starts_with("tests_");
        if !is_state_machine_test {
            assert!(
                !txt.contains("TaskExecutor"),
                "task_exec/{fname} 出现 `TaskExecutor`——生产职责子模块不得反向引用状态机（应经显式入参，R7 守卫）"
            );
        }
    }
}

/// 冻结的 route 清单（拆分前 build_router 的全部 `.route("…")` 字面量，含跨行写法）。
/// 拆分是纯重组：少一条=线上 404，多一条=意外暴露。
const FROZEN_ROUTES: &[&str] = &[
    "/agent/detect",
    "/agent/status",
    "/agent/test",
    "/diagnostics",
    "/harness",
    "/harness/custom/files",
    "/harness/custom/file",
    "/harness/custom/toggle",
    "/harness/custom/template",
    "/harness/custom/generate",
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
    "/repos/{id}/session-queue",
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
    "/ws",
];

#[test]
fn router_route_list_is_frozen() {
    let txt = read("router.rs");
    let mut actual: Vec<String> = extract_routes(&txt);
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

/// routes/*.rs 的纵向编排依赖禁止串（方案 R8①）——handler 只做 HTTP 边界，
/// 数据库 / agent / 编排助手 / spawn 必须落 `crate::service`。
/// 同时扫 `use` 行与函数体内限定路径：裸用 `easyvibe_db::…` 与 `use easyvibe_db::…` 都被子串命中。
const ROUTES_FORBIDDEN_ORCH: &[&str] = &[
    "easyvibe_db::",
    "easyvibe_ai_agent::",
    "crate::task_exec",
    "crate::reinduce",
    "crate::freshness",
    "crate::map_concerns",
    "crate::assets::resolve_text_asset",
    "easyvibe_map::atomic_write",
    "tokio::spawn",
    ".start_induction(",
    "session_manager.start_",
    ".try_register(",
];

/// 存量违规文件白名单（LEGACY ratchet，方案 §六-2：本轮只锁 map.rs，其余文件先登记现值）。
/// 键集必须与实际违规集**全等**：文件清干净后须摘牌，不得新增/改名蒙混。
/// 值 = 冻结的违规串出现次数（只降不升）。
const ROUTES_LEGACY_BUDGET: &[(&str, usize)] = &[
    ("agent.rs", 3),
    ("chat.rs", 2),
    ("dev_docs.rs", 1),
    ("repo.rs", 3),
    ("sessions.rs", 5),
    ("settings.rs", 6),
    ("task.rs", 9),
];

/// routes/*.rs 文件集归属快照（照 componentGuard 归属快照范式，方案 R8④）。
const ROUTES_FROZEN_FILES: &[&str] = &[
    "agent.rs", "chat.rs", "dev_docs.rs", "map.rs", "mod.rs", "repo.rs", "sessions.rs", "settings.rs", "task.rs",
];

/// service/ 文件集归属快照（方案 R1 拆分为 {mod,chat,task,map}.rs）。
const SERVICE_FROZEN_FILES: &[&str] = &["chat.rs", "map.rs", "mod.rs", "task.rs"];

fn rs_files(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "rs").unwrap_or(false))
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    v.sort();
    v
}

#[test]
fn routes_dependency_whitelist() {
    let dir = app_src().join("routes");
    let mut violating: Vec<String> = Vec::new();
    for fname in rs_files(&dir) {
        if fname == "mod.rs" { continue; }
        let txt = std::fs::read_to_string(dir.join(&fname)).unwrap();
        let count: usize = ROUTES_FORBIDDEN_ORCH.iter().map(|p| txt.matches(p).count()).sum();
        if count == 0 { continue; }
        violating.push(fname.clone());
        match ROUTES_LEGACY_BUDGET.iter().find(|(f, _)| *f == fname) {
            Some((_, budget)) => assert!(
                count <= *budget,
                "routes/{fname} 编排依赖从 {budget} 增至 {count}——handler 只做 HTTP 边界，编排须落 crate::service（R8①）"
            ),
            None => panic!(
                "routes/{fname} 出现 {count} 处纵向编排依赖（{ROUTES_FORBIDDEN_ORCH:?}）——须下沉 crate::service / crate::state，不得新登记 LEGACY"
            ),
        }
    }
    let mut declared: Vec<String> = ROUTES_LEGACY_BUDGET.iter().map(|(f, _)| f.to_string()).collect();
    declared.sort();
    violating.sort();
    assert_eq!(
        violating, declared,
        "LEGACY 棘轮漂移——实际违规集 {violating:?} != 登记集 {declared:?}（清干净须摘牌，新增文件不得蒙混进表）"
    );
}

#[test]
fn routes_file_set_is_frozen() {
    let actual = rs_files(&app_src().join("routes"));
    let mut expected: Vec<String> = ROUTES_FROZEN_FILES.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(actual, expected, "routes/ 文件集漂移——新增/改名须登记快照（防换地址复活，R8④）");
}

#[test]
fn service_file_set_is_frozen() {
    let actual = rs_files(&app_src().join("service"));
    let mut expected: Vec<String> = SERVICE_FROZEN_FILES.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(actual, expected, "service/ 文件集漂移——须登记快照（R1/R8④）");
}
