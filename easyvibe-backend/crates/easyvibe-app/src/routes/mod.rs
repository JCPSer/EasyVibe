//! 资源域路由子模块。共享助手在 crate::state，兄弟域之间禁止横向 use（守卫断言）。
pub(crate) mod agent;
pub(crate) mod chat;
pub(crate) mod dev_docs;
pub(crate) mod map;
pub(crate) mod repo;
pub(crate) mod sessions;
pub(crate) mod settings;
pub(crate) mod task;
