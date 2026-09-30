-- 任务→会话关联（M3-4 审批流的原料：知道这个任务对应哪次会话）
ALTER TABLE tasks ADD COLUMN session_id TEXT;
