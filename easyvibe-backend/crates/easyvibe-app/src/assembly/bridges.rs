//! 装配格 · 落库/直播桥（5 条 spawn 工厂）。
//!
//! c-arch-10 R2/R3：自 `bootstrap.rs` 内联闭包**纯搬运**，逐段语义与顺序不变。
//! 这些桥是「为什么需要装配格」的核心证据：它们把会话/事件/输出三条流接进持久层与总线。

use crate::state::session_kind_from_label;
use easyvibe_api_types::{SessionStatus, SessionStatusChanged};
use easyvibe_event_bus::{publish, BusEvent};
use easyvibe_session::{OutputStream, SessionManager, SessionMetaUpdate as Meta};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

/// ① 会话终态桥：终态时 label/kind/terminal_at 落库（INSERT OR IGNORE 兜底无 spawn 会话），
/// 随后每会话只留最近 50k 行输出；随后把 `SessionStatus` 广播进总线。
pub(crate) fn spawn_session_terminal(
    mut session_rx: mpsc::Receiver<SessionStatusChanged>,
    session_manager: Arc<SessionManager>,
    agent_session_repo: Arc<easyvibe_db::AgentSessionRepo>,
    session_output_repo: Arc<easyvibe_db::SessionOutputRepo>,
    event_bus: broadcast::Sender<BusEvent>,
) {
    tokio::spawn(async move {
        while let Some(s) = session_rx.recv().await {
            // M1/U1 终态收尾：label/kind/terminal_at 落库（INSERT OR IGNORE 兜底 Stub 巡检等无 spawn 会话）
            if matches!(s.status, SessionStatus::Succeeded | SessionStatus::Failed) {
                let label = session_manager.label_of(&s.session_id).await.unwrap_or_else(|| s.session_id.clone());
                let kind = session_kind_from_label(&label);
                let now = chrono::Utc::now().to_rfc3339();
                let repo = agent_session_repo.clone();
                let sid = s.session_id.clone();
                let repo_id = s.repo.clone();
                let status = format!("{:?}", s.status).to_lowercase();
                let out_repo = session_output_repo.clone();
                tokio::spawn(async move {
                    if let Err(err) = repo.finalize(&sid, &repo_id, &status, &now, None, Some(&label), &kind).await {
                        tracing::warn!("[agent_sessions] finalize 失败 {sid}: {err}");
                    }
                    // M2 容量策略：终态后每会话只留最近 50k 行
                    if let Err(err) = out_repo.prune_session(&sid, 50_000).await {
                        tracing::warn!("[session_outputs] 修剪失败 {sid}: {err}");
                    }
                });
            }
            publish(&event_bus, BusEvent::SessionStatus(s));
        }
    });
}

/// ② 输出直播桥：agent 输出 → 事件总线（过程直播）。
pub(crate) fn spawn_output_live(session_manager: &Arc<SessionManager>, event_bus: broadcast::Sender<BusEvent>) {
    let mut rx = session_manager.subscribe_output();
    tokio::spawn(async move {
        while let Ok(o) = rx.recv().await {
            let stream = match o.stream {
                OutputStream::Stdout => "stdout",
                OutputStream::Stderr => "stderr",
            };
            publish(&event_bus, BusEvent::SessionOutput { session_id: o.session_id, seq: o.seq, stream: stream.into(), line: o.line });
        }
    });
}

