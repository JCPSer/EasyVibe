//! R3/R4/R5 契约面守卫（c-arch-1 双枢纽收敛）。
//!
//! 与 `module_size_guard.rs` / `arch_guard.rs` 同范式：只读源码文本（`std::fs` +
//! `CARGO_MANIFEST_DIR`），不 import 符号（本 crate 为 bin-only），毫秒级进 `cargo test`。
//!
//! 断言组：
//!   R5-a WS 事件名清单（`easyvibe-api-types::WS_EVENTS`）== 冻结 10 类；
//!   R5-b 每个事件名在 ws.rs 翻译层出现（防「清单登记了、翻译层漏了/改名了」）；
//!   R4   /api 响应版本头已接线（router.rs 注入 `x-easyvibe-api-version`，复用 VERSION）；
//!   R3-a REST DTO 单一事实源存在（api-types 承载资源 DTO）；
//!   R3-b 内联 `json!` 响应体**棘轮**：按文件冻结计数，只降不升（契约面收敛趋势可验证）。

use std::path::{Path, PathBuf};

fn app_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// easyvibe-api-types 的 lib.rs（同 workspace，相对 app crate 的上两级）。
fn api_types_lib() -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../easyvibe-api-types/src/lib.rs");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读取 api-types lib.rs 失败: {e}"))
}

fn read_app(rel: &str) -> String {
    let p = app_src().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读取 {rel} 失败: {e}"))
}

fn count_occurrences(txt: &str, needle: &str) -> usize {
    txt.matches(needle).count()
}

/// 抓取 `pub const WS_EVENTS: &[&str] = &[ ... ];` 中的字符串字面量。
fn parse_ws_events(txt: &str) -> Vec<String> {
    let start = txt.find("pub const WS_EVENTS").expect("api-types 缺 WS_EVENTS 清单（R5）");
    // 跳过 `: &[&str] = `，从真正的数组字面量 `&[` 起算
    let eq = txt[start..].find("= &[").expect("WS_EVENTS 缺 `= &[` 数组字面量") + start;
    let open = eq + "= ".len();
    let close = txt[open..].find(']').expect("WS_EVENTS 数组未闭合") + open;
    let seg = &txt[open..close];
    seg.split('"')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, s)| s.to_string())
        .collect()
}

/// easyvibe-common 的 `events` 常量表（`pub const NAME: &str = "value";`）→ name→value。
fn common_event_consts() -> Vec<(String, String)> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../easyvibe-common/src/lib.rs");
    let txt = std::fs::read_to_string(&p).expect("读取 easyvibe-common lib.rs 失败");
    let mut out = Vec::new();
    for line in txt.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("pub const ") {
            if let Some((name, val)) = rest.split_once(": &str = ") {
                let name = name.trim().to_string();
                let val = val.trim().trim_end_matches(';').trim().trim_matches('"').to_string();
                out.push((name, val));
            }
        }
    }
    out
}

/// 冻结的 WS 事件名（与前端 `repoLayout.test.ts::FROZEN_WS_EVENTS` 全等）。
const FROZEN_WS_EVENTS: &[&str] = &[
    "map.changed",
    "growth.event",
    "session.statusChanged",
    "queue.changed",
    "session.output",
    "patrol.finished",
    "freshness.changed",
    "task.contractAlert",
    "task.contractViolated",
    "task.statusChanged",
];

#[test]
fn ws_event_names_are_frozen_in_contract_crate() {
    let mut declared = parse_ws_events(&api_types_lib());
    declared.sort();
    let mut frozen: Vec<String> = FROZEN_WS_EVENTS.iter().map(|s| s.to_string()).collect();
    frozen.sort();
    assert_eq!(declared, frozen, "easyvibe-api-types::WS_EVENTS 与冻结清单漂移（R5 单一事实源）");
}

