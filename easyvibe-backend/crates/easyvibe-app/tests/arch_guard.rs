//! R8 防回归守卫：task-engine 源码不得再出现指向 server-api 符号的 `crate::` 反向引用。
//!
//! 与 `.easyvibe/map/scan_easyvibe.py` 的模块表（`MODS['task-engine']`）同源——扫描器的
//! `task-engine -> server-api` 边由 `task_exec.rs` 中的 `crate::BusEvent` 字面量触发；
//! 本测试以源码事实守卫同一约束（权威口径，先于启发式）。
//! 违反即失败：后续在 task_exec.rs 写回 `crate::BusEvent` 即让 server-api ↔ task-engine 环复发。
//!
//! c-arch-7（本文件新增断言组）——**app 层 DB 边界**：server-api（`src/routes/**`）与
//! task-engine（`task_exec` 生产文件）不得直连 `easyvibe_db`：
//!   · 命题原文：「把跨域写库收敛到各自 service，并在 app crate 的 arch_guard 禁用串里加
//!     『task_exec/** 与 routes/** 不得直接 use easyvibe_db』，让这条隐式耦合变成可执行判据。」
//!   · routes 侧：跨域写库下沉 `crate::service`；routes handler 亦不得再直取 `st.<repo>`。
//!   · task_exec 侧：持久化经切片内端口（`task_exec::ports`），适配器落组合根 `crate::db_ports`。
//!   · 测试夹具（`task_exec/{test_util,tests_*}.rs`）构造真实 SQLite 仓储验证原子关卡语义，
//!     **显式冻结豁免**（双向全等 + 只降不升棘轮），不得用 glob——防漏报防多报。
//!   · 反绕过（R5）：`src/**` 不得 `use easyvibe_db as <alias>` 再导出（堵 `crate::db::X` 逃逸）；
//!     判据自身带负例自证（见 `db_boundary_guard_selfcheck`）。
//!   · CI 镜像：`scripts/check_app_db_boundary.py`（本仓 CI 不跑 cargo，解析本文件常量为单一事实源）。
//!
//! 判据语义边界（显式承认）：子串/字面量层判据，不解析宏展开；Rust 层去注释取「真实代码」口径，
//! python 载具取「原始文本」最严口径，两层互补。

use std::path::PathBuf;

fn app_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 读取 src/ 下相对路径文件；缺失即失败（守卫不得因文件消失而静默放行）。
fn read_src(rel: &str) -> String {
    let p = app_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读取 {rel} 失败: {e}"))
}

/// 列目录下的 `.rs` 文件名（升序，含 mod.rs）。
fn rs_files(dir: &str) -> Vec<String> {
    let d = app_root().join(dir);
    let mut v: Vec<String> = std::fs::read_dir(&d)
        .unwrap_or_else(|e| panic!("读取目录 {dir} 失败: {e}"))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "rs").unwrap_or(false))
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    v.sort();
    v
}

/// task-engine 模块文件集合（与 scan_easyvibe.py 的 MODS['task-engine'] 保持一致）：
/// 主文件 + task_exec/ 子模块（拆分为 prompt/review/harness/changes/contract 后必须全覆盖，
/// 否则子模块里的 crate:: 反向引用会被漏报）。
fn task_engine_files() -> Vec<String> {
    let mut v = vec!["src/task_exec.rs".to_string()];
    for f in rs_files("src/task_exec") {
        v.push(format!("src/task_exec/{f}"));
    }
    v.sort();
    v
}

/// 逆向引用禁止子串（server-api / 同 crate 应用层符号）。
/// c-arch-7 R3 加固：端口适配器落组合根后，`crate::db_ports` 与新增三个 service 域
/// 一并入表——task_exec 只能经切片内端口取能力，不得反向引用组合根/编排层。
const FORBIDDEN: &[&str] = &[
    "crate::BusEvent",
    "crate::publish",
    "crate::agent_conf",
    "crate::AppState",
    "crate::AppError",
    // c-arch-13 R2（ΔS4）：map.rs 按流程拆分后，旧锚点失效，改指**新锚点集合**
    // （service/{patrol,reinduce_start,submap}.rs；与 module_size_guard.rs::TASK_ENGINE_FORBIDDEN 逐字一致）
    "crate::service::patrol::start_",
    "crate::service::reinduce_start::start_",
    "crate::service::submap::analyze_submap",
    "crate::session_queue",
    "crate::db_ports",
    "crate::service::sessions",
    "crate::service::settings",
    "crate::service::agent",
    // c-arch-10：切片不得反向引用装配格（装配 → 切片 单向）
    "crate::assembly",
];

