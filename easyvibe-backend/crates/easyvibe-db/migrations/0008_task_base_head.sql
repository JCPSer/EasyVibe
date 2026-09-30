-- 变更归因（实弹#3 记档限制兑现）：任务级 base_head 快照
-- 工作区脏时 git diff HEAD 会混入历史未提交改动；以任务启动时的 HEAD 为基准
-- 只统计本任务会话期间的变更
ALTER TABLE tasks ADD COLUMN base_head TEXT;
