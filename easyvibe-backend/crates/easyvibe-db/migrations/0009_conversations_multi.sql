-- M4-2 多会话：会话升级为"任务的上位容器"（v3 简报：AionUI 模式）
-- conversations.title：用户可命名的会话（缺省由前端按首问派生）
-- tasks.conversation_id：任务←→会话关联（升级路径 + 工作台聚合待审批/影响面）
ALTER TABLE conversations ADD COLUMN title TEXT;
ALTER TABLE tasks ADD COLUMN conversation_id TEXT;
CREATE INDEX IF NOT EXISTS idx_tasks_conv ON tasks(conversation_id);
