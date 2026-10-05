//! 会话领域：外部 agent 的统一会话抽象（M2-3 先落地 CLI 后端；ACP 后端后续挂同一边界，
//! 参考 AionCore aionui-session 的"直连 CLI 与 ACP 统一会话"思路）。
//!
//! 职责边界（与 easyvibe-map 分工）：
//! - 本 crate 只管"把 agent 跑起来、看住它、报状态"
//! - 四通道文件（progress/growth.log/parts/map.json）的消费由 easyvibe-map 的 watcher 负责，
//!   agent 只要遵守 v2.2 协议写文件，后端不关心它怎么写
//!
//! 门面（2026-10-05 防膨胀拆分）：本文件只留 SessionManager 结构 + 构造器 + mod/reexport；
//! 公开类型归 types，输出解析归 output，进程/看门归 pty，生命周期/状态机归 lifecycle。
//! 对外 `pub` 面逐字保持。

mod types;
mod output;
mod pty;
mod lifecycle;

pub use types::{OutputStream, SessionEventSender, SessionMetaEvent, SessionMetaUpdate, SessionOutput};
pub(crate) use output::{append_capture, parse_stream_event, parse_stream_meta};

use chrono::{DateTime, Utc};
use easyvibe_api_types::SessionStatusChanged;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

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
    output_tx: Arc<broadcast::Sender<SessionOutput>>,
    /// M1/U1：会话元事件广播（model/usage/exit——落盘桥在 app 层订阅写 agent_sessions）
    meta_tx: Arc<broadcast::Sender<SessionMetaEvent>>,
    /// 会话超时（秒）：超时未终态 → 杀进程判 Failed。环境可调（EASYVIBE_SESSION_TIMEOUT_SECS），默认 30 分钟。
    timeout: std::time::Duration,
    /// I1 旁路表①：session_id → 注册时刻（UTC）。不改 SessionStatusChanged 结构，
    /// startedAt 由 try_register 唯一汇聚点盖戳——各发起入口无需各自记时间
    started: Arc<RwLock<HashMap<String, DateTime<Utc>>>>,
    /// I1 旁路表②：session_id → 展示标签（「归纳」「巡检」「分析模块 X」…）。
    /// 由 app 层 note_label 打标；终态 publish 时与 started 一并清理
    labels: Arc<RwLock<HashMap<String, String>>>,
}

impl SessionManager {
    pub fn new(events: SessionEventSender) -> Arc<Self> {
        let (output_tx, _) = broadcast::channel(512);
        let (meta_tx, _) = broadcast::channel(256);
        Arc::new(Self {
            active: Default::default(),
            by_id: Default::default(),
            outputs: Default::default(),
            killers: Default::default(),
            counter: AtomicU64::new(0),
            events,
            output_tx: Arc::new(output_tx),
            meta_tx: Arc::new(meta_tx),
            timeout: pty::session_timeout(),
            started: Default::default(),
            labels: Default::default(),
        })
    }

    /// 测试专用：显式超时常数（env 在并行测试下变异是竞态，不走 EASYVIBE_SESSION_TIMEOUT_SECS）
    #[cfg(test)]
    pub fn new_with_timeout(events: SessionEventSender, timeout: std::time::Duration) -> Arc<Self> {
        let (output_tx, _) = broadcast::channel(512);
        let (meta_tx, _) = broadcast::channel(256);
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
}
