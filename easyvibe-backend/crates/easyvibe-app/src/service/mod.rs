//! 应用服务编排层（方案 R2 批 B / S7）：把原 handler 内联的 100–200 行业务编排抽成
//! 显式入参的服务函数，handler 退化为「解析入参 → 调 service → 映射错误/响应」。
//!
//! 依赖方向（单向，不引用 routes，避免二次重叠）：
//!   routes/*.rs（HTTP 边界） → service（编排） → state.rs / 各 domain crate
//!
//! 按资源域拆分（方案 R1）：chat / task 编排原样搬迁自 `service.rs`，map 编排原样
//! 搬迁自 `routes/map.rs`，**零语义改动**。`crate::service::<fn>` 对外路径经 re-export 保持全等。

pub(crate) mod chat;
pub(crate) mod git;
pub(crate) mod map;
pub(crate) mod reinduce;
pub(crate) mod repo;
pub(crate) mod task;

// chat/task 编排的既有 `crate::service::<fn>` 调用路径经 re-export 保持全等；
// map 不 re-export——强制唯一规范路径 `crate::service::map::<fn>`，使 R7 逆向引用
// 守卫（`crate::service::map::start_`）不存在 `crate::service::start_*` 别名绕过面。
pub(crate) use chat::*;
pub(crate) use task::*;
