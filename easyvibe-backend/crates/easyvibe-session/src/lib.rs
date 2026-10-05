//! 会话领域：外部 agent 的统一会话抽象（M2-3 先落地 CLI 后端；ACP 后端后续挂同一边界，
//! 参考 AionCore aionui-session 的"直连 CLI 与 ACP 统一会话"思路）。
//!
//! 职责边界（与 easyvibe-map 分工）：
//! - 本 crate 只管"把 agent 跑起来、看住它、报状态"
//! - 四通道文件（progress/growth.log/parts/map.json）的消费由 easyvibe-map 的 watcher 负责，
//!   agent 只要遵守 v2.2 协议写文件，后端不关心它怎么写
use easyvibe_api_types::{SessionStatus, SessionStatusChanged};
use easyvibe_common::ApiError;
use chrono::{DateTime, Utc};
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

/// 会话 stdout/stderr 行（改进#2"分析中黑洞"——agent 过程直播的原料；行已截断）。
/// M2 扩展：seq = 每会话单调序号（落盘回放/断线补拉的幂等锚点，看门任务内 AtomicU64）；
/// stream = stdout/stderr（stderr 行仍带 [err] 前缀，stream 用于终端档着色）。
#[derive(Debug, Clone)]
pub struct SessionOutput {
    pub session_id: String,
    pub seq: u64,
    pub stream: OutputStream,
    pub line: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

/// M1/U1 会话元事件（2026-10-05）：真实模型名 / token 用量 / 退出码，看门任务边读边分派。
/// 设计约束（docs/llm-usage-page-design-v1.md §1.2）：result 事件不进 1MB 缓冲、
/// 终态后补解析不可行——解析必须在读行循环内做；被 kill 的会话无 result 事件，usage 恒 NULL。
/// 首期只解析 claude stream-json；其余 CLI 的 usage 列留 NULL 诚实降级。
#[derive(Debug, Clone)]
pub enum SessionMetaUpdate {
    /// spawn 入口已知（命令名）——落盘桥据此刻写 agent_sessions 行
    Cli(String),
    /// app 层 note_label 即播——运行期落库 label/kind（此前仅终态 finalize 才写，
    /// 运行中的会话在用量/运行页显示 unknown/归纳误标，2026-10-05 实弹）
    Label(String),
    /// system init 事件的真实模型名（不等终态，kill 也能拿到）
    Model(String),
    /// result 事件的会话级累计 usage（claude 自报口径，含缓存折扣价）
    Usage {
        input_tokens: i64,
        output_tokens: i64,
        cache_read_tokens: i64,
        cache_write_tokens: i64,
        cost_usd: f64,
        duration_ms: i64,
        turns: i64,
    },
    /// 进程退出码（被 kill/超时无退出码则不分派）
    ExitCode(i32),
}

#[derive(Debug, Clone)]
pub struct SessionMetaEvent {
    pub repo: String,
    pub session_id: String,
    pub update: SessionMetaUpdate,
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
    /// M1/U1：会话元事件广播（model/usage/exit——落盘桥在 app 层订阅写 agent_sessions）
    meta_tx: Arc<tokio::sync::broadcast::Sender<SessionMetaEvent>>,
    /// 会话超时（秒）：超时未终态 → 杀进程判 Failed。环境可调（EASYVIBE_SESSION_TIMEOUT_SECS），默认 30 分钟。
    timeout: std::time::Duration,
    /// I1 旁路表①：session_id → 注册时刻（UTC）。不改 SessionStatusChanged 结构，
    /// startedAt 由 try_register 唯一汇聚点盖戳——各发起入口无需各自记时间
    started: Arc<RwLock<HashMap<String, DateTime<Utc>>>>,
    /// I1 旁路表②：session_id → 展示标签（「归纳」「巡检」「分析模块 X」…）。
    /// 由 app 层 note_label 打标；终态 publish 时与 started 一并清理
    labels: Arc<RwLock<HashMap<String, String>>>,
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

/// claude `--output-format stream-json` 事件 → 可读文本。
/// 只取 assistant 事件的正文/思考块（进度可见）；result 事件是全量回放，跳过防双份。
/// 返回 None = 该事件无可展示内容（system/user/result 或解析失败）。
fn parse_stream_event(line: &str) -> Option<String> {
    let ev: serde_json::Value = serde_json::from_str(line).ok()?;
    if ev["type"].as_str() != Some("assistant") {
        return None;
    }
    let content = &ev["message"]["content"];
    let blocks = content.as_array().cloned().unwrap_or_default();
    let mut out = String::new();
    if blocks.is_empty() {
        // 兼容 content 为纯字符串的形态
        if let Some(t) = content.as_str() {
            out.push_str(t);
        }
    }
    for b in blocks {
        match b["type"].as_str() {
            Some("text") => {
                if let Some(t) = b["text"].as_str() {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(t);
                }
            }
            Some("thinking") => {
                if let Some(t) = b["thinking"].as_str() {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str("[思考] ");
                    out.push_str(t);
                }
            }
            _ => {}
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// M1/U1：stream-json 事件 → 会话元更新（system init 抽模型名 / result 抽 usage）。
/// 与 parse_stream_event 互补：那边跳过的 system/result，在这里被分派去落盘——
/// 文本归直播，元数据归 agent_sessions，互不干扰。
fn parse_stream_meta(line: &str) -> Option<SessionMetaUpdate> {
    let ev: serde_json::Value = serde_json::from_str(line).ok()?;
    match ev["type"].as_str()? {
        "system" if ev["subtype"].as_str() == Some("init") => {
            ev["model"].as_str().map(|m| SessionMetaUpdate::Model(m.to_string()))
        }
        "result" => {
            let usage = &ev["usage"];
            // cost_usd 为必填门槛：连自报成本都没有的事件不更新 usage 列（保持 NULL 语义）
            let cost_usd = ev["total_cost_usd"].as_f64()?;
            Some(SessionMetaUpdate::Usage {
                input_tokens: usage["input_tokens"].as_i64()?,
                output_tokens: usage["output_tokens"].as_i64()?,
                cache_read_tokens: usage["cache_read_input_tokens"].as_i64().unwrap_or(0),
                cache_write_tokens: usage["cache_creation_input_tokens"].as_i64().unwrap_or(0),
                cost_usd,
                duration_ms: ev["duration_ms"].as_i64().unwrap_or(0),
                turns: ev["num_turns"].as_i64().unwrap_or(0),
            })
        }
        _ => None,
    }
}

impl SessionManager {
    pub fn new(events: SessionEventSender) -> Arc<Self> {
        let (output_tx, _) = tokio::sync::broadcast::channel(512);
        let (meta_tx, _) = tokio::sync::broadcast::channel(256);
        Arc::new(Self {
            active: Default::default(),
            by_id: Default::default(),
            outputs: Default::default(),
            killers: Default::default(),
            counter: AtomicU64::new(0),
            events,
            output_tx: Arc::new(output_tx),
            meta_tx: Arc::new(meta_tx),
            timeout: session_timeout(),
            started: Default::default(),
            labels: Default::default(),
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
        let (meta_tx, _) = tokio::sync::broadcast::channel(256);
        Arc::new(Self {
            active: Default::default(),
            by_id: Default::default(),
            outputs: Default::default(),
            killers: Default::default(),
            counter: AtomicU64::new(100), // 与 new() 的计数器错开，避免测试间 session id 冲突
            events,
            output_tx: Arc::new(output_tx),
            meta_tx: Arc::new(meta_tx),
            timeout,
            started: Default::default(),
            labels: Default::default(),
        })
    }

    /// grace 收尸（2026-10-03 重审 P1：归纳进度 100% 后 agent 不退出 = 僵尸进程泄漏）：
    /// 杀进程，但终态必须记 Succeeded——产物已交付，判 Failed 是冤案（实弹#4 的原始诉求）。
    /// 顺序保证：notify kill → 轮询等看门狗写出终态（Failed）→ note_status 覆盖为 Succeeded。
    /// 覆盖放在"观察到终态之后"，消除与看门狗终态写的竞态。
    pub async fn grace_finish(&self, session_id: &str) -> Result<(), ApiError> {
        let s = self.by_id.read().await.get(session_id).cloned().ok_or_else(|| ApiError::NotFound(format!("会话 {session_id} 不存在")))?;
        if !matches!(s.status, SessionStatus::Starting | SessionStatus::Running) {
            return Err(ApiError::Conflict(format!("会话 {session_id} 已终态（{:?}）", s.status)));
        }
        let notify = self.killers.read().await.get(session_id).cloned().ok_or_else(|| {
            ApiError::Conflict(format!("会话 {session_id} 无终止通道（外部注册会话不支持 kill）"))
        })?;
        notify.notify_one();
        // 等看门狗把 Killed→Failed 的终态写出来（kill 是即时的，2.5s 上限宽到离谱）
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if let Some(cur) = self.by_id.read().await.get(session_id) {
                if !matches!(cur.status, SessionStatus::Starting | SessionStatus::Running) {
                    break;
                }
            }
        }
        self.note_status(SessionStatusChanged {
            repo: s.repo,
            session_id: session_id.to_string(),
            status: SessionStatus::Succeeded,
        })
        .await;
        Ok(())
    }

    /// 订阅会话 stdout 行（app 层翻译为 WS session.output）
    pub fn subscribe_output(&self) -> tokio::sync::broadcast::Receiver<SessionOutput> {
        self.output_tx.subscribe()
    }

    /// M1/U1：订阅会话元事件（app 层落盘 agent_sessions 的数据源）
    pub fn subscribe_meta(&self) -> tokio::sync::broadcast::Receiver<SessionMetaEvent> {
        self.meta_tx.subscribe()
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

    /// I1：app 层各发起入口打展示标签（「归纳」「巡检」「分析模块 X」「自动归纳」「任务执行」）
    pub async fn note_label(&self, session_id: &str, label: String) {
        self.labels.write().await.insert(session_id.to_string(), label.clone());
        // M1.1：标签同时经元事件落库（运行期 kind/label 可见——用量/运行页不再等终态）
        let repo_id = self
            .by_id
            .read()
            .await
            .get(session_id)
            .map(|s| s.repo.clone())
            .unwrap_or_default();
        let _ = self.meta_tx.send(SessionMetaEvent {
            repo: repo_id,
            session_id: session_id.to_string(),
            update: SessionMetaUpdate::Label(label),
        });
    }

    /// I1：会话注册时刻（气泡已运行时长的数据源；缺失则不显示时长）
    pub async fn started_at_of(&self, session_id: &str) -> Option<DateTime<Utc>> {
        self.started.read().await.get(session_id).copied()
    }

    /// I1：会话展示标签（缺失时前端降级显示「会话 {id}」）
    pub async fn label_of(&self, session_id: &str) -> Option<String> {
        self.labels.read().await.get(session_id).cloned()
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
        // I1：注册即盖 startedAt 戳（唯一汇聚点——所有 spawn/外部注册路径都经这里）
        self.started.write().await.insert(s.session_id.clone(), Utc::now());
        // 非阻塞送达：通道满（下游死亡/测试无消费者）时事件丢弃——
        // 会话注册绝不能被监控通道背压卡死（2026-10-03 实弹：阶段初审让每任务
        // 会话数 4→6，测试 channel(16) 被填满，第 16 个 send 永久阻塞注册，死锁）
        if let Err(e) = self.events.try_send(s) {
            tracing::warn!("[session] 状态事件通道已满，丢弃事件（下游可能已死亡）: {e}");
        }
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
                                if let Ok(mut buf) = stdout_buf.lock() {
                                    if buf.len() < 1_048_576 {
                                        buf.push_str(&payload);
                                        if !payload.ends_with('\n') {
                                            buf.push('\n');
                                        }
                                    } else if payload.contains("[EASYVIBE-RESULT]") {
                                        // 超帽时仍保留 RESULT 归档行（短行，替换式保底）
                                        let trimmed: String = payload.chars().take(4096).collect();
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
    }

    async fn set_status(&self, repo: &str, session_id: &str, status: SessionStatus) {
        let s = SessionStatusChanged { repo: repo.into(), session_id: session_id.into(), status };
        self.publish(s).await;
    }

    /// 状态发布（B1 护栏在此）：`active[repo]` 已存在**不同 session_id 的活动会话**
    /// （Starting/Running）时，本条迟到事件（含 grace 收尸的迟到 Succeeded 终态）
    /// 不得覆写 active——整条事件丢弃 + warn。同 session_id 的正常状态推进不受限；
    /// by_id 按 session_id 键，天然不受跨会话影响。
    async fn publish(&self, s: SessionStatusChanged) {
        {
            let mut active = self.active.write().await;
            if let Some(existing) = active.get(&s.repo) {
                if existing.session_id != s.session_id
                    && matches!(existing.status, SessionStatus::Starting | SessionStatus::Running)
                {
                    tracing::warn!(
                        "[session] B1 护栏：丢弃迟到事件——仓库 {} 已有活动会话 {}，事件 {}（{:?}）不得覆写 active",
                        s.repo, existing.session_id, s.session_id, s.status
                    );
                    return;
                }
            }
            active.insert(s.repo.clone(), s.clone());
        }
        self.by_id.write().await.insert(s.session_id.clone(), s.clone());
        // B4（M1，2026-10-05）：终态不再删旁路表——startedAt/label 是 runs 页列表与
        // agent_sessions finalize 的兜底来源；两张表仅按会话数自然增长（每项几十字节，可控）。
        // （原清理逻辑移除；测试 bypass_tables_* 已同步改期望）
        // 非阻塞送达（背压卡死防线，同 try_register）
        if let Err(e) = self.events.try_send(s) {
            tracing::warn!("[session] 状态事件通道已满，丢弃事件: {e}");
        }
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

    // B1 护栏单测：迟到事件不得覆写新活动会话（整条丢弃，by_id 不受影响）
    #[tokio::test]
    async fn publish_guard_drops_late_event_from_other_session() {
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let mgr = SessionManager::new(tx);
        let mk = |sid: &str, status: SessionStatus| SessionStatusChanged {
            repo: "g1".into(),
            session_id: sid.into(),
            status,
        };
        // S1 走完终态 → S2 抢注为新活动会话
        mgr.note_status(mk("s1", SessionStatus::Failed)).await;
        mgr.try_register(mk("s2", SessionStatus::Running)).await.unwrap();
        // 迟到的非终态事件（同仓库旧会话）必须被丢弃
        mgr.note_status(mk("s1", SessionStatus::Running)).await;
        let cur = mgr.status_of("g1").await.unwrap();
        assert_eq!(cur.session_id, "s2", "迟到事件不得顶掉新活动会话");
        assert_eq!(cur.status, SessionStatus::Running);
        // grace 乱序：迟到的 Succeeded 终态同样不得覆写（B1 原始场景）
        mgr.note_status(mk("s1", SessionStatus::Succeeded)).await;
        let cur = mgr.status_of("g1").await.unwrap();
        assert_eq!(cur.session_id, "s2", "grace 迟到 Succeeded 不得顶掉新活动会话");
        assert_eq!(cur.status, SessionStatus::Running);
        // by_id 按 session_id 键：s1 保留自己的终态归属（try_register 不写 by_id，s2 经 status_of 观测）
        assert_eq!(mgr.status_of_session("s1").await.unwrap().status, SessionStatus::Failed);
        // 同 session_id 的正常推进不受限
        mgr.note_status(mk("s2", SessionStatus::Succeeded)).await;
        assert_eq!(mgr.status_of("g1").await.unwrap().status, SessionStatus::Succeeded);
    }

    // I1 旁路表 + B4（M1）：try_register 盖 startedAt 戳、note_label 打标；
    // 终态 publish 不再清理两表（历史可查，见 publish 注释）
    #[tokio::test]
    async fn bypass_tables_stamp_label_and_keep_on_terminal() {
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let mgr = SessionManager::new(tx);
        mgr.try_register(SessionStatusChanged {
            repo: "b1".into(),
            session_id: "s-1".into(),
            status: SessionStatus::Running,
        })
        .await
        .unwrap();
        let started = mgr.started_at_of("s-1").await.expect("注册必须盖 startedAt 戳");
        assert!((Utc::now() - started).num_seconds() < 5, "戳应为刚才: {started}");
        assert!(mgr.label_of("s-1").await.is_none());
        mgr.note_label("s-1", "归纳".into()).await;
        assert_eq!(mgr.label_of("s-1").await.as_deref(), Some("归纳"));
        // 活动期两表都在
        mgr.note_status(SessionStatusChanged { repo: "b1".into(), session_id: "s-1".into(), status: SessionStatus::Running }).await;
        assert!(mgr.started_at_of("s-1").await.is_some());
        assert!(mgr.label_of("s-1").await.is_some());
        // 终态 publish 保留（B4：runs 页列表与 agent_sessions finalize 的兜底来源）
        mgr.note_status(SessionStatusChanged { repo: "b1".into(), session_id: "s-1".into(), status: SessionStatus::Succeeded }).await;
        assert!(mgr.started_at_of("s-1").await.is_some(), "终态后 started 表项保留（B4）");
        assert!(mgr.label_of("s-1").await.is_some(), "终态后 label 表项保留（B4）");
        // 无竞争会话时 grace 路径的 note_status(Succeeded) 仍全量生效（既有行为不受护栏影响）
        mgr.try_register(SessionStatusChanged { repo: "b2".into(), session_id: "s-2".into(), status: SessionStatus::Running }).await.unwrap();
        mgr.note_status(SessionStatusChanged { repo: "b2".into(), session_id: "s-2".into(), status: SessionStatus::Succeeded }).await;
        assert_eq!(mgr.status_of("b2").await.unwrap().status, SessionStatus::Succeeded);
    }

    #[test]
    fn stream_meta_extracts_model_and_usage() {
        // system init → Model（不等终态）
        let init = r#"{"type":"system","subtype":"init","model":"claude-sonnet-4-6"}"#;
        match parse_stream_meta(init) {
            Some(SessionMetaUpdate::Model(m)) => assert_eq!(m, "claude-sonnet-4-6"),
            other => panic!("init 事件应解析出模型名: {other:?}"),
        }
        // result → Usage（claude 自报口径：cost/tokens/时长/轮数）
        let result = r#"{"type":"result","model":"claude-sonnet-4-6","total_cost_usd":0.0421,"duration_ms":52000,"num_turns":3,
            "usage":{"input_tokens":12000,"output_tokens":3000,"cache_read_input_tokens":8000,"cache_creation_input_tokens":4000}}"#;
        match parse_stream_meta(result) {
            Some(SessionMetaUpdate::Usage { input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cost_usd, duration_ms, turns }) => {
                assert_eq!(input_tokens, 12000);
                assert_eq!(output_tokens, 3000);
                assert_eq!(cache_read_tokens, 8000);
                assert_eq!(cache_write_tokens, 4000);
                assert!((cost_usd - 0.0421).abs() < 1e-9);
                assert_eq!(duration_ms, 52000);
                assert_eq!(turns, 3);
            }
            other => panic!("result 事件应解析出 usage: {other:?}"),
        }
        // 无 total_cost_usd → 不更新（usage 列保持 NULL 语义）
        let no_cost = r#"{"type":"result","usage":{"input_tokens":1,"output_tokens":1}}"#;
        assert!(parse_stream_meta(no_cost).is_none());
        // assistant/垃圾行不归元分派
        assert!(parse_stream_meta(r#"{"type":"assistant"}"#).is_none());
        assert!(parse_stream_meta("not json").is_none());
    }

    #[test]
    fn stream_event_extracts_assistant_text_and_skips_result() {
        // stream-json → 文本还原：assistant 正文/思考块提取；result/system 跳过（防双份）
        let text_ev = r###"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"先读文件"},{"type":"text","text":"## 交付摘要\n改动完成"}]}}"###;
        let out = parse_stream_event(text_ev).expect("assistant 事件必须有文本");
        assert!(out.contains("[思考] 先读文件"), "思考块保留: {out}");
        assert!(out.contains("## 交付摘要"), "正文保留: {out}");

        // 协议行在文本块内保持纯文本形态——take_output 的 RESULT/REVIEW 解析零改动
        let proto_ev = r###"{"type":"assistant","message":{"content":[{"type":"text","text":"完成\n[EASYVIBE-RESULT] {\"summary\":\"ok\"}"}]}}"###;
        let out = parse_stream_event(proto_ev).unwrap();
        assert!(out.lines().any(|l| l.starts_with("[EASYVIBE-RESULT]")), "协议行必须是独立纯文本行: {out}");

        // result 事件 = 全量回放，必须跳过（否则双份）
        let result_ev = r#"{"type":"result","result":"全部内容回放"}"#;
        assert!(parse_stream_event(result_ev).is_none());
        // system 事件跳过
        let sys_ev = r#"{"type":"system","subtype":"init"}"#;
        assert!(parse_stream_event(sys_ev).is_none());
        // 非 JSON 行（异常输出）→ None（不广播原始垃圾）
        assert!(parse_stream_event("not json at all").is_none());
    }
}
