//! 组合根适配器族（c-arch-13 R1）：按**端口域**拆分为多个适配器文件。
//!
//! 本目录是**唯一**允许同时看见 `task_exec::ports` / service 端口与具体仓储（`easyvibe_db`）
//! 的地方；适配器落组合根保证依赖方向仍为 server-api → task-engine（端口在依赖方、实现在
//! 编排层，与 `impl QueueHost for AppState` 同构）。task_exec 生产文件因此零直连、零反向引用。
//!
//! 拆分纪律（c-arch-13）：纯搬家，零语义改动；`crate::db_ports::<Port|DTO>` 路径经下方
//! `pub(crate) use` 再导出**保持全等**，service/**、routes/** 调用点零改动。
//! 每个适配器文件只承载**单一端口域**，改任一表字段无需通读整个组合根。

mod approval;
mod conversation;
mod dto;
mod event;
mod health;
mod repo;
mod settings;
mod task;
mod task_engine;

pub(crate) use approval::*;
pub(crate) use conversation::*;
pub(crate) use dto::*;
pub(crate) use event::*;
pub(crate) use health::*;
pub(crate) use repo::*;
pub(crate) use settings::*;
pub(crate) use task::*;
pub(crate) use task_engine::*;

// ============================================================================
// c-arch-10 R4：service/** 编排层消费的持久化端口（**扩展 trait**）与本地 DTO
// ----------------------------------------------------------------------------
// 为什么是「扩展 trait + 具体类型字段」而非 `Arc<dyn …>`：`agent_conf::resolve_agent(
// &SqliteSettingsRepository, …)` 与 `PatrolService<R: HealthRepository>` 都接受**具体/泛型具体**类型，
// AppState 字段改 dyn 会连带破坏这 6 处组合 ⇒ 唯一零 `dyn` 路径是「impl XPort for 具体仓储」。
// service/** 只见 `crate::db_ports::*Port` 与本地 DTO，不再直连 `easyvibe_db`。
// 组合根（state.rs / db_ports.rs / assembly/** / 测试面）仍是 `easyvibe_db` 的唯一合法落点。
// ============================================================================