#[test]
fn task_engine_has_no_reverse_crate_refs() {
    for f in task_engine_files() {
        let f = f.as_str();
        let txt = read_src(f);
        for bad in FORBIDDEN {
            assert!(
                !txt.contains(bad),
                "{f} 出现逆向引用 `{bad}`——server-api ↔ task-engine 环复发（R8 守卫）"
            );
        }
    }
}

// ============================================================================
// c-arch-7：app 层 DB 边界（routes/** 与 task_exec 生产不得直连 easyvibe_db）
// ============================================================================

/// 禁用标识符（覆盖 `use easyvibe_db::…` 与内联 `easyvibe_db::X`）。
const DB_BANNED: &str = "easyvibe_db";

/// 反绕过（R5a）：`use easyvibe_db as <alias>` 再导出——堵 `crate::db::SqliteTaskRepository` 式逃逸。
/// 归一化空白后子串匹配；不误伤 `use easyvibe_db::{TaskRepository as _};`（归一化后不含 `easyvibe_db as `）。
const ALIAS_REEXPORT: &str = "use easyvibe_db as ";

/// task_exec **生产**文件集（显式冻结，禁 glob——防漏防多）。
const TASK_ENGINE_PROD: &[&str] = &[
    "src/task_exec.rs",
    "src/task_exec/ports.rs",
    "src/task_exec/changes.rs",
    "src/task_exec/contract.rs",
    "src/task_exec/harness.rs",
    "src/task_exec/prompt.rs",
    "src/task_exec/review.rs",
];

/// task_exec **测试夹具**豁免集（显式冻结；磁盘 `src/task_exec/*.rs` 必须 == PROD ∪ EXEMPT）。
/// 豁免理由（c-arch-15 订正）：这些文件仅 `#[cfg(test)]` 编译，持久化改经**切片内内存端口**
/// （`task_exec::ports` 的 `InMemoryTaskStore` / `InMemoryApprovalStore`），不再直连任何具体仓储，
/// 故本清单的作用是「文件集双向全等」的登记面，而**不是** DB 直连豁免面（预算已归零）。
/// 关于 `try_advance_gate`：其**单线程原子语义**由 N27 用例
/// （`tests_flow::decide_is_atomic_against_double_submit`：双击第二发必 409、留痕恰 1 条）
/// 经生产路径（`decide` → `TaskStore::try_advance_gate`）间接覆盖，内存 double 已按同一条件
/// UPDATE（`status='awaiting_approval' AND gate IS ?` + 影响行数 0/1，同一把锁内 check-then-act）
/// 忠实复刻；**真多线程并发**至今仍无覆盖，属另立需求，本条不承担。
const TASK_ENGINE_TEST_EXEMPT: &[&str] = &[
    "src/task_exec/test_util.rs",
    "src/task_exec/tests_changes.rs",
    "src/task_exec/tests_contract.rs",
    "src/task_exec/tests_flow.rs",
    "src/task_exec/tests_gates.rs",
    "src/task_exec/tests_harness.rs",
    "src/task_exec/tests_prompt.rs",
];

/// 豁免集 `easyvibe_db` 出现次数预算（棘轮：只降不升）。
/// c-arch-15：测试夹具改走切片内内存端口后实测为 0——**本棘轮已归零**，任何测试面直连
/// 具体仓储即红（含注释/字符串口径，去注释前的最严判据见 task_engine_test_exempt_budget_only_shrinks）。
const TASK_ENGINE_TEST_BUDGET: usize = 0;

/// routes handler 冻结规则（R1）：handler 只做 HTTP 边界，不得直取组合根仓储句柄。
/// 命中即红——跨域读写必须经 `crate::service::<域>`。
const ROUTES_REPO_FIELDS: &[&str] = &[
    "st.task_repo",
    "st.approval_repo",
    "st.conversation_repo",
    "st.event_repo",
    "st.health_repo",
    "st.settings_repo",
    "st.agent_session_repo",
    "st.session_output_repo",
];

