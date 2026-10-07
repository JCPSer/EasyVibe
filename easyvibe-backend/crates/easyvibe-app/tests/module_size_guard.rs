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
    // 装配层其余文件（c-arch-1：git/freshness/reinduce/pipeline/map_concerns 已迁出本 crate；
    // c-arch-10：bootstrap.rs 纯搬运至 assembly/**，本列表不再含 bootstrap）
    for f in ["state.rs", "router.rs", "ws.rs", "assets.rs", "session_queue_routes.rs", "db_ports.rs"] {
        assert!(loc(f) <= 700, "{f} 超 700 行: {}", loc(f));
    }
    // c-arch-10：独立装配格（assembly/**）≤600（使命是「短主线 + 分组工厂」，不得再膨胀为 god 格）
    let asm_dir = app_src().join("assembly");
    for e in std::fs::read_dir(&asm_dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().map(|x| x == "rs").unwrap_or(false) {
            let name = format!("assembly/{}", p.file_name().unwrap().to_string_lossy());
            let n = std::fs::read_to_string(&p).unwrap().lines().count();
            assert!(n <= 600, "{name} 超 600 行: {n}（装配格须保持短主线 + 分组工厂）");
        }
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
    // c-arch-7 R3：端口适配器落组合根后，切片不得反向引用组合根/新增 service 域
    "crate::db_ports",
    "crate::service::sessions",
    "crate::service::settings",
    "crate::service::agent",
    // c-arch-10：切片不得反向引用装配格
    "crate::assembly",
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
///
/// R2（c-arch-1）：自注册后本清单不再由 router.rs 直接比对，而由 `FROZEN_DOMAIN_ROUTES`
/// 的并集**聚合导出**（断言组 B）——单一事实源是域表，本表只做「总数不增不减」的交叉校验。
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
    "/repos/{id}/git/diff",
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

/// R1/R2（c-arch-1）：域 → 路由集合冻结表（**单一事实源**）。
/// 由原 router.rs 内联 65 处 `.route(` 按 **handler 归属**拆分后逐条复核：
/// 路由随 handler 所在文件归属，避免 routes 兄弟域横向引用（守卫 `route_modules_have_no_horizontal_deps`）。
///   - repo.rs：仓库域 + git 子资源（git handler 留 crate::git，仅注册）
///   - sessions.rs：会话/队列/用量/看板/埋点/巡检历史（含 /repos/{id}/patrol-runs——
///     其 handler list_patrol_runs/prune_patrol_runs 实际定义在 sessions.rs）
///   - router.rs：装配层仅 `/ws`
/// 键集必须与 routes/ 磁盘文件集（除 mod.rs）+ router.rs **全等**（新增域未登记 → 断言 A 红）。
const FROZEN_DOMAIN_ROUTES: &[(&str, &[&str])] = &[
    (
        "repo.rs",
        &[
            "/health",
            "/repos",
            "/repos/{id}",
            "/repos/{id}/git/status",
            "/repos/{id}/git/log",
            "/repos/{id}/git/commit",
            "/repos/{id}/git/diff",
            "/repos/{id}/git/pull",
            "/repos/{id}/git/push",
            "/repos/{id}/git/discard",
            "/repos/{id}/git/commit-message",
        ],
    ),
    (
        "map.rs",
        &[
            "/repos/{id}/map",
            "/repos/{id}/freshness",
            "/repos/{id}/modules/{module_id}/health-history",
            "/repos/{id}/modules/{module_id}/analyze-submap",
            "/repos/{id}/growth",
            "/repos/{id}/progress",
            "/repos/{id}/modules/{module_id}",
            "/repos/{id}/reinduce",
            "/repos/{id}/patrol",
        ],
    ),
    (
        "sessions.rs",
        &[
            "/repos/{id}/session-queue",
            "/repos/{id}/patrol-runs",
            "/sessions/overview",
            "/repos/{id}/agent-sessions",
            "/repos/{id}/usage",
            "/repos/{id}/health-dashboard",
            "/repos/{id}/events",
            "/repos/{id}/events/summary",
            "/repos/{id}/sessions/{sid}/kill",
            "/repos/{id}/sessions/{sid}/output",
        ],
    ),
    (
        "chat.rs",
        &[
            "/repos/{id}/chat",
            "/repos/{id}/conversations",
            "/repos/{id}/conversations/{cid}",
            "/repos/{id}/chat/compact",
            "/repos/{id}/chat/reset",
            "/repos/{id}/views",
            "/repos/{id}/views/{slug}",
        ],
    ),
    (
        "task.rs",
        &[
            "/repos/{id}/tasks",
            "/repos/{id}/tasks/{tid}",
            "/repos/{id}/tasks/{tid}/decide",
            "/repos/{id}/tasks/{tid}/retry",
            "/repos/{id}/tasks/{tid}/remediate",
            "/repos/{id}/tasks/{tid}/rewind",
            "/repos/{id}/tasks/{tid}/review",
            "/repos/{id}/tasks/{tid}/approvals",
            "/repos/{id}/tasks/{tid}/diff",
            "/repos/{id}/tasks/{tid}/kill",
            "/repos/{id}/suggest",
        ],
    ),
    ("dev_docs.rs", &["/repos/{id}/dev-docs", "/repos/{id}/dev-doc"]),
    (
        "settings.rs",
        &[
            "/settings",
            "/settings/set",
            "/settings/{scope}/{key}",
            "/harness",
            "/harness/custom/files",
            "/harness/custom/file",
            "/harness/custom/toggle",
            "/harness/custom/template",
            "/harness/custom/generate",
            "/diagnostics",
        ],
    ),
    (
        "agent.rs",
        &["/agent/status", "/agent/detect", "/agent/test", "/llm/test"],
    ),
    ("router.rs", &["/ws"]),
];

/// 读某个域文件的 `.route(` 路径集合（去重升序）。router.rs 走装配层路径。
fn domain_route_set(file: &str) -> Vec<String> {
    let txt = if file == "router.rs" { read("router.rs") } else { read(&format!("routes/{file}")) };
    let mut v: Vec<String> = extract_routes(&txt);
    v.sort();
    v.dedup();
    v
}

/// 断言组 A：域表 ↔ 磁盘自注册**双向全等**（键集 + 每域路径集）。
/// 新增域未登记、某域路由增删改名，必失败。
#[test]
fn domain_route_table_is_frozen() {
    // 键集全等：routes/ 实况（除 mod.rs）+ router.rs
    let mut actual_keys: Vec<String> = rs_files(&app_src().join("routes"))
        .into_iter()
        .filter(|f| f != "mod.rs")
        .collect();
    actual_keys.push("router.rs".to_string());
    actual_keys.sort();
    let mut expected_keys: Vec<String> = FROZEN_DOMAIN_ROUTES.iter().map(|(f, _)| f.to_string()).collect();
    expected_keys.sort();
    assert_eq!(
        actual_keys, expected_keys,
        "域表键集漂移——routes/ 文件集与 FROZEN_DOMAIN_ROUTES 不一致（新增/删除/改名域须同步登记）"
    );

    for (file, paths) in FROZEN_DOMAIN_ROUTES {
        let mut expected: Vec<String> = paths.iter().map(|s| s.to_string()).collect();
        expected.sort();
        expected.dedup();
        let actual = domain_route_set(file);
        assert_eq!(
            actual, expected,
            "routes/{file} 自注册路由漂移——少一条=404，多一条=意外暴露（未登记即失败）"
        );
    }
}

/// 断言组 B：域表并集 == 全局冻结清单 64 条（单一事实源在域表，本表只交叉校验）。
#[test]
fn domain_routes_union_matches_global_frozen_list() {
    let mut union: Vec<String> = FROZEN_DOMAIN_ROUTES
        .iter()
        .flat_map(|(_, paths)| paths.iter().map(|s| s.to_string()))
        .collect();
    union.sort();
    union.dedup();
    let mut frozen: Vec<String> = FROZEN_ROUTES.iter().map(|s| s.to_string()).collect();
    frozen.sort();
    frozen.dedup();
    assert_eq!(
        union, frozen,
        "域表并集与全局 FROZEN_ROUTES 漂移——两处清单必须恒等（防双轨维护）"
    );
    assert_eq!(union.len(), 65, "route 总数应为 65（含 /ws）");
}

/// 断言组 C（防假绿·关键）：每个域模块自注册非空（≥1）。
/// 杜绝「某域 router() 被删/清空但域表仍写着、extract 空集与空集对比」的静默失效。
#[test]
fn every_domain_self_registers_at_least_one_route() {
    for (file, _) in FROZEN_DOMAIN_ROUTES {
        let actual = domain_route_set(file);
        assert!(
            !actual.is_empty(),
            "routes/{file} 自注册路由为空——router() 被删/清空或 extract_routes 静默失效（R2 断言 C）"
        );
    }
}

/// 断言组 D（防假绿·关键）：装配层 router.rs 除 `/ws` 外零资源域 `.route(`。
/// 保证「新增域必去各域模块」，也为断言 A 的 router.rs 条目兜底。
#[test]
fn router_assembly_registers_only_ws() {
    assert_eq!(
        domain_route_set("router.rs"),
        vec!["/ws".to_string()],
        "router.rs 只应注册 /ws——资源域路由须落各自 routes/<域>.rs（R1 收敛）"
    );
    let count = read("router.rs").matches(".route(").count();
    assert_eq!(count, 1, "router.rs 出现 {count} 处 .route(（应为 1，仅 /ws）——路由回流装配层");
}

/// routes/*.rs 的纵向编排依赖禁止串（方案 R8①）——handler 只做 HTTP 边界，
/// 数据库 / agent / 编排助手 / spawn 必须落 `crate::service`。
/// 同时扫 `use` 行与函数体内限定路径：裸用 `easyvibe_db::…` 与 `use easyvibe_db::…` 都被子串命中。
const ROUTES_FORBIDDEN_ORCH: &[&str] = &[
    // c-arch-7：`easyvibe_db::` 判据已移交 tests/arch_guard.rs（权威落点，含去注释与反绕过），
    // 本表不再重复（避免双轨：一处绿一处红）。
    "easyvibe_ai_agent::",
    "crate::task_exec",
    "crate::reinduce",
    "crate::freshness",
    "crate::map_concerns",
    // c-arch-1：git / pipeline 均已外提——routes 取能力只经 crate::service
    "crate::git",
    "crate::pipeline",
    "easyvibe_git::",
    "easyvibe_pipeline::",
    // c-arch-1 审查 P1：easyvibe-map 的领域子模块是同一批规则的新地址，
    // service/map.rs 正是直接调 easyvibe_map::{freshness,induction,concerns}——
    // 若不入表，routes/* 写 easyvibe_map::induction::decide(..) 即可绕过本守卫
    "easyvibe_map::freshness::",
    "easyvibe_map::induction::",
    "easyvibe_map::concerns::",
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
    // c-arch-7 R1 后实测：仅 settings.rs 残留（harness 自定义槽端点，纯 FS + harness 装载），
    // 其余六域全部摘牌（编排/仓储调用已下沉 crate::service）。棘轮只降不升，键集与实际集全等。
    ("settings.rs", 4),
];

/// routes/*.rs 文件集归属快照（照 componentGuard 归属快照范式，方案 R8④）。
const ROUTES_FROZEN_FILES: &[&str] = &[
    "agent.rs", "chat.rs", "dev_docs.rs", "map.rs", "mod.rs", "repo.rs", "sessions.rs", "settings.rs", "task.rs",
];

/// service/ 文件集归属快照（方案 R1 拆分为 {mod,chat,task,map}.rs；
/// c-arch-1 增 {git,repo,reinduce}.rs —— routes 只经 crate::service 取能力的落点）。
const SERVICE_FROZEN_FILES: &[&str] = &["agent.rs", "chat.rs", "git.rs", "map.rs", "mod.rs", "reinduce.rs", "repo.rs", "sessions.rs", "settings.rs", "task.rs"];

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

// ============================================================================
// R6 + R9（c-arch-1）：app crate 源码文件集冻结 + 归属分类 + 五领域文件禁止存在
// ============================================================================

/// 文件归属类别（**封闭**：无第五类，且每类必须非空——见 `app_files_have_declared_ownership`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SrcClass {
    /// 组合根：装配与共享状态（命题点名的 main / router / state / db_ports）
    CompositionRoot,
    /// c-arch-10：独立装配格（启动装配 + 静态托管；同层单向 assembly → server-api）
    Assembly,
    /// HTTP/WS 边界：请求 / 响应面
    HttpBoundary,
    /// task-engine 切片（共享同一 crate；由 `arch_guard.rs` 约束反向引用）
    TaskEngineSlice,
    /// 测试夹具（只进 test target）
    TestFixture,
}

