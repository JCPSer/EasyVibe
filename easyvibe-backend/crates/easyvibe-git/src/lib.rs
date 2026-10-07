//! git 工作树领域：状态/历史查询 + 写操作（提交/拉取/推送/撤销）。
//!
//! 命令统一走 tokio::process + 超时；解析逻辑全部是纯函数，单测覆盖（含真 git 集成测试）。
//! 安全：写操作只接受相对路径且禁止 `..` 越界；discard 按跟踪状态区分 checkout/clean。
//!
//! 边界（由 `tests/module_size_guard.rs` 的禁用串表断言）：只有领域规则——
//! 不含 HTTP 框架、应用共享状态、持久层仓储、agent 客户端，也不反向依赖应用层 crate。
pub mod exec;
pub mod model;
pub mod ops;
pub mod parse;

pub use exec::{git, git_opt, GIT_TIMEOUT};
pub use model::{CommitDetail, CommitFileStat, FileDiff, GitFile, GitLogRow, GitStatus};
pub use ops::{commit_all, diff, discard, discard_all, log, pull, push, show_commit, status, DIFF_MAX_LINES};
pub use parse::{parse_log, parse_numstat, parse_porcelain, validate_rel_path};

#[cfg(test)]
mod tests;
