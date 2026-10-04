-- M1 归因扩展（L1 治理账单，2026-10-05）：
-- 会话花费归到架构模块头上——任务会话经 task_id→tasks.modules 反查回填，
-- 子图会话 spawn 时直接写入。用量页「按模块 TopN」与模块详情「治理账单」的归因列。
ALTER TABLE agent_sessions ADD COLUMN module_id TEXT;
CREATE INDEX IF NOT EXISTS idx_agent_sessions_module ON agent_sessions(repo, module_id, started_at);
