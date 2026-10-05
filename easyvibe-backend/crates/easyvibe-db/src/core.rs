//! 跨子模块共用的数据库辅助：错误桥接、时间戳口径与注销清除。

use easyvibe_common::ApiError;

pub(crate) fn db_err(e: sqlx::Error) -> ApiError {
    ApiError::Internal(format!("db: {e}"))
}

pub(crate) fn now_secs() -> String {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string()
}

/// tasks 表时间戳统一口径：epoch 毫秒（create_task/try_advance_gate 即毫秒——
/// 2026-10-03 实弹 bug：update_status 等写秒，dev-docs 时间窗当毫秒解析导致产物全漏检）。
/// 会话/事件等内部自洽的表仍用 now_secs，不混用。
pub(crate) fn now_ms() -> String {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0).to_string()
}

/// 注销仓库的数据清除（2026-10-03 重审 P1）：抹掉该仓库在本地库的全部痕迹。
/// 删序：子表先、父表后（schema 无外键级联，全手动序）；settings 按 scope = repo id 清除。
/// 返回删除总行数（粗粒度观测值，日志用）。
pub async fn wipe_repo(pool: &sqlx::SqlitePool, repo: &str) -> Result<u64, ApiError> {
    let mut n = 0u64;
    macro_rules! del {
        ($sql:expr) => {
            n += sqlx::query($sql).bind(repo).execute(pool).await.map_err(db_err)?.rows_affected();
        };
    }
    del!("DELETE FROM module_health_history WHERE run_id IN (SELECT id FROM patrol_runs WHERE repo = ?)");
    del!("DELETE FROM patrol_runs WHERE repo = ?");
    del!("DELETE FROM events WHERE repo = ?");
    del!("DELETE FROM approvals WHERE task_id IN (SELECT id FROM tasks WHERE repo = ?)");
    del!("DELETE FROM conversation_messages WHERE conversation_id IN (SELECT id FROM conversations WHERE repo = ?)");
    del!("DELETE FROM conversations WHERE repo = ?");
    del!("DELETE FROM tasks WHERE repo = ?");
    del!("DELETE FROM settings WHERE scope = ?");
    Ok(n)
}
