//! R10③（c-arch-1）：easyvibe-map 防回胀 / 方向守卫——文件集快照 + 单文件 LOC + 禁用依赖面 + 导出符号冻结。
//!
//! 与 `easyvibe-app` / `easyvibe-git` / `easyvibe-pipeline` 的 `module_size_guard.rs` **同范式**：
//! 只读源码文本（含注释），不 import 任何 crate 符号，零 dev-dependencies。
//!
//! 为什么需要它：c-arch-1 把 freshness / induction / concerns 三个领域实现外提到本 crate，
//! 但本 crate 此前**只有纪律、没有断言**（地图文案已宣称「自带守卫」而事实为无，属无守卫区）。
//! 断言组：
//!   1. 文件集双向全等——多一个=未登记的新子模块，少一个=误删（拆分须显式登记，见下）；
//!   2. 单文件 LOC 上限——拆完又长即红（`synthesis.rs` 余量见 LOC_LIMITS 注释）；
//!   3. 禁用依赖面——领域 crate 不得回流 HTTP / 应用状态 / 持久层 / agent 层；
//!   4. 导出符号冻结——改名 / 删除 / 跨文件迁移必须同步冻结表与调用方。
//!
//! 与 `c-map-domain-1`（拆分 `synthesis.rs`）的关系：拆分**是被鼓励的**，
//! 本守卫的作用只是让拆分**必须显式登记**（FROZEN_FILES 加新文件、LOC_LIMITS 加行、
//! 符号迁移则更新 FROZEN_SYMBOLS）——摩擦是有意的，把「悄悄长回去」变成「评审可见的差异」。
//!
//! CI 侧同源复核：`scripts/check_map_domain_guard.py`（纯 python3）**解析本文件的常量表**
//! 后对 FS 复判同一套判据；解析不到常量即 fail-closed。本文件因此是**单一事实源**，
//! 常量须保持「单行、字面量」写法（见该脚本 --selfcheck N5）。

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

/// 归属快照：文件集迁移（含 `c-map-domain-1` 这类正当拆分）须显式改本表并说明理由。
/// 双向全等：多一个=未登记的新子模块；少一个=误删。
const FROZEN_FILES: &[&str] = &["concerns.rs", "freshness.rs", "induction.rs", "lib.rs", "synthesis.rs"];

/// 禁止出现的依赖面（边界层符号回流即失败；文本扫描**含注释**，防「注释里写 axum」这类绕过）。
const FORBIDDEN: &[&str] = &["axum", "AppState", "easyvibe_app", "easyvibe_db", "easyvibe_ai_agent", "easyvibe_session"];

/// 单文件行数上限（`*` = 未列名文件的默认值）。实测：lib 410 / synthesis 557 / induction 380 /
/// freshness 179 / concerns 175；`synthesis.rs` 距 600 仅 43 行——不因「余量小」调阈值，
/// 而是让拆分必须先改本表（评审可见）。
const LOC_LIMITS: &[(&str, usize)] = &[("lib.rs", 450), ("*", 600)];

/// 导出符号冻结（文本锚；防改名 / 删除造成调用方静默断链）。
/// ①–⑨ 为 c-arch-1 外提产物（R3/R4/R5）；⑩–⑭ 为三新子模块的调用面（Q-D：一并冻结）。
const FROZEN_SYMBOLS: &[&str] = &[
    "pub enum FreshnessStatus",        // ① freshness.rs —— R4 外提产物
    "pub fn assess(",                  // ② freshness.rs —— R4 外提产物
    "pub fn decide(",                  // ③ induction.rs —— R3 外提产物（七道门判定入口）
    "pub fn render_incremental_prompt(", // ④ induction.rs —— R3 外提产物
    "pub fn head_sha(",                // ⑤ induction.rs —— decision_head 单次读的载体
    "pub fn should_advance_anchor(",    // ⑥ induction.rs —— R3 外提产物
    "pub fn extract_concerns(",        // ⑦ concerns.rs —— R5 外提产物
    "pub fn diff_concerns(",           // ⑧ concerns.rs —— R5 外提产物
    "pub fn assign_concern_ids(",      // ⑨ concerns.rs —— R5 外提产物（id 幂等兜底）
    "pub async fn apply_incremental(",  // ⑩ synthesis.rs —— 归纳终态入口（调用面）
    "pub fn owners_of_file",           // ⑪ synthesis.rs —— 归属引擎（调用面）
    "pub fn parse_patch(",             // ⑫ synthesis.rs —— patch 解析（调用面）
    "pub fn synthesize(",              // ⑬ synthesis.rs —— 地图合成（调用面）
    "pub fn validate_strict(",         // ⑭ lib.rs —— 严格校验（调用面）
];

fn loc_limit(f: &str) -> usize {
    LOC_LIMITS
        .iter()
        .find(|(name, _)| *name == f)
        .or_else(|| LOC_LIMITS.iter().find(|(name, _)| *name == "*"))
        .map(|(_, n)| *n)
        .unwrap_or_else(|| panic!("LOC_LIMITS 未覆盖 {f}"))
}

#[test]
fn file_set_is_frozen() {
    let mut expected: Vec<String> = FROZEN_FILES.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(
        rs_files(),
        expected,
        "easyvibe-map/src 文件集漂移——多一个=未登记的新子模块，少一个=误删（R10③；拆分须显式登记）"
    );
}

#[test]
fn files_stay_below_size_limits() {
    for f in rs_files() {
        let n = read(&f).lines().count();
        let lim = loc_limit(&f);
        assert!(n <= lim, "easyvibe-map/src/{f} 超 {lim} 行: {n}（拆完又长即红，R10③）");
    }
}

#[test]
fn no_upward_dependency_faces() {
    for f in rs_files() {
        let txt = read(&f);
        for bad in FORBIDDEN {
            assert!(
                !txt.contains(bad),
                "easyvibe-map/src/{f} 出现禁用依赖面 `{bad}`——领域 crate 不得回流 HTTP/状态/持久层/agent 层（R10③）"
            );
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
        assert!(
            all.contains(s),
            "easyvibe-map 导出符号 `{s}` 缺失——改名/删除/迁移须同步冻结表与调用方（R10③）"
        );
    }
}
