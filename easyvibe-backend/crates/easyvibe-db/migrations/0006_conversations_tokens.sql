-- M3-5：会话持久化（对话历史落域 2）+ token 用量记录（§10 #4）
-- 对话：每仓库一个会话；完整原文留痕（compacted 标记区分运行态 vs 归档态）；
-- 压缩摘要存 conversations.summary（回放从库读 = 完整原文，运行上下文 = 摘要+近期窗口）

CREATE TABLE IF NOT EXISTS conversations (
  id TEXT PRIMARY KEY,              -- 目前一仓一会话："chat:{repo}"
  repo TEXT NOT NULL,
  summary TEXT,                     -- LLM 结构化压缩摘要（含会话状态：未决问题/决策）
  compacted_before INTEGER NOT NULL DEFAULT 0,  -- 已压缩的消息水位（message rowid ≤ 此值均为 compacted=1）
  prompt_tokens INTEGER NOT NULL DEFAULT 0,     -- 累计用量（成本护栏第一步，§10 #4）
  completion_tokens INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS conversation_messages (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  conversation_id TEXT NOT NULL REFERENCES conversations(id),
  role TEXT NOT NULL,               -- user / assistant / system
  content TEXT NOT NULL,
  compacted INTEGER NOT NULL DEFAULT 0,  -- 1 = 已折叠进摘要（原文保留，留痕可回放）
  tokens INTEGER NOT NULL DEFAULT 0,     -- 估算 token（estimate：CJK 混合 ~1 token / 2 字符）
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_conv_msgs ON conversation_messages (conversation_id, id);

-- token 用量列（patrol_runs / tasks；会话型 CLI agent 无法回报则留 NULL）
ALTER TABLE patrol_runs ADD COLUMN prompt_tokens INTEGER;
ALTER TABLE patrol_runs ADD COLUMN completion_tokens INTEGER;
ALTER TABLE tasks ADD COLUMN prompt_tokens INTEGER;
ALTER TABLE tasks ADD COLUMN completion_tokens INTEGER;
