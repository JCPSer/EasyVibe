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

/// 会话 stdout 行（改进#2"分析中黑洞"——agent 过程直播的原料；行已截断）
#[derive(Debug, Clone)]
pub struct SessionOutput {
    pub session_id: String,
    pub line: String,
}

pub struct SessionManager {
    active: Arc<RwLock<HashMap<String, SessionStatusChanged>>>, // repo -> 最新会话状态（互斥判定用）
    by_id: Arc<RwLock<HashMap<String, SessionStatusChanged>>>,  // session_id -> 状态（终态归属用，审查 🔴1）
    /// session_id -> stdout 捕获（M4-1 产物归档采集的原料；单会话 ≤1MB 封顶）
    outputs: Arc<RwLock<HashMap<String, Arc<std::sync::Mutex<String>>>>>,
    /// P0（审查后端#1）：session_id -> 终止信号。主动 kill 与超时 kill 都经此通道，
    /// 由看门任务 select 接收后 start_kill——child 句柄不外泄，避免共享可变状态。
    killers: Arc<RwLock<HashMap<String, Arc<tokio::sync::Notify>>>>,
    counter: AtomicU64,
    events: SessionEventSender,
    /// agent 过程直播：stdout 逐行广播（改进#2）
    output_tx: Arc<tokio::sync::broadcast::Sender<SessionOutput>>,
    /// 会话超时（秒）：超时未终态 → 杀进程判 Failed。环境可调（EASYVIBE_SESSION_TIMEOUT_SECS），默认 30 分钟。
    timeout: std::time::Duration,
}

