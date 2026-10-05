//! 出厂资产编译期内嵌（提示词/schema/dist）与文本资产解析链。

use crate::state::data_dir;

/// 出厂资产编译期内嵌——独立分发形态（双击 exe / 单文件拷到干净机器）下环境变量与旁路文件都不存在，
/// 解析链兜底到这里。单一事实源仍是仓库根的这四个文件：改提示词后重编后端即更新内嵌快照。
/// DIST 由 build.rs 决定内嵌真身还是占位页（渲染器未构建的 CI 场景），见 EMBEDDED_UI_REAL。
pub const MAP_PROMPT: &str = include_str!("../../../../easyvibe-map-prompt-v2.2.md");
pub const PATROL_PROMPT: &str = include_str!("../../../../easyvibe-map-patrol-prompt-v2.md");
pub const MAP_SCHEMA: &str = include_str!("../../../../easyvibe-map-schema-v1.1.json");
pub const SUBMAP_PROMPT: &str = include_str!("../../../../easyvibe-module-submap-prompt.md");
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