/// 顶层归属快照（**单一事实源**：文件 + 类别 + 理由）。
/// `app_top_files()` / `app_task_engine_files()` 均由本表**派生**（键集不再双轨维护）。
///
/// R9 命题对照：「app crate 只留 main/router/state/bootstrap/ws 与 HTTP 边界」是散文，
/// 实测白名单 12 项 ≠ 命题 5 项。差异**逐项给出理由**（下表第 3 列），使「只留 X」与代码事实一致：
///   - 命题点名的 5 项：main / router / state / bootstrap / ws —— 见 CompositionRoot + HttpBoundary(ws)；
///   - 命题未点名但正当的 3 项：assets（受管资产解析，零业务规则）、
///     session_queue_routes（会话队列请求/响应面）、task_exec 切片（task-engine，另有 arch_guard 约束）；
///   - 命题未提及的 5 项：test_support / tests_a~d —— 只为测试而存在，不进生产二进制。
const APP_SRC_OWNERSHIP: &[(SrcClass, &str, &str)] = &[
    // —— 组合根（命题点名）——
    (SrcClass::CompositionRoot, "main.rs", "命题点名：进程入口与 mod 声明"),
    (SrcClass::CompositionRoot, "router.rs", "命题点名：域自注册 merge 与 /ws 装配"),
    (SrcClass::CompositionRoot, "state.rs", "命题点名：共享状态与助手；组合根本就要装配依赖"),
    (SrcClass::CompositionRoot, "db_ports.rs", "c-arch-7/c-arch-10 组合根：端口 ↔ 具体仓储适配器唯一落点（依赖倒置）"),
    // —— c-arch-10 独立装配格（启动装配 + 静态托管；bootstrap.rs 纯搬运 + router.rs 静态段外提）——
    (SrcClass::Assembly, "assembly/mod.rs", "c-arch-10：run() 主线 + build_state() 工厂；装配只许落装配格"),
    (SrcClass::Assembly, "assembly/logging.rs", "c-arch-10：日志/仓库注册/预热/资产/agent 解析（自 bootstrap.rs 搬运）"),
    (SrcClass::Assembly, "assembly/bridges.rs", "c-arch-10：5 条落库/直播桥（自 bootstrap.rs 内联闭包搬运）"),
    (SrcClass::Assembly, "assembly/schedulers.rs", "c-arch-10：定时器/启动探测/队列宿主（自 bootstrap.rs 内联闭包搬运）"),
    (SrcClass::Assembly, "assembly/static_host.rs", "c-arch-10：静态回落 + 内嵌 dist + MIME（自 router.rs/assets.rs 外提）"),
    // —— HTTP/WS 边界 ——
    (SrcClass::HttpBoundary, "ws.rs", "命题点名：WS 面（事件名契约由 contract_guard 冻结）"),
    (SrcClass::HttpBoundary, "assets.rs", "命题未点名：受管资产解析（编译期 include 清单 + 启动期读取），零业务规则"),
    (SrcClass::HttpBoundary, "session_queue_routes.rs", "命题未点名：会话队列请求/响应面；含 1 处 map-domain 纯校验调用（入棘轮）"),
    // —— task-engine 切片（命题未点名，另有 arch_guard 独立约束）——
    (SrcClass::TaskEngineSlice, "task_exec.rs", "task-engine 切片根：装配与阶段状态机，由 arch_guard.rs 约束反向引用"),
    (SrcClass::TaskEngineSlice, "task_exec/ports.rs", "c-arch-7 切片内持久化端口与本地 DTO（去具体仓储类型）"),
    (SrcClass::TaskEngineSlice, "task_exec/changes.rs", "切片内按职责拆分（变更集）"),
    (SrcClass::TaskEngineSlice, "task_exec/contract.rs", "切片内按职责拆分（契约门）"),
    (SrcClass::TaskEngineSlice, "task_exec/harness.rs", "切片内按职责拆分（harness 装载）"),
    (SrcClass::TaskEngineSlice, "task_exec/prompt.rs", "切片内按职责拆分（提示词渲染）"),
    (SrcClass::TaskEngineSlice, "task_exec/review.rs", "切片内按职责拆分（审查门）"),
    (SrcClass::TaskEngineSlice, "task_exec/test_util.rs", "切片内测试夹具（写状态机，仅 test target）"),
    (SrcClass::TaskEngineSlice, "task_exec/tests_changes.rs", "切片内按域拆分的集成级夹具"),
    (SrcClass::TaskEngineSlice, "task_exec/tests_contract.rs", "切片内按域拆分的集成级夹具"),
    (SrcClass::TaskEngineSlice, "task_exec/tests_flow.rs", "切片内按域拆分的集成级夹具"),
    (SrcClass::TaskEngineSlice, "task_exec/tests_gates.rs", "切片内按域拆分的集成级夹具"),
    (SrcClass::TaskEngineSlice, "task_exec/tests_harness.rs", "切片内按域拆分的集成级夹具"),
    (SrcClass::TaskEngineSlice, "task_exec/tests_prompt.rs", "切片内按域拆分的集成级夹具"),
    // —— 测试夹具 ——
    (SrcClass::TestFixture, "test_support.rs", "命题未提及：跨文件共享测试夹具，仅 test target 编译"),
    (SrcClass::TestFixture, "tests_a.rs", "命题未提及：按域拆分的集成级夹具"),
    (SrcClass::TestFixture, "tests_b.rs", "命题未提及：按域拆分的集成级夹具"),
    (SrcClass::TestFixture, "tests_c.rs", "命题未提及：按域拆分的集成级夹具"),
    (SrcClass::TestFixture, "tests_d.rs", "命题未提及：按域拆分的集成级夹具"),
];

