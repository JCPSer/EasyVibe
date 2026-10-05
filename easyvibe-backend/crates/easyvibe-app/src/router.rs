//! 路由表装配：聚合各资源域自注册路由 + 静态回落。
//!
//! R1（c-arch-1 双枢纽收敛）：路由「路径/方法 ↔ handler」对照点已下沉各
//! `routes/<域>.rs` 的 `router()`；本文件退化为 **merge 聚合 + `/ws` + 中间件 + 静态回落**，
//! 不含任何资源域路由（守卫 `module_size_guard.rs` 断言组 D 兜底）。
//!
//! 域 → 路径前缀 → 文件索引（读侧索引，由守卫与各域自注册清单交叉校验）：
//!   repo.rs     `/health` `/repos*` `/repos/{id}/git/*`（git 仅注册，handler 留 crate::git）
//!   map.rs      `/repos/{id}/map|freshness|growth|progress|reinduce|patrol|modules/*`
//!   sessions.rs `/repos/{id}/session-queue|patrol-runs|agent-sessions|usage|health-dashboard|events*|sessions/*` `/sessions/overview`
//!   chat.rs     `/repos/{id}/chat*` `/repos/{id}/conversations*` `/repos/{id}/views*`
//!   task.rs     `/repos/{id}/tasks*` `/repos/{id}/suggest`
//!   dev_docs.rs `/repos/{id}/dev-doc*`
//!   settings.rs `/settings*` `/harness*` `/diagnostics`
//!   agent.rs    `/agent/*` `/llm/test`
//!   本文件      `/ws`
//!
//! R4（版本锚）：`/api/*` 响应统一带只读响应头 `X-EasyVibe-Api-Version`（复用 Y7 的
//! `crate::VERSION`，与 `/api/health` 的 `version` 字段同源，不另造第二套版本语义）。

use axum::{
    response::{IntoResponse, Response},
    routing::get, Router,
};
use crate::state::*;
use crate::VERSION;
use crate::assets;
use crate::assets::embedded_ui_available;
use crate::ws::ws_handler;
use crate::routes::{agent, chat, dev_docs, map, repo, sessions, settings, task};

/// R4：为 `/api/*` 响应注入契约版本头（additive，不改任何 JSON 形状/状态码/ETag）。
async fn api_version_header(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut resp = next.run(req).await;
    resp.headers_mut()
        .insert("x-easyvibe-api-version", axum::http::HeaderValue::from_static(VERSION));
    resp
}

pub fn build_router(state: AppState) -> Router {
    // 各资源域自注册（新增域 = 新增 routes/<new>.rs + 此处 1 行 merge，既有域文件零改动）
    let api = Router::new()
        .merge(repo::router())
        .merge(map::router())
        .merge(sessions::router())
        .merge(chat::router())
        .merge(task::router())
        .merge(dev_docs::router())
        .merge(settings::router())
        .merge(agent::router())
        .layer(axum::middleware::from_fn(api_version_header))
        .with_state(state.clone());

    let router = Router::new()
        .nest("/api", api)
        .route("/ws", get(ws_handler))
        // D5：桌面壳（Tauri v2 WebView）固定源。跨源请求带此 Origin 时放行
        // （CORS 层同时只回这一个源的 Access-Control-Allow-Origin）；
        // evil 页面无法伪造 Origin，Y6 对其它 cross-site 的拦截不受影响。
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(tower_http::cors::AllowOrigin::exact(
                    TAURI_ORIGIN.parse().expect("合法 origin"),
                ))
                .allow_methods([axum::http::Method::GET, axum::http::Method::POST, axum::http::Method::PUT, axum::http::Method::DELETE])
                .allow_headers(tower_http::cors::Any),
        )
        // Y6 清债：跨站请求防护—— evil 页面可对 127.0.0.1 发 simple POST（无 CORS 拦截）。
        // 现代浏览器带 Sec-Fetch-Site 头：cross-site 一律拒（health 放行供连通探测）；
        // 桌面壳 Origin 一律放（WS 握手与 preflight 都带 Origin，无 Sec-Fetch 维度可依赖）。
        .layer(axum::middleware::from_fn(|req: axum::http::Request<axum::body::Body>, next: axum::middleware::Next| async move {
            let (parts, body) = req.into_parts();
            let is_health = parts.uri.path() == "/api/health";
            let is_tauri = parts.headers.get("origin").and_then(|v| v.to_str().ok()) == Some(TAURI_ORIGIN);
            let cross_site = parts.headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) == Some("cross-site");
            if cross_site && !is_health && !is_tauri {
                let mut resp = axum::http::Response::new(axum::body::Body::from("cross-site request blocked"));
                *resp.status_mut() = axum::http::StatusCode::FORBIDDEN;
                return Ok::<_, std::convert::Infallible>(resp);
            }
            Ok::<_, std::convert::Infallible>(next.run(axum::http::Request::from_parts(parts, body)).await)
        }))
        .with_state(state);

    // D5：桌面壳同源托管——EASYVIBE_STATIC_DIR 指向渲染器构建产物（dist）时，
    // / 与未命中路径回落到静态资源；前端 fetch('/api/...') 与 /ws 全部同源，
    // 桌面 WebView 直接加载 http://127.0.0.1:{port}，CORS/跨站问题整体消失。
    // 独立分发形态（未指定静态目录）回落到编译期内嵌的 dist——后端自己同源托管 UI，
    // 双击 exe 直接出界面（2026-10-04：此前裸 exe 只挂 API 无 UI，用户双击"没反应"）。
    match std::env::var("EASYVIBE_STATIC_DIR").ok().filter(|d| !d.is_empty()) {
        Some(dir) => {
            use tower_http::services::ServeDir;
            router.fallback_service(ServeDir::new(dir).append_index_html_on_directories(true))
        }
        None if embedded_ui_available() => router.fallback_service(get(embedded_static)),
        None => router,
    }
}

/// 内嵌 dist 的静态回落（独立形态）：按路径精确查找，未命中回落 index.html（SPA 前端路由）
pub(crate) async fn embedded_static(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file = assets::DIST
        .get_file(path)
        .or_else(|| assets::DIST.get_file("index.html"));
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

/// D5：Tauri v2 生产 WebView 的固定源（WKWebView 自定义协议映射为 http://tauri.localhost）。
/// 后端只信任这一个跨站源；前端在壳内以 http://127.0.0.1:{EASYVIBE_PORT} 直连。
pub(crate) const TAURI_ORIGIN: &str = "http://tauri.localhost";