/// 会话超时：覆盖归纳/巡检/子图分析/任务执行全部 spawn 路径（审查后端#1——agent 挂死 =
/// 写互斥永占 + 任务 permit 泄漏 + 重试循环空转）。agent 正常执行都在分钟级，30 分钟为宽限。
fn session_timeout() -> std::time::Duration {
    std::env::var("EASYVIBE_SESSION_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&s| s > 0)
        .map(std::time::Duration::from_secs)
        .unwrap_or(std::time::Duration::from_secs(30 * 60))
}

impl SessionManager {
    pub fn new(events: SessionEventSender) -> Arc<Self> {
        let (output_tx, _) = tokio::sync::broadcast::channel(512);
        Arc::new(Self {
            active: Default::default(),
            by_id: Default::default(),
            outputs: Default::default(),
            killers: Default::default(),
            counter: AtomicU64::new(0),
            events,
            output_tx: Arc::new(output_tx),
            timeout: session_timeout(),
        })
    }

    /// 主动终止会话（P0 审查后端#1）：kill 信号 → 看门任务 select 命中 → start_kill → 判 Failed。
    /// 仅活动会话可 kill；已终态返回 409。
    pub async fn kill(&self, session_id: &str) -> Result<(), ApiError> {
        let Some(s) = self.by_id.read().await.get(session_id).cloned() else {
            return Err(ApiError::NotFound(format!("会话 {session_id} 不存在")));
        };
        if !matches!(s.status, SessionStatus::Starting | SessionStatus::Running) {
            return Err(ApiError::Conflict(format!("会话 {session_id} 已终态（{:?}）", s.status)));
        }
        let notify = self.killers.read().await.get(session_id).cloned();
        match notify {
            Some(n) => {
                n.notify_one();
                Ok(())
            }
            None => Err(ApiError::Conflict(format!("会话 {session_id} 无终止通道（外部注册会话不支持 kill）"))),
        }
    }

    /// 测试专用：显式超时常数（env 在并行测试下变异是竞态，不走 EASYVIBE_SESSION_TIMEOUT_SECS）
    #[cfg(test)]
    pub fn new_with_timeout(events: SessionEventSender, timeout: std::time::Duration) -> Arc<Self> {
        let (output_tx, _) = tokio::sync::broadcast::channel(512);
        Arc::new(Self {
            active: Default::default(),
            by_id: Default::default(),
            outputs: Default::default(),
            killers: Default::default(),
            counter: AtomicU64::new(100), // 与 new() 的计数器错开，避免测试间 session id 冲突
            events,
            output_tx: Arc::new(output_tx),
            timeout,
        })
    }

    /// 订阅会话 stdout 行（app 层翻译为 WS session.output）
    pub fn subscribe_output(&self) -> tokio::sync::broadcast::Receiver<SessionOutput> {
        self.output_tx.subscribe()
    }

    /// 会话 stdout（M4-1）：任务终态后解析 [EASYVIBE-RESULT] 的原料；无捕获返回 None。
    /// 注意：只读不取——缓冲在 take_output 消费前一直保留（契约：终态后随时可读）。
    pub async fn output_of(&self, session_id: &str) -> Option<String> {
        let buf = self.outputs.read().await.get(session_id)?.clone();
        Some(buf.lock().map(|s| s.clone()).unwrap_or_default())
    }

    /// 消费式领取 stdout 缓冲（领取即从表移除——SessionManager 内存只增不减债务的消解点：
    /// 任务执行器终态采集走这里；无人消费的历史缓冲由 map 容量自然受限于会话数，1MB/会话封顶）
    pub async fn take_output(&self, session_id: &str) -> Option<String> {
        let buf = self.outputs.write().await.remove(session_id)?;
        Some(buf.lock().map(|mut s| std::mem::take(&mut *s)).unwrap_or_default())
    }

    pub async fn status_of(&self, repo_id: &str) -> Option<SessionStatusChanged> {
        self.active.read().await.get(repo_id).cloned()
    }

    /// 注册一个外部活动会话（如巡检）；仓库已有活动会话（归纳/巡检任一）则拒绝。
    /// 与 start_induction 共用同一纪律：地图写操作全局单飞。
    /// 注意：检查与插入必须在同一把写锁内完成（修代码审查发现的 TOCTOU 竞态）。
    pub async fn try_register(&self, s: SessionStatusChanged) -> Result<(), ApiError> {
        let mut map = self.active.write().await;
        if let Some(existing) = map.get(&s.repo) {
            if matches!(existing.status, SessionStatus::Starting | SessionStatus::Running) {
                return Err(ApiError::Conflict(format!(
                    "仓库 {} 有活动会话 {}，地图写操作需排队",
                    s.repo, existing.session_id
                )));
            }
        }
        map.insert(s.repo.clone(), s.clone());
        drop(map);
        let _ = self.events.send(s).await;
        Ok(())
    }

    /// 外部会话状态变更（不校验，直接发布）——巡检等自管生命周期的会话使用
    pub async fn note_status(&self, s: SessionStatusChanged) {
        self.publish(s).await;
    }

    /// 启动一次归纳会话（v2.2 协议执行者 = 外部 agent CLI）。
    /// 单会话纪律：一个仓库同时只允许一个活动会话。
    /// timeout：N26 按槽位分类——透明槽位（归纳/巡检/子图）用默认 30min，
    /// 任务槽（自由 coding 常态 40-60 分钟）传 Some(90min)；None = 默认。
    pub async fn start_induction(
        &self,
        repo_id: &str,
        repo_root: &Path,
        prompt_template: &str,
        command: &str,
        args: &[String],
        timeout: Option<std::time::Duration>,
    ) -> Result<SessionStatusChanged, ApiError> {
        let session_id = format!("ind-{}", self.counter.fetch_add(1, Ordering::SeqCst));
        // 单会话纪律：与巡检等地图写操作共用（try_register 内含活动会话检查）
        self.try_register(SessionStatusChanged {
            repo: repo_id.to_string(),
            session_id: session_id.clone(),
            status: SessionStatus::Starting,
        })
        .await?;

        let rendered = prompt_template.replace("<REPO_ROOT>", &repo_root.to_string_lossy());
        let mut cmd = Command::new(command);
        cmd.args(args)
            .current_dir(repo_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                // spawn 失败必须释放会话注册，否则仓库写路径永久锁死（审查发现 Y1）
                self.note_status(SessionStatusChanged {
                    repo: repo_id.to_string(),
                    session_id: session_id.clone(),
                    status: SessionStatus::Failed,
                })
                .await;
                let hint = if e.kind() == std::io::ErrorKind::NotFound {
                    format!("——未找到 CLI agent `{command}`（GUI 启动的进程 PATH 极薄，后端启动时会解析绝对路径；仍失败请安装 agent 或设置 EASYVIBE_AGENT_CMD 为绝对路径）")
                } else {
                    String::new()
                };
                return Err(ApiError::Internal(format!("spawn {command} 失败: {e}{hint}")));
            }
        };

        // prompt 经 stdin 注入（避免 argv 长度限制；agent 自行读 SCHEMA 文件）
        if let Some(mut stdin) = child.stdin.take() {
            let rendered2 = rendered.clone();
            tokio::spawn(async move {
                let _ = stdin.write_all(rendered2.as_bytes()).await;
                // stdin 关闭即 EOF，agent 收到完整 prompt
            });
        }

        // stdout 捕获缓冲（M4-1）：任务终态后解析 [EASYVIBE-RESULT] 的原料
        let stdout_buf = Arc::new(std::sync::Mutex::new(String::new()));
        self.outputs.write().await.insert(session_id.clone(), stdout_buf.clone());
        let out_output_tx = self.output_tx.clone();
        let err_output_tx = self.output_tx.clone();
        let out_session_id = session_id.clone();
        // P0：终止通道（主动 kill / 超时共用）——notify 幂等，重复 kill 无副作用
        let kill_notify = Arc::new(tokio::sync::Notify::new());
        self.killers.write().await.insert(session_id.clone(), kill_notify.clone());
        let timeout = timeout.unwrap_or(self.timeout);

        self.set_status(repo_id, &session_id, SessionStatus::Running).await;

        // 看门任务：stdout/stderr 并发排空（审查 🔴2：串行"先 wait 后排 stderr"会让
        // 运行期写满 64KB 的 agent 死锁——必须同时读两个管道），再等退出，报终态
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let events = self.events.clone();
        let active = self.active.clone();
        let by_id = self.by_id.clone();
        let killers = self.killers.clone();
        let repo = repo_id.to_string();
        let session_id_task = session_id.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt as _;
            let out_id = session_id_task.clone();
            let out_task = tokio::spawn(async move {
                let mut lines = 0u64;
                if let Some(mut s) = stdout {
                    let mut reader = tokio::io::BufReader::new(&mut s);
                    let mut line = String::new();
                    loop {
                        match reader.read_line(&mut line).await {
                            Ok(0) => break,
                            Ok(_) => {
                                lines += 1;
                                if lines <= 3 || lines % 50 == 0 {
                                    let peek: String = line.chars().take(200).collect();
                                    info!("[session {out_id}] stdout#{lines}: {peek}");
                                }
                                // 改进#2：过程直播——行截断 200 字符广播（行率不高，直接发）
                                let _ = out_output_tx.send(SessionOutput {
                                    session_id: out_session_id.clone(),
                                    line: line.chars().take(200).collect(),
                                });
                                // M4-1：捕获进缓冲（1MB 封顶，保头丢尾——RESULT 行在末尾）
                                if let Ok(mut buf) = stdout_buf.lock() {
                                    if buf.len() < 1_048_576 {
                                        buf.push_str(&line);
                                    } else if line.contains("[EASYVIBE-RESULT]") {
                                        // 超帽时仍保留 RESULT 归档行（短行，替换式保底）
                                        let trimmed: String = line.chars().take(4096).collect();
                                        buf.push_str(&trimmed);
                                    }
                                }
                                line.clear();
                            }
                            Err(_) => break,
                        }
                    }
                }
            });
            let err_id = session_id_task.clone();
            let err_task = tokio::spawn(async move {
                let mut lines = 0u64;
                if let Some(mut e) = stderr {
                    let mut reader = tokio::io::BufReader::new(&mut e);
                    let mut line = String::new();
                    loop {
                        match reader.read_line(&mut line).await {
                            Ok(0) => break,
                            Ok(_) => {
                                lines += 1;
                                let peek: String = line.chars().take(200).collect();
                                warn!("[session {err_id}] stderr#{lines}: {peek}");
                                // 失败可诊断：stderr 也进过程直播（[err] 前缀），任务卡可见死亡原因
                                let _ = err_output_tx.send(crate::SessionOutput {
                                    session_id: err_id.clone(),
                                    line: format!("[err] {peek}"),
                                });
                                line.clear();
                            }
                            Err(_) => break,
                        }
                    }
                }
            });
            // P0 审查后端#1：wait 与 主动kill / 超时 三者竞速——任一命中先杀进程再判终态。
            // 顺序必须是 select 在前 join 在后：join 等管道 EOF，而 EOF 依赖子进程死亡——
            // 若先 join 后 select，kill/超时永远轮不到，形成死锁（本次实弹教训）。
            enum Outcome {
                Killed(&'static str),
                Exited(std::io::Result<std::process::ExitStatus>),
            }
            let outcome = tokio::select! {
                _ = kill_notify.notified() => Outcome::Killed("用户主动终止"),
                _ = tokio::time::sleep(timeout) => Outcome::Killed("会话超时（agent 挂死防线）"),
                s = child.wait() => Outcome::Exited(s),
            };
            let status = match outcome {
                Outcome::Killed(reason) => {
                    warn!("[session {session_id_task}] 被终止: {reason}");
                    if let Err(e) = child.start_kill() {
                        warn!("[session {session_id_task}] start_kill 失败: {e}");
                    }
                    let _ = child.wait().await;
                    SessionStatus::Failed
                }
                Outcome::Exited(Ok(exit)) if exit.success() => SessionStatus::Succeeded,
                Outcome::Exited(Ok(_exit)) => SessionStatus::Failed,
                Outcome::Exited(Err(e)) => {
                    warn!("[session {session_id_task}] wait 失败: {e}");
                    SessionStatus::Failed
                }
            };
            // 子进程已退出：管道写端关闭，排空任务很快收尾
            let _ = tokio::join!(out_task, err_task);
            // 终态清理：kill 通道随会话结束回收（stdout 缓冲由消费者 take_output 领取，见 M4-1 契约）
            killers.write().await.remove(&session_id_task);
            let final_status = SessionStatusChanged { repo: repo.clone(), session_id: session_id_task.clone(), status };
            active.write().await.insert(repo.clone(), final_status.clone());
            by_id.write().await.insert(session_id_task.clone(), final_status.clone());
            let _ = events.send(final_status).await;
            info!("[session {session_id_task}] 终态: {:?}", status);
        });

        Ok(SessionStatusChanged {
            repo: repo_id.to_string(),
            session_id,
            status: SessionStatus::Running,
        })
    }

    async fn set_status(&self, repo: &str, session_id: &str, status: SessionStatus) {
        let s = SessionStatusChanged { repo: repo.into(), session_id: session_id.into(), status };
        self.publish(s).await;
    }

    async fn publish(&self, s: SessionStatusChanged) {
        self.active.write().await.insert(s.repo.clone(), s.clone());
        self.by_id.write().await.insert(s.session_id.clone(), s.clone());
        let _ = self.events.send(s).await;
    }

    /// 按 session_id 查询状态（终态归属的唯一依据——避免按仓库轮询的归属竞态）
    pub async fn status_of_session(&self, session_id: &str) -> Option<SessionStatusChanged> {
        self.by_id.read().await.get(session_id).cloned()
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
            .start_induction("repo1", &dir, "test", "sleep", &["30".to_string()], None)
            .await
            .unwrap();
        assert_eq!(s1.status, SessionStatus::Running);
        // 同仓库第二个会话必须被拒
        let err = mgr.start_induction("repo1", &dir, "test", "sleep", &["1".to_string()], None).await;
        assert!(matches!(err, Err(ApiError::Conflict(_))));
        // 其他仓库不受影响
        assert!(mgr.start_induction("repo2", &dir, "test", "sleep", &["1".to_string()], None).await.is_ok());
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
    async fn write_mutex_shared_between_kinds() {
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        // 归纳会话占住仓库
        let _s = mgr.start_induction("repo1", &dir, "t", "sleep", &["30".to_string()], None).await.unwrap();
        // 巡检注册必须被拒（跨类型的写互斥）
        let err = mgr
            .try_register(SessionStatusChanged {
                repo: "repo1".into(),
                session_id: "patrol-x".into(),
                status: SessionStatus::Running,
            })
            .await;
        assert!(matches!(err, Err(ApiError::Conflict(_))));
        // 其他仓库的巡检不受影响
        assert!(mgr
            .try_register(SessionStatusChanged {
                repo: "repo2".into(),
                session_id: "patrol-y".into(),
                status: SessionStatus::Running,
            })
            .await
            .is_ok());
    }

    // P0 审查后端#1：主动 kill——挂死的 agent 可被杀掉并释放写互斥
    #[tokio::test]
    async fn kill_active_session_terminates_and_releases_mutex() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        let s = mgr.start_induction("repo1", &dir, "t", "sleep", &["30".to_string()], None).await.unwrap();
        mgr.kill(&s.session_id).await.unwrap();
        let evt = recv_terminal(&mut rx).await;
        assert_eq!(evt.session_id, s.session_id);
        assert!(matches!(evt.status, SessionStatus::Failed));
        // 终态后再 kill → 409；互斥随终态释放（新会话可启动）
        assert!(matches!(mgr.kill(&s.session_id).await, Err(ApiError::Conflict(_))));
        assert!(mgr.start_induction("repo1", &dir, "t", "sleep", &["0".to_string()], None).await.is_ok());
        // 不存在的会话 → 404
        assert!(matches!(mgr.kill("nope").await, Err(ApiError::NotFound(_))));
    }

    // P0 审查后端#1：超时防线——agent 挂死也不会永占写互斥（无需人工 kill）
    #[tokio::test]
    async fn hung_agent_killed_by_timeout() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new_with_timeout(tx, std::time::Duration::from_millis(150));
        let dir = std::env::temp_dir();
        let s = mgr.start_induction("repo1", &dir, "t", "sleep", &["30".to_string()], None).await.unwrap();
        let evt = recv_terminal(&mut rx).await;
        assert_eq!(evt.session_id, s.session_id);
        assert!(matches!(evt.status, SessionStatus::Failed), "超时必须判 Failed");
        assert!(mgr.start_induction("repo1", &dir, "t", "sleep", &["0".to_string()], None).await.is_ok(), "互斥已释放");
        // 终态清理：首个会话的 kill 通道已回收（第二个会话自己的通道随其终态回收）
        assert!(!mgr.killers.read().await.contains_key(&s.session_id));
    }

    #[tokio::test]
    async fn try_register_race_is_closed() {
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let mgr = SessionManager::new(tx);
        let mk = |sid: &str| SessionStatusChanged {
            repo: "race".into(),
            session_id: sid.into(),
            status: SessionStatus::Running,
        };
        // 真并发双注册：必须恰好一个成功（审查 🔴2 的回归测试）
        let (a, b) = tokio::join!(mgr.try_register(mk("s1")), mgr.try_register(mk("s2")));
        let ok_count = [a.is_ok(), b.is_ok()].into_iter().filter(|x| *x).count();
        assert_eq!(ok_count, 1, "并发注册必须恰好一个成功");
    }

    #[tokio::test]
    async fn spawn_failure_releases_registration() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        // 不存在的命令 → spawn 失败 → 会话必须被判 Failed 并释放（审查 Y1）
        let err = mgr.start_induction("repo1", &dir, "t", "definitely-not-a-real-cmd-xyz", &[], None).await;
        assert!(matches!(err, Err(ApiError::Internal(_))));
        // 终态 Failed 事件已发布
        let mut saw_failed = false;
        while let Ok(evt) = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await {
            let Some(e) = evt else { break };
            if e.status == SessionStatus::Failed {
                saw_failed = true;
                break;
            }
        }
        assert!(saw_failed, "spawn 失败必须发布 Failed 终态");
        // 注册已释放：同仓库可再注册
        assert!(mgr.try_register(SessionStatusChanged { repo: "repo1".into(), session_id: "next".into(), status: SessionStatus::Running }).await.is_ok());
    }

    #[tokio::test]
    async fn session_lifecycle_succeeded() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        // true 命令立即成功退出
        let s = mgr.start_induction("repo1", &dir, "test", "true", &[], None).await.unwrap();
        assert_eq!(s.status, SessionStatus::Running);
        let final_evt = recv_terminal(&mut rx).await;
        assert_eq!(final_evt.status, SessionStatus::Succeeded);
        assert_eq!(final_evt.repo, "repo1");
    }

    #[tokio::test]
    async fn session_output_captured_for_result_parsing() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        // echo 忽略 stdin 直接打印——stdout 必须被捕获供 M4-1 解析 [EASYVIBE-RESULT]
        let s = mgr.start_induction("repo1", &dir, "ignored", "echo", &["hello-easyvibe [EASYVIBE-RESULT] {\"summary\":\"x\"}".to_string()], None).await.unwrap();
        let _ = recv_terminal(&mut rx).await;
        let out = mgr.output_of(&s.session_id).await.expect("stdout 应被捕获");
        assert!(out.contains("hello-easyvibe"), "捕获内容: {out}");
        assert!(out.contains("[EASYVIBE-RESULT]"), "归档行必须保留（即使超帽路径）");
        // 未知会话返回 None
        assert!(mgr.output_of("nope").await.is_none());
    }

    #[tokio::test]
    async fn session_lifecycle_failed() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        let _ = mgr.start_induction("repo1", &dir, "test", "false", &[], None).await.unwrap();
        let final_evt = recv_terminal(&mut rx).await;
        assert_eq!(final_evt.status, SessionStatus::Failed);
    }
}