/// 封闭类别全集（键集必须与此**全等**：既无第五类，也不得某类为空）。
const APP_SRC_CLASSES: &[SrcClass] = &[
    SrcClass::CompositionRoot,
    SrcClass::Assembly,
    SrcClass::HttpBoundary,
    SrcClass::TaskEngineSlice,
    SrcClass::TestFixture,
];

/// c-arch-10：装配格文件集（显式冻结；磁盘 `src/assembly/*.rs` 必须与之全等）。
const ASSEMBLY_FROZEN_FILES: &[&str] = &[
    "mod.rs", "logging.rs", "bridges.rs", "schedulers.rs", "static_host.rs",
];

/// 顶层文件集（由归属表派生：非 task-engine 切片、非装配格项——后两者按目录单独登记）。
fn app_top_files() -> Vec<String> {
    APP_SRC_OWNERSHIP
        .iter()
        .filter(|(c, _, _)| *c != SrcClass::TaskEngineSlice && *c != SrcClass::Assembly)
        .map(|(_, f, _)| f.to_string())
        .collect()
}

/// 装配格文件集（由归属表派生：Assembly 项，含 `assembly/` 前缀）。
fn app_assembly_files() -> Vec<String> {
    APP_SRC_OWNERSHIP
        .iter()
        .filter(|(c, _, _)| *c == SrcClass::Assembly)
        .map(|(_, f, _)| f.to_string())
        .collect()
}

