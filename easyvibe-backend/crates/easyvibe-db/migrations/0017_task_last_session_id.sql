-- 重试续跑（B 案上下文接续）：任务行记录上一次执行的会话 id
-- retry() 在 reset_for_retry（会把 session_id 置 NULL）之前抢救写入；
-- spawn 时按它读上次会话输出尾部注入 prompt 接续块，实现「从中断处继续」
ALTER TABLE tasks ADD COLUMN last_session_id TEXT;