/// 单遍状态机去注释（**禁止**逐行去 `//` 的简易实现——本仓曾因盲区致 30 处逃逸）。
/// 状态：Code / Line / Block / Str / RawStr(hashes)；字符串内容**保留**（从严：字符串里
/// 出现 `easyvibe_db` 亦可疑），仅剔除注释。
fn strip_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0usize;
    #[derive(PartialEq)]
    enum S {
        Code,
        Line,
        Block,
        Str,
        Raw(usize),
    }
    let mut st = S::Code;
    while i < b.len() {
        let c = b[i];
        let n = if i + 1 < b.len() { b[i + 1] } else { 0 };
        match st {
            S::Code => {
                if c == b'/' && n == b'/' {
                    st = S::Line;
                    i += 2;
                    continue;
                }
                if c == b'/' && n == b'*' {
                    st = S::Block;
                    i += 2;
                    continue;
                }
                // r"…" / r#"…"# / br#"…"#：原样保留内容
                if c == b'r' || (c == b'b' && n == b'r') {
                    let base = if c == b'b' { i + 1 } else { i };
                    if b.get(base + 1) == Some(&b'"') || b.get(base + 1) == Some(&b'#') {
                        let mut j = base + 1;
                        let mut hashes = 0usize;
                        while b.get(j) == Some(&b'#') {
                            hashes += 1;
                            j += 1;
                        }
                        if b.get(j) == Some(&b'"') {
                            for k in i..=j {
                                out.push(b[k]);
                            }
                            i = j + 1;
                            st = S::Raw(hashes);
                            continue;
                        }
                    }
                }
                if c == b'"' {
                    st = S::Str;
                }
                out.push(c);
            }
            S::Line => {
                if c == b'\n' {
                    st = S::Code;
                    out.push(c);
                }
            }
            S::Block => {
                if c == b'*' && n == b'/' {
                    st = S::Code;
                    i += 2;
                    continue;
                }
            }
            S::Str => {
                out.push(c);
                if c == b'\\' && i + 1 < b.len() {
                    out.push(n);
                    i += 2;
                    continue;
                }
                if c == b'"' {
                    st = S::Code;
                }
            }
            S::Raw(hashes) => {
                out.push(c);
                if c == b'"' {
                    let mut ok = true;
                    for k in 1..=hashes {
                        if b.get(i + k) != Some(&b'#') {
                            ok = false;
                            break;
                        }
                    }
                    if ok {
                        for k in 1..=hashes {
                            out.push(b[i + k]);
                        }
                        i += hashes + 1;
                        st = S::Code;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 空白归一化（仅用于别名再导出判据，避免 `use easyvibe_db  as  db` 绕过）。
fn norm_ws(code: &str) -> String {
    code.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 判据本体（供断言与负例自证复用）：合成文本 → 是否命中禁用标识符 / 别名再导出。
fn detect_db_literal(text: &str) -> bool {
    strip_comments(text).contains(DB_BANNED)
}

fn detect_alias_reexport(text: &str) -> bool {
    norm_ws(&strip_comments(text)).contains(ALIAS_REEXPORT)
}

/// R4-①：routes/** 与 task_exec 生产文件去注释后零 `easyvibe_db`；
/// R4-②：routes handler 不得直取 `st.<repo>`；
/// R4-③：task_exec 磁盘文件集 == 生产 ∪ 豁免（双向全等——新增 tests_x.rs 不登记即红）。
#[test]
fn db_direct_access_is_banned_in_routes_and_task_engine_prod() {
    // ① routes/**（含 mod.rs；routes 无测试文件，故无豁免）
    for f in rs_files("src/routes") {
        let rel = format!("src/routes/{f}");
        let code = strip_comments(&read_src(&rel));
        assert!(
            !code.contains(DB_BANNED),
            "{rel} 直连 `{DB_BANNED}`——跨域写库须经 crate::service（c-arch-7 R1/R4 判据）"
        );
        for h in ROUTES_REPO_FIELDS {
            assert!(
                !code.contains(h),
                "{rel} 直取组合根仓储句柄 `{h}`——handler 只做 HTTP 边界，编排须落 crate::service（R1 冻结规则）"
            );
        }
    }
    // ② task_exec 生产文件
    for rel in TASK_ENGINE_PROD {
        let code = strip_comments(&read_src(rel));
        assert!(
            !code.contains(DB_BANNED),
            "{rel} 直连 `{DB_BANNED}`——task-engine 持久化须经切片内端口 task_exec::ports（c-arch-7 R2/R4 判据）"
        );
    }
    // ③ 文件集双向全等
    let mut disk: Vec<String> = vec!["src/task_exec.rs".to_string()];
    disk.extend(rs_files("src/task_exec").into_iter().map(|f| format!("src/task_exec/{f}")));
    let mut declared: Vec<String> =
        TASK_ENGINE_PROD.iter().chain(TASK_ENGINE_TEST_EXEMPT.iter()).map(|s| s.to_string()).collect();
    disk.sort();
    declared.sort();
    assert_eq!(
        disk, declared,
        "task_exec 文件集漂移——新增/删除/改名须同步 TASK_ENGINE_PROD / TASK_ENGINE_TEST_EXEMPT（豁免须显式登记）"
    );
}

/// R4-④：豁免棘轮（只降不升）——实测值超预算即红，倒逼测试夹具改造或显式登记复核。
#[test]
fn task_engine_test_exempt_budget_only_shrinks() {
    let mut used = 0usize;
    for rel in TASK_ENGINE_TEST_EXEMPT {
        used += strip_comments(&read_src(rel)).matches(DB_BANNED).count();
    }
    assert!(
        used <= TASK_ENGINE_TEST_BUDGET,
        "task_exec 测试夹具 `{DB_BANNED}` 出现 {used} 次 > 预算 {TASK_ENGINE_TEST_BUDGET}——棘轮只降不升（c-arch-7 R4）"
    );
}

/// R5a：`src/**` 任何文件不得 `use easyvibe_db as <alias>` 再导出（堵别名绕过）。
/// 逐目录递归扫描（组合根/边界层一视同仁；正当用法 `use easyvibe_db::{A, B};` 不受影响）。
#[test]
fn no_db_alias_reexport_anywhere_in_src() {
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&app_root().join("src"), &mut files);
    files.sort();
    for p in files {
        let rel = p.strip_prefix(app_root()).unwrap().to_string_lossy().replace('\\', "/");
        let txt = std::fs::read_to_string(&p).unwrap();
        assert!(
            !detect_alias_reexport(&txt),
            "{rel} 出现 `use easyvibe_db as <别名>` 再导出——禁 DB 边界判据可被 `crate::<别名>::X` 绕过（c-arch-7 R5a）"
        );
    }
}

/// R5b：判据负例自证（合成文本，不读仓库）——证明「注入必红、注释不误伤、别名可判、正当用法不误伤」。
#[test]
fn db_boundary_guard_selfcheck() {
    let cases: &[(&str, bool, &str)] = &[
        ("use easyvibe_db::TaskRepository;", true, "N1 use 行必红"),
        ("let x = easyvibe_db::SqliteTaskRepository::new(pool);", true, "N1b 内联限定路径必红"),
        ("// use easyvibe_db::TaskRepository;\n", false, "N2 行注释剥离后不误伤"),
        ("/* easyvibe_db */ let x = 1;", false, "N2b 块注释剥离后不误伤"),
        ("let s = \"easyvibe_db\";", true, "N3 字符串保留（从严）"),
        ("let s = r#\"easyvibe_db\"#;", true, "N3b 原始字符串保留（从严）"),
        ("pub(crate) fn f() {}\n", false, "N4 干净文本必绿"),
    ];
    for (text, expect, why) in cases {
        assert_eq!(detect_db_literal(text), *expect, "自证失败（{why}）: {text:?}");
    }
    let alias_cases: &[(&str, bool, &str)] = &[
        ("pub(crate) use easyvibe_db as db;", true, "A1 别名再导出必红"),
        ("pub(crate) use easyvibe_db  as  db;", true, "A1b 空白变体必红"),
        ("use easyvibe_db::{TaskRepository as _};", false, "A2 `as _` 非再导出，不得误伤"),
        ("pub use easyvibe_db::{TaskRepository, TaskRow};", false, "A3 具名导入不得误伤"),
        ("// use easyvibe_db as db;\n", false, "A4 注释里的别名不得误伤"),
    ];
    for (text, expect, why) in alias_cases {
        assert_eq!(detect_alias_reexport(text), *expect, "别名自证失败（{why}）: {text:?}");
    }
}

// ============================================================================
// c-arch-10：service/** 不得直连 easyvibe_db（端口化）+ 装配格单向
// ============================================================================

/// service/** 生产文件 `easyvibe_db` 出现次数棘轮（只降不升；端口化后为 0）。
/// 与 `check_app_service_boundary.py`（CI 载体）**同源**：本常量是单一事实源。
const SERVICE_DB_RATCHET: usize = 0;

/// service/** 不得出现的具体仓储类型名——换名直连 = 端口未真正收窄（c-arch-10 R4）。
const SERVICE_BANNED_TYPES: &[&str] = &[
    "SqliteTaskRepository",
    "SqliteSettingsRepository",
    "SqliteApprovalRepository",
    "SqliteConversationRepository",
    "SqliteEventRepository",
    "SqliteHealthRepository",
    "AgentSessionRepo",
    "SessionOutputRepo",
    "TaskRow",
    "SettingRow",
    "ConversationRow",
    "ConversationMessageRow",
];

/// 非装配格文件不得反向引用装配格（唯一合法声明是 main.rs 的 `mod assembly;`，无 `crate::` 路径）。
const ASSEMBLY_REVERSE: &str = "crate::assembly";

// ============================================================================
// c-arch-13：组合根直连集中度（焦点面 = `src/db_ports.rs` 或 `src/db_ports/**` + `src/state.rs`）
// ----------------------------------------------------------------------------
// 直连面没有消失，只是换了宿主：`db_ports.rs`（513 行 / 80 处）按端口域目录化后，若不再把
// 三量钉死，下一轮必然以同样方式复现。故 I11 由「单条上界」升级为**守恒律 + 双边全等**三条
// 子判据（ΔS-Q11）：① 总处数守恒（防搬运丢行/重复/新写直连）；② 单文件最大 == 登记值；
// ③ 有直连的落点文件数 == 登记值。
// python3 CI 载体 `scripts/check_app_db_boundary.py` 解析下列常量表为单一事实源（fail-closed）。
// ============================================================================

/// 直连计数口径字面量（含 `::`，与 `grep -c "easyvibe_db::"` 真值同源；注释中的回指亦计入，
/// 故 `db_ports/repo.rs` = 3）。**不**做去注释——去注释会使总数跌至 90 与登记值 91 冲突。
const DB_DIRECT_LITERAL: &str = "easyvibe_db::";

/// 焦点面 `easyvibe_db::` 出现**总处数**（守恒律：丢行 / 重复搬 / 新写直连即红）。
const DB_DIRECT_TOTAL: usize = 91;

/// 焦点面**单文件最大**直连数（双边全等：登记值必须 == 实测最大值，不是 `<=`）。
/// c-arch-16 R7：`task_engine.rs` 按 4 端口域拆分后，单文件最大 22 → **15**（`conversation.rs` 15
/// 反超，`task_engine.rs` 降至 14）；总数守恒仍 91（I11a 不变），落点文件数 10 → 13。
const DB_DIRECT_FOCUS_MAX: usize = 15;

/// 焦点面**有直连的落点文件数**（双边全等）。
/// c-arch-16 R7：db_ports 拆 4（10 → 13 文件）。登记 13（13 为本次登记值，只允许在拆分方向变化）。
const DB_DIRECT_FOCUS_FILES_MAX: usize = 13;

/// c-arch-10 R4：service/** 去注释后零 `easyvibe_db`、零具体仓储类型名。
#[test]
fn service_layer_has_no_db_direct_access() {
    let mut used = 0usize;
    for f in rs_files("src/service") {
        let rel = format!("src/service/{f}");
        let code = strip_comments(&read_src(&rel));
        used += code.matches(DB_BANNED).count();
        for t in SERVICE_BANNED_TYPES {
            assert!(
                !code.contains(t),
                "{rel} 出现具体仓储类型名 `{t}`——service 须只见 crate::db_ports 端口与本地 DTO"
            );
        }
    }
    assert!(
        used <= SERVICE_DB_RATCHET,
        "service/** 直连 `{DB_BANNED}` 出现 {used} 次 > 棘轮 {SERVICE_DB_RATCHET}——编排层须经 crate::db_ports（c-arch-10 R4）"
    );
}

/// c-arch-13 I11a/b/c：焦点面直连三量**双边全等**（守恒律 / 单文件最大 / 落点文件数）。
/// 兼容目录化前后：磁盘存在 `src/db_ports/` 取目录，否则取 `src/db_ports.rs`（以磁盘实际为准）。
#[test]
fn db_direct_focus_stays_bounded() {
    let mut by: Vec<(String, usize)> = Vec::new();
    let dir = app_root().join("src/db_ports");
    if dir.is_dir() {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "rs").unwrap_or(false))
            .collect();
        files.sort();
        for p in files {
            let rel = p.strip_prefix(app_root()).unwrap().to_string_lossy().replace('\\', "/");
            let n = std::fs::read_to_string(&p).unwrap().matches(DB_DIRECT_LITERAL).count();
            by.push((rel, n));
        }
    } else {
        by.push(("src/db_ports.rs".to_string(), read_src("src/db_ports.rs").matches(DB_DIRECT_LITERAL).count()));
    }
    by.push(("src/state.rs".to_string(), read_src("src/state.rs").matches(DB_DIRECT_LITERAL).count()));

    let total: usize = by.iter().map(|(_, n)| *n).sum();
    let max = by.iter().map(|(_, n)| *n).max().unwrap_or(0);
    let with_direct = by.iter().filter(|(_, n)| *n > 0).count();
    let table: Vec<String> = by.iter().map(|(f, n)| format!("{f}={n}")).collect();
    assert_eq!(
        total, DB_DIRECT_TOTAL,
        "焦点面直连总处数 {total} != {DB_DIRECT_TOTAL}（I11a 守恒律破：搬运丢行/重复/新写直连）\n{table:?}"
    );
    assert_eq!(
        max, DB_DIRECT_FOCUS_MAX,
        "焦点面单文件最大直连数 {max} != {DB_DIRECT_FOCUS_MAX}（I11b 双边全等，须同步登记）\n{table:?}"
    );
    assert_eq!(
        with_direct, DB_DIRECT_FOCUS_FILES_MAX,
        "焦点面有直连的文件数 {with_direct} != {DB_DIRECT_FOCUS_FILES_MAX}（I11c 落点面漂移）\n{table:?}"
    );
}

/// c-arch-10 R5②：非装配格 `src/**` 不得出现 `crate::assembly`（装配 → server-api 单向）。
#[test]
fn only_main_declares_assembly_module() {
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&app_root().join("src"), &mut files);
    files.sort();
    for p in files {
        let rel = p.strip_prefix(app_root()).unwrap().to_string_lossy().replace('\\', "/");
        if rel.starts_with("src/assembly/") || rel == "src/main.rs" {
            continue; // 装配格自身 + main.rs 的 `mod assembly;` 是合法声明
        }
        let code = strip_comments(&std::fs::read_to_string(&p).unwrap());
        assert!(
            !code.contains(ASSEMBLY_REVERSE),
            "{rel} 出现 `{ASSEMBLY_REVERSE}`——server-api 反向引用装配格（c-arch-10 R5）"
        );
    }
}

// ============================================================================
// c-arch-16 R6：db_ports/** 单端口域断言（「只依赖本域仓储方法」）
// ----------------------------------------------------------------------------
// 背景：db_ports 目录化只把直连面从 1 个 513 行文件搬到 13 个适配器文件（总处数守恒 91），
// 「任何领域改动都要先读懂」的单点并未消除，只是粒度变了。缺一条**禁止适配器跨域**的断言。
// 本表 = 每个 `db_ports/*.rs` 允许出现的 `easyvibe_db::<符号>` 集合（**显式登记、禁 glob、
// 与磁盘文件集双向全等**）。CI 镜像 `scripts/check_app_db_boundary.py` 解析本常量
// （单一事实源，零双写；解析失败 fail-closed）。
//
// 口径边界（诚实记录）：本断言**只**约束 `easyvibe_db::` 前缀符号，**不**约束
// `easyvibe_ai_agent::` 等其它域引用——如 `agent_slot.rs` 引
// `easyvibe_ai_agent::agent_conf::resolve_agent`（**非** easyvibe_db，不在本断言面内；
// 其保留理由见 c-arch-16 §R1 的 agent-runtime 裁决）。默认**不**加「单文件只可出现一个
// `impl …Port for`」的结构判据（会误伤 dto.rs / mod.rs）。
// ============================================================================

/// db_ports 单端口域归属（显式登记；键集必须 == 磁盘 `src/db_ports/*.rs`，双向全等）。
const DB_PORTS_DOMAIN_OWNERSHIP: &[(&str, &[&str])] = &[
    // 纯 mod 声明 + pub(crate) use 再导出：零 easyvibe_db 符号。
    ("mod.rs", &[]),
    // DTO 文件（本地 DTO 五型 + to_message_rows）：只承载会话消息 DTO 的转换，
    // 唯一触及的仓储行类型 = ConversationMessageRow（并非跨域适配器）。
    ("dto.rs", &["ConversationMessageRow"]),
    ("repo.rs", &["wipe_repo", "sqlx"]),
    ("task.rs", &["TaskRepository", "TaskRow", "SqliteTaskRepository"]),
    ("settings.rs", &["SettingsRepository", "SettingRow", "SqliteSettingsRepository"]),
    ("approval.rs", &["ApprovalRepository", "ApprovalRow", "SqliteApprovalRepository"]),
    (
        "conversation.rs",
        &[
            "ConversationRepository",
            "ConversationRow",
            "ConversationMessageRow",
            "SqliteConversationRepository",
        ],
    ),
    (
        "health.rs",
        &["HealthRepository", "ModuleHealthRow", "RunModuleAvg", "PatrolRunRow", "SqliteHealthRepository"],
    ),
    ("event.rs", &["EventRepository", "EventCountRow", "SqliteEventRepository"]),
    // c-arch-16 R7：task_engine.rs 按端口域拆 4（以下四行各自单域，合计 22 处守恒）。
    ("task_engine.rs", &["TaskRepository", "TaskRow", "SqliteTaskRepository"]),
    ("task_engine_approval.rs", &["ApprovalRepository", "ApprovalRow", "SqliteApprovalRepository"]),
    ("session_attribution.rs", &["AgentSessionRepo"]),
    ("agent_slot.rs", &["SqliteSettingsRepository"]),
];

/// 抓取源码中 `easyvibe_db::<符号>` 的符号名（去重升序）。
fn easyvibe_db_symbols(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find(DB_DIRECT_LITERAL) {
        let after = &rest[pos + DB_DIRECT_LITERAL.len()..];
        let sym: String =
            after.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if !sym.is_empty() {
            out.push(sym);
        }
        rest = after;
    }
    out.sort();
    out.dedup();
    out
}

/// c-arch-16 R6：db_ports/** 单端口域断言——磁盘文件集与冻结表**双向全等**，且每个文件中出现的
/// `easyvibe_db::<符号>` 必须落在该文件的允许集内（跨域巨型适配器复发即红）。
#[test]
fn db_ports_files_only_reach_their_own_domain_symbols() {
    let mut declared: Vec<String> =
        DB_PORTS_DOMAIN_OWNERSHIP.iter().map(|(f, _)| f.to_string()).collect();
    declared.sort();
    let disk = rs_files("src/db_ports");
    assert_eq!(
        disk, declared,
        "db_ports 文件集与 DB_PORTS_DOMAIN_OWNERSHIP 不一致（双向全等；新增/删除/改名须同步登记）"
    );
    for (file, allowed) in DB_PORTS_DOMAIN_OWNERSHIP {
        let text = read_src(&format!("src/db_ports/{file}"));
        for sym in easyvibe_db_symbols(&text) {
            assert!(
                allowed.contains(&sym.as_str()),
                "db_ports/{file} 出现跨域符号 `easyvibe_db::{sym}`——单端口域断言破（c-arch-16 R6；允许集 {allowed:?}）"
            );
        }
    }
}