/// task_exec 切片文件集（由归属表派生：task-engine 切片项）。
fn app_task_engine_files() -> Vec<String> {
    APP_SRC_OWNERSHIP
        .iter()
        .filter(|(c, _, _)| *c == SrcClass::TaskEngineSlice)
        .map(|(_, f, _)| f.to_string())
        .collect()
}

/// 五个领域文件必须已外提：app crate 只留 HTTP 边界与装配。
const EXTRACTED_DOMAIN_FILES: &[&str] = &["git.rs", "freshness.rs", "reinduce.rs", "pipeline.rs", "map_concerns.rs"];

/// 递归收集 src/**/*.rs（相对 src/ 的 `/` 分隔路径，升序）。
fn app_src_tree() -> Vec<String> {
    fn walk(dir: &std::path::Path, prefix: &str, out: &mut Vec<String>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if p.is_dir() {
                walk(&p, &format!("{prefix}{name}/"), out);
            } else if name.ends_with(".rs") {
                out.push(format!("{prefix}{name}"));
            }
        }
    }
    let mut out = Vec::new();
    walk(&app_src(), "", &mut out);
    out.sort();
    out
}

#[test]
fn app_src_tree_is_frozen() {
    let mut expected: Vec<String> = app_top_files();
    expected.extend(rs_files(&app_src().join("routes")).into_iter().map(|f| format!("routes/{f}")));
    expected.extend(rs_files(&app_src().join("service")).into_iter().map(|f| format!("service/{f}")));
    expected.extend(app_task_engine_files());
    // c-arch-10：装配格文件按目录单独登记（assembly/ 前缀，单一事实源 = APP_SRC_OWNERSHIP）
    expected.extend(app_assembly_files());
    expected.sort();

    let actual = app_src_tree();
    assert_eq!(
        actual, expected,
        "easyvibe-app/src 文件集漂移——多一个=边界层复活/未登记归属，少一个=误删（c-arch-1 R6 冻结）"
    );
    assert!(
        !expected.iter().any(|f| f.starts_with("routes/") && !f.ends_with(".rs")),
        "routes 归属项必须是 .rs 文件"
    );
}

