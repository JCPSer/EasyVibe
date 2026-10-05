//! WS 事件面：总线订阅 + BusEvent→WsMessage 唯一翻译层。

use crate::state::*;
use axum::{
    extract::{State, WebSocketUpgrade},
    response::Response,
};
use easyvibe_api_types::WsMessage;
use easyvibe_common::events as ev;
use easyvibe_event_bus::BusEvent;
use serde_json::Value;

/// WS：订阅事件总线，向前端推送 domain.camelCase 事件
pub(crate) async fn ws_handler(State(st): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |mut socket| async move {
        use axum::extract::ws::Message;
        let mut rx = st.event_bus.subscribe();
        // Lagged（广播滞后）不致命：跳过丢失的批次继续收；Closed 才退出
        loop {
            let event = match rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!("[ws] 滞后，丢弃 {skipped} 个事件（客户端应经 REST 重同步）");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let msg = translate(event);
            if let Ok(text) = serde_json::to_string(&msg) {
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break; // 前端断开
                }
            }
        }
    })
}

/// BusEvent → WsMessage 的唯一翻译层（R7：抽为纯函数以便快照测试钉死事件名）。
/// 事件名/载荷必须与前端 `App.tsx` 的 `msg.name` 订阅保持一致，迁移不得漂移。
pub(crate) fn translate(event: BusEvent) -> WsMessage<Value> {
    match event {
        BusEvent::MapChanged(d) => WsMessage { name: ev::MAP_CHANGED.into(), data: serde_json::to_value(d).unwrap_or_default() },
        BusEvent::MapInvalid(d) => WsMessage { name: ev::MAP_INVALID.into(), data: serde_json::to_value(d).unwrap_or_default() },
        BusEvent::Growth { repo, event } => WsMessage {
            name: ev::GROWTH_EVENT.into(),
            data: serde_json::json!({ "repo": repo, "event": event }),
        },
        BusEvent::Progress { repo, progress } => WsMessage {
            name: ev::PROGRESS_UPDATED.into(),
            data: serde_json::json!({ "repo": repo, "progress": progress }),
        },
        BusEvent::SessionStatus(s) => WsMessage {
            name: ev::SESSION_STATUS_CHANGED.into(),
            data: serde_json::to_value(s).unwrap_or_default(),
        },
        BusEvent::TaskStatus { repo, task_id, status, gate } => WsMessage {
            name: "task.statusChanged".into(),
            data: serde_json::json!({ "repo": repo, "taskId": task_id, "status": status, "gate": gate }),
        },
        BusEvent::TaskContractViolated { repo, task_id, files } => WsMessage {
            name: "task.contractViolated".into(),
            data: serde_json::json!({ "repo": repo, "taskId": task_id, "files": files }),
        },
        BusEvent::TaskContractAlert { repo, task_id, files } => WsMessage {
            name: "task.contractAlert".into(),
            data: serde_json::json!({ "repo": repo, "taskId": task_id, "files": files }),
        },
        BusEvent::Freshness { repo, status, latest_commit_at, commits_since_map } => WsMessage {
            name: "freshness.changed".into(),
            data: serde_json::json!({ "repo": repo, "status": status, "latestCommitAt": latest_commit_at, "commitsSinceMap": commits_since_map }),
        },
        BusEvent::SessionOutput { session_id, seq, stream, line } => WsMessage {
            name: "session.output".into(),
            data: serde_json::json!({ "sessionId": session_id, "seq": seq, "stream": stream, "line": line }),
        },
        BusEvent::PatrolFinished { repo, run_id, status } => WsMessage {
            name: "patrol.finished".into(),
            data: serde_json::json!({ "repo": repo, "runId": run_id, "status": status }),
        },
        BusEvent::QueueChanged { repo, change } => WsMessage {
            name: ev::QUEUE_CHANGED.into(),
            data: change.to_payload(&repo),
        },
    }
}
