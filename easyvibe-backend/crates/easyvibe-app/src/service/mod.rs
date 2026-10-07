//! 应用服务编排层（方案 R2 批 B / S7）：把原 handler 内联的 100–200 行业务编排抽成
//! 显式入参的服务函数，handler 退化为「解析入参 → 调 service → 映射错误/响应」。
//!
//! 依赖方向（单向，不引用 routes，避免二次重叠）：
//!   routes/*.rs（HTTP 边界） → service（编排） → state.rs / 各 domain crate
//!
//! 按资源域拆分（方案 R1）：chat / task 编排原样搬迁自 `service.rs`，map 编排原样
//! 搬迁自 `routes/map.rs`，**零语义改动**。`crate::service::<fn>` 对外路径经 re-export 保持全等。

pub(crate) mod agent;
pub(crate) mod chat;
pub(crate) mod git;
pub(crate) mod map;
pub(crate) mod patrol;
pub(crate) mod reinduce;
pub(crate) mod reinduce_start;
pub(crate) mod submap;
pub(crate) mod repo;
pub(crate) mod sessions;
pub(crate) mod settings;
pub(crate) mod task;

// chat/task 编排的既有 `crate::service::<fn>` 调用路径经 re-export 保持全等；
// map/patrol/submap/reinduce(_start) 一概不 re-export——强制唯一规范路径
// （`crate::service::{map,patrol,submap,reinduce,reinduce_start}::<fn>`），使逆向引用守卫
// （c-arch-13：`crate::service::patrol::start_` / `crate::service::reinduce_start::start_` /
//  `crate::service::submap::analyze_submap`）不存在 `crate::service::<fn>` 别名绕过面。
pub(crate) use chat::*;
pub(crate) use task::*;
