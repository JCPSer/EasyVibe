//! 装配格 · 静态产物托管（独立分发布局）。
//!
//! c-arch-10 R2/ΔS6：自 `router.rs` 尾段**外提**静态回落 + 自 `assets.rs` 移入内嵌 dist。
//! 关键约束：静态回落必须**包在 `build_router` 之外**——否则 `router.rs` 又要 `use crate::assembly`，
//! 产生 `server-api → assembly` 新出边（本层外向边不得上升）。
//! 层序等价性：现状 fallback 亦追加在 CORS/跨站 layer **之后**（layer 不包裹 fallback），
//! 外提后层序不变 ⇒ URL / 状态码 / MIME 逐字等价。

use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

include!(concat!(env!("OUT_DIR"), "/embedded_dist.rs"));

/// 独立形态（内嵌 UI 可用且未指定外部静态目录）下为 true：启动后自动打开浏览器。
pub(crate) fn embedded_ui_available() -> bool {
    EMBEDDED_UI_REAL
}

/// 独立形态判定（未指定 `EASYVIBE_STATIC_DIR` 且内嵌 UI 真实可用）。
pub(crate) fn standalone() -> bool {
    std::env::var("EASYVIBE_STATIC_DIR").ok().filter(|d| !d.is_empty()).is_none() && embedded_ui_available()
}

/// 追加静态回落（三态）：外部静态目录 → 内嵌 dist → 不挂。
/// **必须**在 `build_router`（纯 API/WS 路由，无 fallback）之后调用。
pub(crate) fn attach(router: Router) -> Router {
    // D5：桌面壳同源托管——EASYVIBE_STATIC_DIR 指向渲染器构建产物（dist）时，
    // / 与未命中路径回落到静态资源；前端 fetch('/api/...') 与 /ws 全部同源。
    // 独立分发形态（未指定静态目录）回落到编译期内嵌的 dist——后端自己同源托管 UI。
    match std::env::var("EASYVIBE_STATIC_DIR").ok().filter(|d| !d.is_empty()) {
        Some(dir) => {
            use tower_http::services::ServeDir;
            router.fallback_service(ServeDir::new(dir).append_index_html_on_directories(true))
        }
        None if embedded_ui_available() => router.fallback_service(get(embedded_static)),
        None => router,
    }
}

/// 内嵌 dist 的静态回落（独立形态）：按路径精确查找，未命中回落 index.html（SPA 前端路由）。
pub(crate) async fn embedded_static(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file = DIST
        .get_file(path)
        .or_else(|| DIST.get_file("index.html"));
    match file {
        Some(f) => (
            [(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static(mime_by_ext(path)),
            )],
            f.contents(),
        )
            .into_response(),
        None => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}

pub(crate) fn mime_by_ext(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ico") => "image/x-icon",
        Some("txt") | Some("md") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// 独立形态：打印人类可读横幅并自动打开浏览器（双击 exe 的完整体验）。
pub(crate) fn announce(standalone: bool, addr: &str, data_dir: &std::path::Path) {
    if standalone {
        let url = format!("http://{addr}");
        eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        eprintln!("  EasyVibe 已启动 → {url}");
        eprintln!("  数据目录: {}", data_dir.display());
        eprintln!("  关闭本窗口即退出服务");
        eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        #[cfg(target_os = "windows")]
        std::process::Command::new("cmd").args(["/C", "start", &url]).spawn().ok();
        #[cfg(target_os = "macos")]
        std::process::Command::new("open").arg(&url).spawn().ok();
    }
}

#[cfg(test)]
mod tests {
    use super::mime_by_ext;

    #[test]
    fn mime_by_ext_known_types() {
        assert!(mime_by_ext("index.html").starts_with("text/html"));
        assert!(mime_by_ext("app.js").contains("javascript"));
        assert!(mime_by_ext("app.css").starts_with("text/css"));
        assert_eq!(mime_by_ext("logo.png"), "image/png");
        assert_eq!(mime_by_ext("font.woff2"), "font/woff2");
        assert_eq!(mime_by_ext("data.bin"), "application/octet-stream");
    }
}
