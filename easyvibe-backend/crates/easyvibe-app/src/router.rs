//! 路由表装配：唯一「路径/方法 ↔ handler」对照点 + 静态回落。

use axum::{
    response::{IntoResponse, Response},
    routing::get, Router,
};
use crate::state::*;
use crate::assets;
use crate::assets::embedded_ui_available;
use crate::ws::ws_handler;
use crate::{git, session_queue_routes};
use crate::routes::{repo::*, map::*, sessions::*, task::*, chat::*, settings::*, agent::*, dev_docs::*};

pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/repos", get(list_repos).post(add_repo))
        .route("/repos/{id}", axum::routing::delete(remove_repo))
        .route("/repos/{id}/map", get(get_map))
        .route("/repos/{id}/freshness", get(get_freshness))
        .route("/repos/{id}/modules/{module_id}/health-history", get(get_health_history))
        .route("/repos/{id}/modules/{module_id}/analyze-submap", axum::routing::post(analyze_submap))
        .route("/repos/{id}/growth", get(get_growth))
        .route("/repos/{id}/progress", get(get_progress))
        .route("/repos/{id}/modules/{module_id}", get(get_submap))
        .route("/repos/{id}/reinduce", axum::routing::post(start_reinduce))
        .route("/repos/{id}/patrol", axum::routing::post(start_patrol))
        // 运行会话排队（需求 v1 §5/§8：GET 查 active+queued；POST 原子裁决入队/代执行；DELETE 取消）
        .route(
            "/repos/{id}/session-queue",
            get(session_queue_routes::get_session_queue)
                .post(session_queue_routes::post_session_queue)
                .delete(session_queue_routes::delete_session_queue),
        )
        .route("/repos/{id}/patrol-runs", get(list_patrol_runs).delete(prune_patrol_runs))
        // 全局运行指示（2026-10-05）：跨仓库活动会话 + 队列——d928389 实现 handler 但漏注册，
        // 前端 404 静默兜底 = 巡检/归纳进行中状态丸与运行页永远空白（实弹 bug）。
        .route("/sessions/overview", get(session_queue_routes::get_sessions_overview))
        .route("/repos/{id}/agent-sessions", get(list_agent_sessions))
        .route("/repos/{id}/usage", get(get_usage))
        .route("/repos/{id}/health-dashboard", get(get_health_dashboard))
        // R3 D1：使用证据埋点——前端交互事件入库 + 门控计数读数
        .route("/repos/{id}/events", axum::routing::post(ingest_event))
        .route("/repos/{id}/events/summary", get(events_summary))
        .route("/repos/{id}/git/status", get(git::get_git_status))
        .route("/repos/{id}/git/log", get(git::get_git_log))
        .route("/repos/{id}/git/commit", get(git::get_git_commit))
        .route("/repos/{id}/git/commit", axum::routing::post(git::post_git_commit))
        .route("/repos/{id}/git/pull", axum::routing::post(git::post_git_pull))
        .route("/repos/{id}/git/push", axum::routing::post(git::post_git_push))
        .route("/repos/{id}/git/discard", axum::routing::post(git::post_git_discard))
        .route("/repos/{id}/git/commit-message", axum::routing::post(git::post_git_commit_message))
        .route("/repos/{id}/sessions/{sid}/kill", axum::routing::post(post_session_kill))
        .route("/repos/{id}/sessions/{sid}/output", get(get_session_output))
        .route("/repos/{id}/tasks/{tid}/kill", axum::routing::post(post_task_kill))
        .route("/repos/{id}/chat", get(get_chat).post(chat))
        .route("/repos/{id}/conversations", get(list_conversations).post(create_conversation))
        .route("/repos/{id}/conversations/{cid}", axum::routing::put(rename_conversation).delete(delete_conversation))
        .route("/repos/{id}/chat/compact", axum::routing::post(compact_chat))
        .route("/repos/{id}/chat/reset", axum::routing::post(reset_chat))
        .route("/repos/{id}/views", get(list_views).post(save_view))
        .route("/repos/{id}/views/{slug}", axum::routing::delete(delete_view).put(rename_view))
        .route("/repos/{id}/tasks", get(list_tasks).post(create_task))
        .route("/repos/{id}/tasks/{tid}", axum::routing::delete(delete_task))
        .route("/repos/{id}/tasks/{tid}/decide", axum::routing::post(decide_task))
        .route("/repos/{id}/tasks/{tid}/retry", axum::routing::post(post_task_retry))
        .route("/repos/{id}/tasks/{tid}/remediate", axum::routing::post(post_task_remediate))
        // 管道回看·节点重开（2026-10-05 方案 §3.1）：把任务放回目标评审关，复用 decide/打回闭环
        .route("/repos/{id}/tasks/{tid}/rewind", axum::routing::post(post_task_rewind))
        // 代码审查节点的人工复审（2026-10-05 用户裁定）：审查-修复闭环，非一键通过
        .route("/repos/{id}/tasks/{tid}/review", axum::routing::post(post_task_review))
        .route("/repos/{id}/tasks/{tid}/approvals", get(list_task_approvals))
        .route("/repos/{id}/tasks/{tid}/diff", get(get_task_diff))
        .route("/repos/{id}/dev-docs", get(get_dev_docs))
        .route("/repos/{id}/dev-doc", get(get_dev_doc).delete(delete_dev_doc))
        .route("/repos/{id}/suggest", axum::routing::post(suggest))
        .route("/settings", get(list_settings))
        .route("/settings/set", axum::routing::put(put_setting))
        .route("/settings/{scope}/{key}", axum::routing::delete(delete_setting))
        .route("/harness", get(get_harness))
        .route("/harness/files", get(list_harness_files))
        .route("/harness/file", get(get_harness_file).put(put_harness_file))
        .route("/harness/backups", get(list_harness_backups))
        .route("/harness/restore", axum::routing::post(restore_harness))
        .route("/agent/status", get(agent_status))
        .route("/agent/detect", axum::routing::post(agent_detect))
        .route("/agent/test", axum::routing::post(agent_test))
        .route("/llm/test", axum::routing::post(llm_test))
        .route("/diagnostics", get(export_diagnostics))
        .route("/harness/reset", axum::routing::post(reset_harness))
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
