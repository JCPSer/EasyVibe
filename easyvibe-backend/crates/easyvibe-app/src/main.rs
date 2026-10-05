//! 组装层：二进制入口，REST 路由 + WS 事件推送。
//!
//! 拆分后本文件只做「装配」：声明子模块、导出共享符号、跑 bootstrap。
//! 资源域 handler 在 `routes/*`，共享状态/助手在 `state`，路由表在 `router`，
//! WS 面在 `ws`，启动装配在 `bootstrap`，资产在 `assets`。
mod assets;
mod bootstrap;
mod freshness;
mod git;
mod pipeline;
mod router;
mod routes;
mod session_queue_routes;
mod state;
mod task_exec;
mod ws;

// 解环：agent_conf 已下沉 easyvibe-ai-agent；session_queue 拆分为
// 「路由/请求面」（本 crate 的 session_queue_routes）与「队列状态机」（easyvibe-event-bus）。

/// 既有 `crate::{AppState, AppError, LlmMode, resolve_llm}` 调用路径经门面保持不漂移
/// （git.rs / session_queue_routes.rs 零改动）。
pub(crate) use state::{resolve_llm, AppError, AppState, LlmMode};

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() {
    bootstrap::run().await;
}

#[cfg(test)] mod test_support;
#[cfg(test)] mod tests_a;
#[cfg(test)] mod tests_b;
#[cfg(test)] mod tests_c;
#[cfg(test)] mod tests_d;