/// c-arch-10：装配格文件集**双向全等**（磁盘 `src/assembly/*.rs` == 显式冻结 == 归属表 Assembly 项）。
#[test]
fn assembly_file_set_is_frozen() {
    let actual = rs_files(&app_src().join("assembly"));
    let mut expected: Vec<String> = ASSEMBLY_FROZEN_FILES.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(actual, expected, "assembly/ 文件集漂移——装配格新增/改名须同步 ASSEMBLY_FROZEN_FILES（c-arch-10 R6）");
    let mut declared: Vec<String> = app_assembly_files().into_iter().map(|f| f.strip_prefix("assembly/").unwrap().to_string()).collect();
    declared.sort();
    assert_eq!(actual, declared, "assembly/ 文件集与 APP_SRC_OWNERSHIP 的 Assembly 项不一致（单一事实源被破坏）");
}

/// R9-①：顶层归属表键集 ↔ 磁盘顶层 .rs **双向全等**（与 app_src_tree_is_frozen 同义，
/// 但键来源是派生自归属表——防「表与派生两处漂移」）。
/// routes/ 与 service/ 仍**由磁盘扫描派生**（不得硬编码，否则新增路由文件会被漏掉）。
#[test]
fn app_top_files_keys_match_disk() {
    let mut expected: Vec<String> = APP_SRC_OWNERSHIP
        .iter()
        .map(|(_, f, _)| f.to_string())
        .filter(|f| !f.contains('/'))
        .collect();
    expected.sort();
    let actual: Vec<String> = app_src_tree().into_iter().filter(|p| !p.contains('/')).collect();
    assert_eq!(
        actual, expected,
        "顶层文件集与 APP_SRC_OWNERSHIP 不一致——新增/删除顶层文件须同步归属表（R9）"
    );
    // 归属表内部一致性：无重复键
    let mut keys: Vec<&str> = APP_SRC_OWNERSHIP.iter().map(|(_, f, _)| *f).collect();
    keys.sort();
    let n = keys.len();
    keys.dedup();
    assert_eq!(keys.len(), n, "APP_SRC_OWNERSHIP 存在重复文件键——单点权威被破坏（R9）");
}

