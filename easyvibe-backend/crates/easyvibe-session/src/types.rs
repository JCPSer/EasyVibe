//! 会话领域的公开类型：事件发送端、输出行/流、元事件（model/usage/exit）。
//! 拆自 lib.rs（2026-10-05 防膨胀）。

use easyvibe_api_types::SessionStatusChanged;

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