#[test]
fn ws_translate_layer_covers_every_registered_event() {
    let mut ws = read_app("ws.rs");
    // 把 `ev::CONST` / `easyvibe_common::events::CONST` 还原成字符串字面量：
    // translate 层部分事件名经常量引用，改名须在两侧同步，故解引用后再断言。
    for (name, val) in common_event_consts() {
        ws = ws.replace(&format!("ev::{name}"), &format!("\"{val}\""));
        ws = ws.replace(&format!("events::{name}"), &format!("\"{val}\""));
    }
    for name in FROZEN_WS_EVENTS {
        assert!(
            ws.contains(&format!("\"{name}\"")),
            "ws.rs 翻译层缺少或改名了已登记事件 `{name}`——契约漂移（R5-b）"
        );
    }
}

#[test]
fn api_version_header_is_wired() {
    let router = read_app("router.rs");
    assert!(
        router.contains("x-easyvibe-api-version"),
        "router.rs 未注入版本响应头 x-easyvibe-api-version（R4 版本锚缺失）"
    );
    assert!(
        router.contains("VERSION"),
        "版本响应头须复用 crate::VERSION（勿另造第二套版本语义，R4）"
    );
}

#[test]
fn rest_dto_single_source_exists() {
    let lib = api_types_lib();
    // 资源 DTO + WS payload 结构必须落在契约 crate（R3-a）
    for needed in [
        "pub struct RepoInfo",
        "pub struct HealthResponse",
        "pub struct TaskActionResult",
        "pub struct TaskStatusChanged",
        "pub struct SessionOutput",
        "pub struct PatrolFinished",
        "pub struct FreshnessChanged",
        "pub struct QueueChanged",
    ] {
        assert!(lib.contains(needed), "easyvibe-api-types 缺 `{needed}`（R3 REST/WS DTO 单一事实源）");
    }
}

/// R3-b 内联 `json!` 响应体棘轮：文件 → 冻结出现次数（只降不升）。
/// 新增响应须走命名 DTO（api-types）；本表是「契约面收敛」趋势的可验证度量——
/// 清干净后须下调，新增文件不得蒙混（未登记即失败）。
const JSON_RATCHET: &[(&str, usize)] = &[
    ("routes/chat.rs", 5),
    ("routes/dev_docs.rs", 2),
    ("routes/map.rs", 3),
    // c-arch-7 R1：route 内联响应映射大部下移至 crate::service（settings/task/agent/sessions）；
    // 路由侧预算随之下降，service 侧按迁移后实测登记——**聚合只降不升**（迁移前 83 → 迁移后 81）。
    ("routes/sessions.rs", 1),
    ("routes/settings.rs", 11),
    ("routes/task.rs", 1),
    ("service/agent.rs", 7),
    ("service/chat.rs", 14),
    // c-arch-13 R2：service/map.rs 按流程拆分，内联 json! 随函数原样搬迁（聚合 7 不变，只改落点）。
    ("service/map.rs", 3),
    ("service/patrol.rs", 2),
    ("service/reinduce_start.rs", 1),
    ("service/submap.rs", 1),
    ("service/sessions.rs", 9),
    ("service/settings.rs", 8),
    ("service/task.rs", 13),
];

fn rs_files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "rs").unwrap_or(false))
        .filter(|p| p.file_name().unwrap() != "mod.rs")
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    v.sort();
    v
}

#[test]
fn inline_json_response_ratchet_only_shrinks() {
    for sub in ["routes", "service"] {
        for fname in rs_files(&app_src().join(sub)) {
            let rel = format!("{sub}/{fname}");
            let n = count_occurrences(&read_app(&rel), "json!");
            match JSON_RATCHET.iter().find(|(f, _)| *f == rel) {
                Some((_, budget)) => assert!(
                    n <= *budget,
                    "{rel} 内联 json! 从 {budget} 增至 {n}——新增响应须走 easyvibe-api-types 命名 DTO（R3 棘轮只降不升）"
                ),
                None => assert_eq!(
                    n, 0,
                    "{rel} 出现 {n} 处内联 json!——新文件不得引入未登记的裸 json! 响应（R3 棘轮）"
                ),
            }
        }
    }
}
