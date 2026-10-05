//! 出厂资产编译期内嵌（提示词/schema/dist）与文本资产解析链。

use crate::state::data_dir;

// 受管资产名册由 build.rs 从 `scripts/assets.json`（唯一事实源）派生：
// 常量 MAP_PROMPT / PATROL_PROMPT / MAP_SCHEMA / SUBMAP_PROMPT / INCREMENTAL_PROMPT
// 与 SPEC 名册均在此展开。改提示词名 / env 名只改名录，编译期自动跟随。
// 本文件不再出现任何契约文件名或 env 名字面量（由 scripts/verify_assets.py --forbid-literals 守卫）。
include!(concat!(env!("OUT_DIR"), "/text_assets_gen.rs"));

/// 按 role（见 `scripts/assets.json`）取受管资产条目。
pub(crate) fn spec(role: &str) -> &'static AssetSpec {
    SPEC.iter()
        .find(|s| s.role == role)
        .unwrap_or_else(|| panic!("资产 role 未在名录中: {role}"))
}

include!(concat!(env!("OUT_DIR"), "/embedded_dist.rs"));

/// 独立形态（内嵌 UI 可用且未指定外部静态目录）下为 true：启动后自动打开浏览器
pub(crate) fn embedded_ui_available() -> bool {
    EMBEDDED_UI_REAL
}

/// 文本资产解析链：env 显式路径 → exe 旁路文件 → 当前目录 → 编译期内嵌。
/// 内嵌命中且 persist 时落盘到数据目录 assets/（schema 需以真实路径交给外部 agent 读取）。
/// 返回 (内容, 实际来源路径)。
pub(crate) fn resolve_text_asset(env_var: &str, filename: &str, embedded: &str, persist: bool) -> (String, String) {
    if let Ok(p) = std::env::var(env_var) {
        if !p.is_empty() {
            match std::fs::read_to_string(&p) {
                Ok(s) => return (s, p),
                Err(e) => eprintln!("[assets] {env_var}={p} 不可读（{e}），回落旁路/内嵌副本"),
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(filename);
            if let Ok(s) = std::fs::read_to_string(&p) {
                return (s, p.to_string_lossy().into_owned());
            }
        }
    }
    if let Ok(s) = std::fs::read_to_string(filename) {
        return (s, filename.into());
    }
    if persist {
        let dir = data_dir().join("assets");
        if std::fs::create_dir_all(&dir).is_ok() {
            let p = dir.join(filename);
            if !p.exists() {
                let _ = std::fs::write(&p, embedded);
            }
            return (embedded.to_string(), p.to_string_lossy().into_owned());
        }
    }
    (embedded.to_string(), format!("<embedded:{filename}>"))
}