/// ③ 输出落盘攒批桥：300ms 攒批 / 200 条上限；`INSERT OR IGNORE` 幂等；落盘失败仅 warn。
pub(crate) fn spawn_output_persist(session_manager: &Arc<SessionManager>, session_output_repo: Arc<easyvibe_db::SessionOutputRepo>) {
    let mut rx = session_manager.subscribe_output();
    let repo = session_output_repo;
    tokio::spawn(async move {
        let mut pending: Vec<easyvibe_db::SessionOutputRow> = Vec::new();
        let mut ticker = tokio::time::interval(std::time::Duration::from_millis(300));
        ticker.tick().await; // 立即 tick 消耗
        loop {
            tokio::select! {
                recv = rx.recv() => {
                    match recv {
                        Ok(o) => {
                            let stream = match o.stream {
                                OutputStream::Stdout => "stdout",
                                OutputStream::Stderr => "stderr",
                            };
                            pending.push(easyvibe_db::SessionOutputRow {
                                session_id: o.session_id,
                                seq: o.seq as i64,
                                ts: chrono::Utc::now().to_rfc3339(),
                                stream: stream.into(),
                                line: o.line,
                            });
                            if pending.len() >= 200 {
                                let batch = std::mem::take(&mut pending);
                                let repo = repo.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = repo.append_batch(&batch).await {
                                        tracing::warn!("[session_outputs] 落盘失败: {e}");
                                    }
                                });
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("[session_outputs] 广播滞后，丢 {n} 行（回放端点兜底）");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = ticker.tick() => {
                    if !pending.is_empty() {
                        let batch = std::mem::take(&mut pending);
                        let repo = repo.clone();
                        tokio::spawn(async move {
                            if let Err(e) = repo.append_batch(&batch).await {
                                tracing::warn!("[session_outputs] 落盘失败: {e}");
                            }
                        });
                    }
                }
            }
        }
    });
}

/// ④ 会话元事件桥：model/usage 边读边写；DB 故障只告警不阻流。
pub(crate) fn spawn_meta_persist(session_manager: &Arc<SessionManager>, agent_session_repo: Arc<easyvibe_db::AgentSessionRepo>) {
    let mut rx = session_manager.subscribe_meta();
    let repo = agent_session_repo;
    tokio::spawn(async move {
        while let Ok(e) = rx.recv().await {
            let res = match &e.update {
                Meta::Cli(cli) => {
                    let cmd = cli.rsplit(['/', '\\']).next().unwrap_or(cli).to_string();
                    repo.upsert_started(&e.session_id, &e.repo, &cmd, &chrono::Utc::now().to_rfc3339()).await
                }
                Meta::Label(label) => {
                    let kind = session_kind_from_label(label);
                    repo.set_label_kind(&e.session_id, label, kind).await
                }
                Meta::Model(m) => repo.set_model(&e.session_id, m).await,
                Meta::Usage { input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cost_usd, duration_ms, turns } => {
                    repo.set_usage(&e.session_id, *input_tokens, *output_tokens, *cache_read_tokens, *cache_write_tokens, *cost_usd, *duration_ms, *turns).await
                }
                Meta::ExitCode(_) => Ok(()), // 退出码随终态事件 finalize 统一写
            };
            if let Err(err) = res {
                tracing::warn!("[agent_sessions] 元事件落盘失败 {}: {err}", e.session_id);
            }
        }
    });
}

/// ⑤ 事件持久化桥：仅 TaskContractAlert / TaskContractViolated / PatrolFinished 三类落 events 表。
pub(crate) fn spawn_event_persist(event_bus: &broadcast::Sender<BusEvent>, pool: &easyvibe_db::sqlx::SqlitePool) {
    use easyvibe_db::EventRepository as _;
    let mut rx = event_bus.subscribe();
    let events = Arc::new(easyvibe_db::SqliteEventRepository::new(pool.clone()));
    tokio::spawn(async move {
        while let Ok(e) = rx.recv().await {
            let (repo, name, payload) = match e {
                BusEvent::TaskContractAlert { repo, task_id, files } => {
                    (repo, "task.contractAlert", serde_json::json!({ "taskId": task_id, "files": files.len() }))
                }
                BusEvent::TaskContractViolated { repo, task_id, files } => {
                    (repo, "task.contractViolated", serde_json::json!({ "taskId": task_id, "files": files.len() }))
                }
                BusEvent::PatrolFinished { repo, run_id, status } => {
                    (repo, "patrol.finished", serde_json::json!({ "runId": run_id, "status": status }))
                }
                _ => continue,
            };
            let _ = events.record(&repo, name, &payload.to_string()).await;
        }
    });
}
