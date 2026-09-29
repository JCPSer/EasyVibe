//! 组装层：二进制入口，REST 路由 + WS 事件推送。
use axum::{
    extract::{Path, State, WebSocketUpgrade},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use easyvibe_api_types::{
    GrowthEvent, HealthResponse, MapChanged, MapInvalid, RepoInfo, WsMessage,
};
use easyvibe_common::{events as ev, ApiError, ApiResponse, ErrorResponse};
use easyvibe_map::{repo_from_root, spawn_map_watcher, MapService};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::info;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone)]
pub struct AppState {
    pub map_service: Arc<MapService>,
    /// 后端 → 前端事件总线（broadcast；WS handler 订阅）
    pub event_bus: broadcast::Sender<BusEvent>,
}

/// 总线事件（内部枚举，发送时翻译为 WsMessage）
#[derive(Debug, Clone)]
pub enum BusEvent {
    MapChanged(MapChanged),
    MapInvalid(MapInvalid),
    Growth(GrowthEvent),
}

pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/repos", get(list_repos))
        .route("/repos/{id}/map", get(get_map))
        .route("/repos/{id}/growth", get(get_growth))
        .route("/repos/{id}/modules/{module_id}", get(get_submap))
        .with_state(state.clone());

    Router::new()
        .nest("/api", api)
        .route("/ws", get(ws_handler))
        .with_state(state)
}

async fn health() -> Json<ApiResponse<HealthResponse>> {
    Json(ApiResponse::ok(HealthResponse { status: "ok".into(), version: VERSION.into() }))
}

async fn list_repos(State(st): State<AppState>) -> Json<ApiResponse<Vec<RepoInfo>>> {
    let repos = st
        .map_service
        .repos()
        .into_iter()
        .map(|r| RepoInfo { id: r.id, name: r.name, root: r.root.to_string_lossy().into_owned() })
        .collect();
    Json(ApiResponse::ok(repos))
}

async fn get_map(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let snap = st.map_service.load_map(&repo).await?;
    Ok(Json(snap.json).into_response())
}

async fn get_growth(State(st): State<AppState>, Path(id): Path<String>) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let events: Vec<GrowthEvent> = st
        .map_service
        .load_growth(&repo)
        .await?
        .into_iter()
        .map(|payload| {
            let event_type = payload.get("type").and_then(|t| t.as_str()).unwrap_or("unknown").to_string();
            GrowthEvent { event_type, payload }
        })
        .collect();
    Ok(Json(events).into_response())
}

async fn get_submap(
    State(st): State<AppState>,
    Path((id, module_id)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let repo = st.map_service.find_repo(&id).ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    Ok(Json(st.map_service.load_submap(&repo, &module_id).await?).into_response())
}

/// WS：订阅事件总线，向前端推送 domain.camelCase 事件
async fn ws_handler(State(st): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |mut socket| async move {
        use axum::extract::ws::Message;
        let mut rx = st.event_bus.subscribe();
        while let Ok(event) = rx.recv().await {
            let msg: WsMessage<Value> = match event {
                BusEvent::MapChanged(d) => WsMessage { name: ev::MAP_CHANGED.into(), data: serde_json::to_value(d).unwrap_or_default() },
                BusEvent::MapInvalid(d) => WsMessage { name: ev::MAP_INVALID.into(), data: serde_json::to_value(d).unwrap_or_default() },
                BusEvent::Growth(g) => WsMessage { name: ev::GROWTH_EVENT.into(), data: serde_json::to_value(g).unwrap_or_default() },
            };
            if let Ok(text) = serde_json::to_string(&msg) {
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break; // 前端断开
                }
            }
        }
    })
}

/// ApiError 的新类型包装（绕过孤儿规则；common 层不依赖 axum）
struct AppError(ApiError);

impl From<ApiError> for AppError {
    fn from(e: ApiError) -> Self {
        AppError(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let err = self.0;
        let (status, code) = match &err {
            ApiError::BadRequest(_) => (axum::http::StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            ApiError::NotFound(_) => (axum::http::StatusCode::NOT_FOUND, "NOT_FOUND"),
            ApiError::MapInvalid(_) => (axum::http::StatusCode::UNPROCESSABLE_ENTITY, "MAP_INVALID"),
            ApiError::Internal(_) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        };
        let body = Json(ErrorResponse { success: false, error: err.to_string(), code: code.into() });
        (status, body).into_response()
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();

    // 仓库注册：从 EASYVIBE_REPO 环境变量读取（逗号分隔的多工作区预留），M2-1 先支持一个
    let repo_roots: Vec<std::path::PathBuf> = std::env::var("EASYVIBE_REPO")
        .expect("请设置 EASYVIBE_REPO 指向代码仓库根目录（可逗号分隔多个）")
        .split(',')
        .map(|s| s.trim().into())
        .collect();
    let repos: Vec<_> = repo_roots.iter().map(|p| repo_from_root(p)).collect();
    for r in &repos {
        info!("注册仓库 {} -> {}", r.id, r.root.display());
    }

    let map_service = MapService::new(repos.clone());
    // 预热缓存（不出残图：加载失败仅告警，不阻断启动）
    for r in &repos {
        if let Err(e) = map_service.load_map(r).await {
            tracing::warn!("预热 {} 失败: {e}", r.id);
        }
    }

    let (event_bus, _) = broadcast::channel(256);

    // 每个仓库一个地图 watcher，变更翻译为总线事件
    for r in repos {
        let mut rx = spawn_map_watcher(map_service.clone(), r.clone());
        let bus = event_bus.clone();
        let repo_id = r.id.clone();
        tokio::spawn(async move {
            while rx.changed().await.is_ok() {
                let event = match rx.borrow().clone() {
                    Ok(snap) => {
                        let version = snap.json.get("version").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                        BusEvent::MapChanged(MapChanged { repo: repo_id.clone(), version })
                    }
                    Err(e) => BusEvent::MapInvalid(MapInvalid { repo: repo_id.clone(), error: e }),
                };
                let _ = bus.send(event);
            }
        });
    }

    let state = AppState { map_service, event_bus };
    let app = build_router(state);
    let addr = "127.0.0.1:7101";
    info!("EasyVibe backend listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.expect("绑定 7101 失败");
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn test_state() -> AppState {
        let svc = MapService::new(vec![]);
        let (bus, _) = broadcast::channel(8);
        AppState { map_service: svc, event_bus: bus }
    }

    #[tokio::test]
    async fn health_ok() {
        let app = build_router(test_state());
        let resp = app.oneshot(axum::http::Request::get("/api/health").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_repo_404() {
        let app = build_router(test_state());
        let resp = app.oneshot(axum::http::Request::get("/api/repos/nope/map").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
    }
}
