//! 编译期决定内嵌哪个前端产物：
//! - 渲染器 dist 已构建（index.html 存在）→ 内嵌真身（独立分发形态双击 exe 直接出 UI）
//! - 未构建（只编后端的 CI/交叉编译场景）→ 内嵌占位页，保证编译通过；
//!   EMBEDDED_UI_REAL=false，后端不挂内嵌静态服务（运行时行为与旧版一致：只挂 API）
//! 单一事实源是 easyvibe-renderer/dist——改了前端记得 `npm run build` 后重编后端。
fn main() {
    println!("cargo:rerun-if-changed=../../../easyvibe-renderer/dist/index.html");
    let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let real = manifest.join("../../../easyvibe-renderer/dist/index.html").is_file();
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("embedded_dist.rs");
    let code = if real {
        r#"/// 渲染器构建产物内嵌（独立分发形态：后端同源托管 UI）
pub static DIST: ::include_dir::Dir<'static> =
    ::include_dir::include_dir!("$CARGO_MANIFEST_DIR/../../../easyvibe-renderer/dist");
/// 内嵌的是真前端（渲染器已构建）
pub const EMBEDDED_UI_REAL: bool = true;
"#
    } else {
        r#"/// 渲染器未构建——内嵌占位页（只编后端的 CI 场景），运行时不可用
pub static DIST: ::include_dir::Dir<'static> =
    ::include_dir::include_dir!("$CARGO_MANIFEST_DIR/frontend-stub");
/// 内嵌的是占位页（渲染器未构建）
pub const EMBEDDED_UI_REAL: bool = false;
"#
    };
    std::fs::write(&out, code).expect("embedded_dist.rs 写入失败");
}
