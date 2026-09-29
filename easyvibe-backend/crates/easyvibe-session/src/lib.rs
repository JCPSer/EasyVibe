//! 会话领域：外部 agent 的统一会话抽象（M2-3 先落地 CLI 后端；ACP 后端后续挂同一边界，
//! 参考 AionCore aionui-session 的"直连 CLI 与 ACP 统一会话"思路）。
//!
//! 职责边界（与 easyvibe-map 分工）：
//! - 本 crate 只管"把 agent 跑起来、看住它、报状态"
//! - 四通道文件（progress/growth.log/parts/map.json）的消费由 easyvibe-map 的 watcher 负责，
//!   agent 只要遵守 v2.2 协议写文件，后端不关心它怎么写
use easyvibe_api_types::{SessionStatus, SessionStatusChanged};
use easyvibe_common::ApiError;
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// 会话完成/状态变化的回调（app 层翻译为 WS 事件）
pub type SessionEventSender = tokio::sync::mpsc::Sender<SessionStatusChanged>;

pub struct SessionManager {
    active: Arc<RwLock<HashMap<String, SessionStatusChanged>>>, // repo -> 最新会话状态
    counter: AtomicU64,
    events: SessionEventSender,
}

impl SessionManager {
    pub fn new(events: SessionEventSender) -> Arc<Self> {
        Arc::new(Self { active: Default::default(), counter: AtomicU64::new(0), events })
    }

    pub async fn status_of(&self, repo_id: &str) -> Option<SessionStatusChanged> {
        self.active.read().await.get(repo_id).cloned()
    }

    /// 启动一次归纳会话（v2.2 协议执行者 = 外部 agent CLI）。
    /// 单会话纪律：一个仓库同时只允许一个活动会话。
    pub async fn start_induction(
        &self,
        repo_id: &str,
        repo_root: &Path,
        prompt_template: &str,
        command: &str,
        args: &[String],
    ) -> Result<SessionStatusChanged, ApiError> {
        if let Some(s) = self.active.read().await.get(repo_id) {
            if matches!(s.status, SessionStatus::Starting | SessionStatus::Running) {
                return Err(ApiError::Conflict(format!("仓库 {repo_id} 已有活动会话 {}", s.session_id)));
            }
        }

        let session_id = format!("ind-{}", self.counter.fetch_add(1, Ordering::SeqCst));
        let status = SessionStatusChanged {
            repo: repo_id.to_string(),
            session_id: session_id.clone(),
            status: SessionStatus::Starting,
        };
        self.publish(status.clone()).await;

        let rendered = prompt_template.replace("<REPO_ROOT>", &repo_root.to_string_lossy());
        let mut cmd = Command::new(command);
        cmd.args(args)
            .current_dir(repo_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| {
            ApiError::Internal(format!("spawn {command} 失败: {e}"))
        })?;

        // prompt 经 stdin 注入（避免 argv 长度限制；agent 自行读 SCHEMA 文件）
        if let Some(mut stdin) = child.stdin.take() {
            let rendered2 = rendered.clone();
            tokio::spawn(async move {
                let _ = stdin.write_all(rendered2.as_bytes()).await;
                // stdin 关闭即 EOF，agent 收到完整 prompt
            });
        }

        self.set_status(&status.repo, &status.session_id, SessionStatus::Running).await;

        // 看门任务：收集输出（翻译层的最小形态，防大输出——只记行数与前 200 字节），等退出，报终态
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let events = self.events.clone();
        let active = self.active.clone();
        let repo = repo_id.to_string();
        tokio::spawn(async move {
            let mut out_lines = 0u64;
            if let Some(mut s) = stdout {
                use tokio::io::AsyncBufReadExt as _;
                let mut reader = tokio::io::BufReader::new(&mut s);
                let mut line = String::new();
                loop {
                    match reader.read_line(&mut line).await {
                        Ok(0) => break,
                        Ok(_) => {
                            out_lines += 1;
                            if out_lines <= 3 || out_lines % 50 == 0 {
                                let peek: String = line.chars().take(200).collect();
                                info!("[session {session_id}] stdout#{out_lines}: {peek}");
                            }
                            line.clear();
                        }
                        Err(e) => {
                            warn!("[session {session_id}] stdout 读取失败: {e}");
                            break;
                        }
                    }
                }
            }
            let status = match child.wait().await {
                Ok(exit) if exit.success() => SessionStatus::Succeeded,
                Ok(exit) => SessionStatus::Failed,
                Err(e) => {
                    warn!("[session {session_id}] wait 失败: {e}");
                    SessionStatus::Failed
                }
            };
            let _ = stderr; // stderr 随进程关闭丢弃（详细排查看 agent 自身日志）
            let final_status = SessionStatusChanged { repo: repo.clone(), session_id: session_id.clone(), status };
            active.write().await.insert(repo.clone(), final_status.clone());
            let _ = events.send(final_status).await;
            info!("[session {session_id}] 终态: {:?}", status);
        });

        Ok(self.active.read().await.get(repo_id).cloned().unwrap_or(status))
    }

    async fn set_status(&self, repo: &str, session_id: &str, status: SessionStatus) {
        let s = SessionStatusChanged { repo: repo.into(), session_id: session_id.into(), status };
        self.publish(s).await;
    }

    async fn publish(&self, s: SessionStatusChanged) {
        self.active.write().await.insert(s.repo.clone(), s.clone());
        let _ = self.events.send(s).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn single_session_discipline() {
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        // 第一个会话：用 sleep 占住（macOS/Linux 均有）
        let s1 = mgr
            .start_induction("repo1", &dir, "test", "sleep", &["30".to_string()])
            .await
            .unwrap();
        assert_eq!(s1.status, SessionStatus::Running);
        // 同仓库第二个会话必须被拒
        let err = mgr.start_induction("repo1", &dir, "test", "sleep", &["1".to_string()]).await;
        assert!(matches!(err, Err(ApiError::Conflict(_))));
        // 其他仓库不受影响
        assert!(mgr.start_induction("repo2", &dir, "test", "sleep", &["1".to_string()]).await.is_ok());
    }

    // 收事件直到终态（队列里有 Starting/Running 前态）
    async fn recv_terminal(rx: &mut tokio::sync::mpsc::Receiver<SessionStatusChanged>) -> SessionStatusChanged {
        loop {
            let evt = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .expect("超时未收到会话事件")
                .expect("事件通道关闭");
            if matches!(evt.status, SessionStatus::Succeeded | SessionStatus::Failed) {
                return evt;
            }
        }
    }

    #[tokio::test]
    async fn session_lifecycle_succeeded() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        // true 命令立即成功退出
        let s = mgr.start_induction("repo1", &dir, "test", "true", &[]).await.unwrap();
        assert_eq!(s.status, SessionStatus::Running);
        let final_evt = recv_terminal(&mut rx).await;
        assert_eq!(final_evt.status, SessionStatus::Succeeded);
        assert_eq!(final_evt.repo, "repo1");
    }

    #[tokio::test]
    async fn session_lifecycle_failed() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        let _ = mgr.start_induction("repo1", &dir, "test", "false", &[]).await.unwrap();
        let final_evt = recv_terminal(&mut rx).await;
        assert_eq!(final_evt.status, SessionStatus::Failed);
    }
}
