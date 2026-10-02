-- R3 D2：返工链结构化（Harness repair loop 与返工率的输入）
-- origin_task_id：本任务由哪个任务复制返工而来（驳回→复制新任务的来源）
-- successor_task_id：原任务视角的反链——驳回次数聚合与返工率统计的命脉
ALTER TABLE tasks ADD COLUMN origin_task_id TEXT;
ALTER TABLE tasks ADD COLUMN successor_task_id TEXT;
