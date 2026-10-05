//! PTY/进程生命周期：会话超时常量 + `start_induction`（spawn 外部 CLI agent、
//! stdin 注入 prompt、stdout/stderr 并发排空、select 竞速 kill/超时/退出、终态发布）。
//! 拆自 lib.rs（2026-10-05 防膨胀）。

use crate::{
    append_capture, parse_stream_event, parse_stream_meta, OutputStream, SessionManager,
    SessionMetaEvent, SessionMetaUpdate, SessionOutput,
};
use chrono::Utc;
use easyvibe_api_types::{SessionStatus, SessionStatusChanged};
use easyvibe_common::ApiError;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tracing::{info, warn};

/// 会话超时：覆盖归纳/巡检/子图分析/任务执行全部 spawn 路径（审查后端#1——agent 挂死 =
/// 写互斥永占 + 任务 permit 泄漏 + 重试循环空转）。agent 正常执行都在分钟级，30 分钟为宽限。
pub(crate) fn session_timeout() -> std::time::Duration {
    std::env::var("EASYVIBE_SESSION_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&s| s > 0)
        .map(std::time::Duration::from_secs)
        .unwrap_or(std::time::Duration::from_secs(30 * 60))
}
impl SessionManager {
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
        // B6：计数器重启归零会与历史行撞 id——纳秒尾缀保证全局唯一，人读仍带序号
        let uniq = Utc::now().timestamp_subsec_nanos() % 65_536;
        let session_id = format!("ind-{}-{uniq:04x}", self.counter.fetch_add(1, Ordering::SeqCst));
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

        // M1/U1：CLI 元事件 → 落盘桥写 agent_sessions 行（spawn 路径的 cli 真实来源）
        let _ = self.meta_tx.send(SessionMetaEvent {
            repo: repo_id.to_string(),
            session_id: session_id.clone(),
            update: SessionMetaUpdate::Cli(command.to_string()),
        });

        // stdout 捕获缓冲（M4-1）：任务终态后解析 [EASYVIBE-RESULT] 的原料
        let stdout_buf = Arc::new(std::sync::Mutex::new(String::new()));
        self.outputs.write().await.insert(session_id.clone(), stdout_buf.clone());
        let out_output_tx = self.output_tx.clone();
        let err_output_tx = self.output_tx.clone();
        // M2：每会话 seq 单调计数（stdout/stderr 共用一个——seq 是全会话的行序号）
        let out_seq = Arc::new(AtomicU64::new(0));
        let err_seq = out_seq.clone();
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
        // 2026-10-03 实弹：claude --output-format stream-json 时每行是一个 JSON 事件——
        // 还原成可读文本再进直播/缓冲，否则终端刷原始 JSON、[EASYVIBE-RESULT] 协议行
        // 也会被 JSON 转义而解析不到。由 args 自动探测，老参数（纯文本）行为不变。
        let stream_json = args.iter().any(|a| a.contains("stream-json"));
        let meta_tx = self.meta_tx.clone();
        let meta_tx2 = self.meta_tx.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt as _;
            let out_id = session_id_task.clone();
            let out_repo = repo.clone();
            let out_task = tokio::spawn(async move {
                let mut lines = 0u64;
                if let Some(mut s) = stdout {
                    let mut reader = tokio::io::BufReader::new(&mut s);
                    let mut line = String::new();
                    loop {
                        match reader.read_line(&mut line).await {
                            Ok(0) => break,
                            Ok(_) => {
                                // stream-json 模式：JSON 事件 → 可读文本（无可展示内容则跳过该行）；
                                // M1/U1：跳过的 system/result 事件在这里分派元数据去落盘
                                let payload: String = if stream_json {
                                    match parse_stream_event(&line) {
                                        Some(text) => text,
                                        None => {
                                            if let Some(update) = parse_stream_meta(&line) {
                                                let _ = meta_tx.send(SessionMetaEvent {
                                                    repo: out_repo.clone(),
                                                    session_id: out_id.clone(),
                                                    update,
                                                });
                                            }
                                            line.clear();
                                            continue;
                                        }
                                    }
                                } else {
                                    line.clone()
                                };
                                lines += 1;
                                if lines <= 3 || lines % 50 == 0 {
                                    let peek: String = payload.chars().take(200).collect();
                                    info!("[session {out_id}] stdout#{lines}: {peek}");
                                }
                                // 改进#2：过程直播——行截断 200 字符广播（行率不高，直接发）
                                for pl in payload.lines() {
                                    let _ = out_output_tx.send(SessionOutput {
                                        session_id: out_session_id.clone(),
                                        seq: out_seq.fetch_add(1, Ordering::SeqCst),
                                        stream: OutputStream::Stdout,
                                        line: pl.chars().take(200).collect(),
                                    });
                                }
                                // M4-1：捕获进缓冲（1MB 封顶，保头丢尾——RESULT 行在末尾）。
                                // stream-json 模式缓冲的是还原后的文本——协议行保持纯文本形态，
                                // take_output 消费方（RESULT/REVIEW 解析）零改动
                                append_capture(&stdout_buf, &payload);
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
                                    seq: err_seq.fetch_add(1, Ordering::SeqCst),
                                    stream: OutputStream::Stderr,
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
            let (status, _exit_code) = match outcome {
                Outcome::Killed(reason) => {
                    warn!("[session {session_id_task}] 被终止: {reason}");
                    if let Err(e) = child.start_kill() {
                        warn!("[session {session_id_task}] start_kill 失败: {e}");
                    }
                    let _ = child.wait().await;
                    (SessionStatus::Failed, None)
                }
                Outcome::Exited(Ok(exit)) => {
                    let code = exit.code();
                    if let Some(c) = code {
                        let _ = meta_tx2.send(SessionMetaEvent {
                            repo: repo.clone(),
                            session_id: session_id_task.clone(),
                            update: SessionMetaUpdate::ExitCode(c),
                        });
                    }
                    (
                        if exit.success() { SessionStatus::Succeeded } else { SessionStatus::Failed },
                        code.map(i64::from),
                    )
                }
                Outcome::Exited(Err(e)) => {
                    warn!("[session {session_id_task}] wait 失败: {e}");
                    (SessionStatus::Failed, None)
                }
            };
            // 子进程已退出：管道写端关闭，排空任务很快收尾
            let _ = tokio::join!(out_task, err_task);
            // 终态清理：kill 通道随会话结束回收（stdout 缓冲由消费者 take_output 领取，见 M4-1 契约）
            killers.write().await.remove(&session_id_task);
            let final_status = SessionStatusChanged { repo: repo.clone(), session_id: session_id_task.clone(), status };
            active.write().await.insert(repo.clone(), final_status.clone());
            by_id.write().await.insert(session_id_task.clone(), final_status.clone());
            // 非阻塞送达（背压卡死防线，同 try_register）
            if let Err(e) = events.try_send(final_status) {
                tracing::warn!("[session {session_id_task}] 终态事件通道已满，丢弃: {e}");
            }
            info!("[session {session_id_task}] 终态: {:?}", status);
        });

        Ok(SessionStatusChanged {
            repo: repo_id.to_string(),
            session_id,
            status: SessionStatus::Running,
        })
    }}