/// R9-②：归属分类可判定——类别全集封闭（无第五类）、每类非空、每条理由非占位、
/// 且与五领域外提清单互斥。
#[test]
fn app_files_have_declared_ownership() {
    let mut declared: Vec<SrcClass> = APP_SRC_OWNERSHIP.iter().map(|(c, _, _)| *c).collect();
    declared.sort_by_key(|c| format!("{c:?}"));
    declared.dedup();
    let mut expected: Vec<SrcClass> = APP_SRC_CLASSES.to_vec();
    expected.sort_by_key(|c| format!("{c:?}"));
    assert_eq!(
        declared, expected,
        "归属类别集合与 APP_SRC_CLASSES 不一致——每类必须非空且无第五类（R9）"
    );

    for class in APP_SRC_CLASSES {
        let n = APP_SRC_OWNERSHIP.iter().filter(|(c, _, _)| c == class).count();
        assert!(n > 0, "归属类别 {class:?} 为空——分类退化为摆设（R9）");
    }

    for (class, f, reason) in APP_SRC_OWNERSHIP {
        assert!(
            reason.chars().count() >= 8,
            "{f} 的归属理由 `{reason}` 过短（{class:?}）——须逐项给出「为什么不外提」（R9）"
        );
        assert!(
            !EXTRACTED_DOMAIN_FILES.contains(f),
            "{f} 同时出现在归属表与五领域外提清单——语义互斥（R9）"
        );
    }
}

/// 边界类文件（HttpBoundary）的纵向编排禁止串（组合根豁免：它就是要装配依赖）。
const APP_BOUNDARY_FORBIDDEN: &[&str] = &[
    "easyvibe_db::",
    "easyvibe_ai_agent::",
    "crate::task_exec",
    "crate::reinduce",
    "crate::freshness",
    "crate::map_concerns",
    "crate::git",
    "crate::pipeline",
    "easyvibe_git::",
    "easyvibe_pipeline::",
    "easyvibe_map::",
    "tokio::spawn",
];

/// 边界类棘轮（键集必须与实际边界文件集全等；数值只降不升）。
/// 实测依据：`session_queue_routes.rs` 现有一处 `easyvibe_map::is_valid_id(mid)`——纯 id 校验
/// （无 IO、无状态），属正当边界校验而非业务规则。逐条禁止会立刻自红，故登记为预算 1 只降不升。
const APP_BOUNDARY_BUDGET: &[(&str, usize)] =
    &[("assets.rs", 0), ("session_queue_routes.rs", 1), ("ws.rs", 0)];

/// R9-③：**行为面**判据——边界类文件不得含纵向编排（DB/agent/领域规则/spawn）。
/// 与 R7 G1（routes/）互补：G1 管 routes/，本条管其余 HTTP/WS 边界面。
#[test]
fn boundary_files_have_no_vertical_orchestration() {
    let boundary: Vec<String> =
        APP_SRC_OWNERSHIP.iter().filter(|(c, _, _)| *c == SrcClass::HttpBoundary).map(|(_, f, _)| f.to_string()).collect();

    let mut declared: Vec<String> = APP_BOUNDARY_BUDGET.iter().map(|(f, _)| f.to_string()).collect();
    declared.sort();
    let mut actual_keys = boundary.clone();
    actual_keys.sort();
    assert_eq!(
        actual_keys, declared,
        "边界类棘轮键集漂移——边界文件集与 APP_BOUNDARY_BUDGET 不一致（清干净须摘牌，R9）"
    );

    for f in &boundary {
        let txt = read(f);
        let count: usize = APP_BOUNDARY_FORBIDDEN.iter().map(|p| txt.matches(p).count()).sum();
        let budget = APP_BOUNDARY_BUDGET
            .iter()
            .find(|(n, _)| n == f)
            .map(|(_, b)| *b)
            .unwrap_or_else(|| panic!("{f} 未登记边界预算"));
        assert!(
            count <= budget,
            "边界文件 {f} 纵向编排从 {budget} 增至 {count}——业务规则须落 crate::service（R9/R7 G1 同源）"
        );
    }
}

