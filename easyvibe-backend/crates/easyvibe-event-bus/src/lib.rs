//! event-bus：应用层共享的事件出口与运行会话队列状态机。
//!
//! 位于 application 层之下、foundation/contract 之上：server-api（路由/编排）与
//! task-engine（任务执行）各自**单向**依赖本 crate，二者之间不再互相 `crate::` 引用，
//! 从而让 import 图能判定应用层的依赖方向。
//!
//! 依赖纪律（守卫断言）：
//! - 只向下依赖 easyvibe-common / easyvibe-api-types；
//! - 禁止依赖任何 application 层 crate（easyvibe-app / easyvibe-session /
//!   easyvibe-db / easyvibe-map / easyvibe-ai-agent）；
//! - 禁止引入 axum（事件与队列是纯应用层能力，不绑 HTTP 框架）。
pub mod bus;
pub mod queue;

pub use bus::{publish, BusEvent};
pub use queue::{JobKind, QueueChange, QueueHost, QueueState, QueuedJob};
