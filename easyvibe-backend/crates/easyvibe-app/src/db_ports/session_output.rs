//! 会话输出读取端口适配（重试续跑；单一端口域 = 会话输出行仓储）。

use crate::task_exec::ports::SessionOutputTail;
use easyvibe_common::ApiError;

/// 会话输出尾部适配（seq 倒序取行后还原升序，换行拼接；非破坏性只读）。
#[async_trait::async_trait]
impl SessionOutputTail for easyvibe_db::SessionOutputRepo {
    async fn tail(&self, session_id: &str, max_lines: i64) -> Result<String, ApiError> {
        let rows = easyvibe_db::SessionOutputRepo::fetch_tail(self, session_id, max_lines).await?;
        Ok(rows.into_iter().map(|r| r.line).collect::<Vec<_>>().join("\n"))
    }
}