#[test]
fn extracted_domain_files_are_gone() {
    for f in EXTRACTED_DOMAIN_FILES {
        assert!(
            !app_src().join(f).exists(),
            "easyvibe-app/src/{f} 仍存在——领域规则内驻边界层，c-arch-1 复发（R6）"
        );
        assert!(
            !read("main.rs").contains(&format!("mod {};", f.trim_end_matches(".rs"))),
            "main.rs 仍声明 `mod {};`——领域模块未真正外提（R6）",
            f.trim_end_matches(".rs")
        );
    }
}

// ============================================================================
// R7（c-arch-1）：边界层不得含业务规则 —— G1（禁串，见 ROUTES_FORBIDDEN_ORCH）
//                              + G2 禁解析型业务 + G3′ 归属映射扇出
// ============================================================================

/// 去 `//` 行注释与 `/* */` 块注释（保留字符串字面量），用于按「真实代码」判定。
fn strip_comments(txt: &str) -> String {
    let bytes = txt.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    let mut state = 0u8; // 0=code 1=line 2=block 3=str
    while i < bytes.len() {
        let c = bytes[i];
        let n = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        match state {
            0 => {
                if c == b'/' && n == b'/' {
                    state = 1;
                    i += 2;
                    continue;
                }
                if c == b'/' && n == b'*' {
                    state = 2;
                    i += 2;
                    continue;
                }
                if c == b'"' {
                    state = 3;
                }
                out.push(c);
            }
            1 => {
                if c == b'\n' {
                    state = 0;
                    out.push(c);
                }
            }
            2 => {
                if c == b'*' && n == b'/' {
                    state = 0;
                    i += 2;
                    continue;
                }
            }
            _ => {
                out.push(c);
                if c == b'\\' && i + 1 < bytes.len() {
                    out.push(n);
                    i += 2;
                    continue;
                }
                if c == b'"' {
                    state = 0;
                }
            }
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 内联 serde 解析棘轮（G2）：routes 只做入参解析与响应映射，不得在此解码业务 JSON。
/// 现存命中均为「DB 列 JSON / 透传字段」的错误兜底——逐文件显式登记，只降不升。
const ROUTES_JSON_PARSE_RATCHET: &[(&str, usize)] = &[
    // c-arch-7 R1 后 routes 侧零内联业务 JSON 解析（全部随编排下沉 crate::service）——
    // 空表即「不得再出现」：新增解析即未登记必红。
];
const ROUTES_FORBIDDEN_PARSE: &[&str] = &["serde_json::from_str", "serde_json::from_value", "serde_json::to_value"];

#[test]
fn routes_have_no_undeclared_json_decoding() {
    let dir = app_src().join("routes");
    let mut violating: Vec<String> = Vec::new();
    for fname in rs_files(&dir) {
        if fname == "mod.rs" { continue; }
        let txt = read(&format!("routes/{fname}"));
        let count: usize = ROUTES_FORBIDDEN_PARSE.iter().map(|p| txt.matches(p).count()).sum();
        if count == 0 { continue; }
        violating.push(fname.clone());
        match ROUTES_JSON_PARSE_RATCHET.iter().find(|(f, _)| *f == fname) {
            Some((_, budget)) => assert!(
                count <= *budget,
                "routes/{fname} 内联 JSON 解析从 {budget} 增至 {count}——解析型业务须落 crate::service（R7 G2）"
            ),
            None => panic!(
                "routes/{fname} 出现 {count} 处内联 JSON 解析——新文件不得在边界层解码业务 JSON（R7 G2）"
            ),
        }
    }
    let mut declared: Vec<String> = ROUTES_JSON_PARSE_RATCHET.iter().map(|(f, _)| f.to_string()).collect();
    declared.sort();
    violating.sort();
    assert_eq!(violating, declared, "R7 G2 棘轮漂移——实际 {violating:?} != 登记 {declared:?}");
}

/// G3′（v2 重写）：routes 文件的 service 域**归属映射 + 受限例外**。
/// 默认每文件 ≤1 个 service 域；全局恰允许 1 个 len==2 的登记项
/// （`repo.rs` = 主域 `repo` + 子资源域 `git`，由 `/repos/{id}/git/*` 路径前缀证明其非跨域编排）。
const ROUTE_SERVICE_MAP: &[(&str, &[&str], &str)] = &[
    ("agent.rs", &["agent"], "agent"),
    ("chat.rs", &["chat"], "chat"),
    ("dev_docs.rs", &["task"], "task"),
    ("map.rs", &["map"], "map"),
    ("repo.rs", &["repo", "git"], "repo"),
    ("sessions.rs", &["sessions"], "sessions"),
    ("settings.rs", &["settings"], "settings"),
    ("task.rs", &["task"], "task"),
];
const FANOUT_EXCEPTION_MAX: usize = 1;

#[test]
fn routes_service_fanout_is_declared() {
    // 键集联动：与 routes 磁盘文件集、域路由表键集**同源**（结构不可悄悄漂移）
    let mut keys: Vec<String> = rs_files(&app_src().join("routes")).into_iter().filter(|f| f != "mod.rs").collect();
    keys.sort();
    let mut declared: Vec<String> = ROUTE_SERVICE_MAP.iter().map(|(f, _, _)| f.to_string()).collect();
    declared.sort();
    assert_eq!(keys, declared, "ROUTE_SERVICE_MAP 键集必须与 routes/ 磁盘文件集（除 mod.rs）全等");

    let mut domain_keys: Vec<String> =
        FROZEN_DOMAIN_ROUTES.iter().map(|(f, _)| f.to_string()).filter(|f| f != "router.rs").collect();
    domain_keys.sort();
    assert_eq!(domain_keys, declared, "ROUTE_SERVICE_MAP 键集必须与 FROZEN_DOMAIN_ROUTES（除 router.rs）全等");

    let exception_count = ROUTE_SERVICE_MAP.iter().filter(|(_, d, _)| d.len() == 2).count();
    assert!(
        exception_count <= FANOUT_EXCEPTION_MAX,
        "len==2 的扇出例外有 {exception_count} 个 > {FANOUT_EXCEPTION_MAX}——例外只降不升（R7 G3′）"
    );

    for (file, decl, primary) in ROUTE_SERVICE_MAP {
        assert!(decl.len() <= 2, "routes/{file} 声明的 service 域 {} 个 > 2（R7 G3′）", decl.len());
        let raw = read(&format!("routes/{file}"));
        let txt = strip_comments(&raw);
        // 去 `use X as _;` 别名绕过面
        let txt: String = txt.lines().filter(|l| !(l.trim_start().starts_with("use ") && l.contains(" as _;"))).collect::<Vec<_>>().join("\n");
        let mut actual: Vec<String> = Vec::new();
        let mut rest = txt.as_str();
        while let Some(pos) = rest.find("crate::service::") {
            let after = &rest[pos + "crate::service::".len()..];
            let id: String = after.chars().take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_').collect();
            if !id.is_empty() && !actual.contains(&id) { actual.push(id); }
            rest = &after[..];
            if rest.is_empty() { break; }
        }
        let uses_root = txt.contains("crate::service::{");
        for d in &actual {
            assert!(decl.contains(&d.as_str()), "routes/{file} 调用了未声明的 service 域 `{d}`（声明 {decl:?}）——R7 G3′");
        }
        if uses_root {
            assert!(decl.contains(primary), "routes/{file} 经 crate::service::{{…}} 根 re-export，但主域 `{primary}` 未声明（R7 G3′）");
        }
        if decl.is_empty() {
            assert!(!uses_root && actual.is_empty(), "routes/{file} 声明零 service 域却仍在编排（R7 G3′）");
        }
        // 子资源域证明：len==2 只能是「主域 + 其子资源路径前缀」，不是任意跨域编排
        if decl.len() == 2 {
            let sub = decl.iter().find(|d| **d != *primary).expect("非主域项");
            let routes = FROZEN_DOMAIN_ROUTES.iter().find(|(f, _)| f == file).map(|(_, r)| *r).unwrap_or(&[]);
            let sub_routes = routes.iter().filter(|r| r.contains(&format!("/{sub}/"))).count();
            assert!(*file == "repo.rs" && *sub == "git" && sub_routes > 0,
                "routes/{file} 的 2 项扇出未通过子资源前缀证明（唯一合法组合是 repo.rs = repo + git）——换组合须走评审改表");
        }
    }
}
