-- M4-1：任务产物归档采集（F6 变更透明化供料）
-- tasks.result = JSON：{summary, changedModules, archivedPath, diffStat, collectedAt}
-- diff 关审批时前端据此展示"改了什么"，无需再碰 agent 会话
ALTER TABLE tasks ADD COLUMN result TEXT;
