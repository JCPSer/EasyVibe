-- agent 会话表（M1/U1，2026-10-05）：
-- 运行页历史回放与用量页统计的地基。会话（除巡检 run 外）此前无持久化——
-- 终态即删元数据，后端重启后「刚才那次归纳怎么样了」无从查起。
-- 设计：docs/agent-observability-research.md §2 B1 + docs/llm-usage-page-design-v1.md §2。
-- 建模预留多 agent：parent_session_id（任务→初审/审查子 agent 归属）一次到位。
-- usage 列全部可空、不设 DEFAULT——NULL=未回报（被 kill/超时/非 claude CLI 的诚实降级），
-- 区别于 0（0006 注释先例：会话型 CLI agent 无法回报则留 NULL）。
CREATE TABLE IF NOT EXISTS agent_sessions (
  id TEXT PRIMARY KEY,
  repo TEXT NOT NULL,
  kind TEXT NOT NULL DEFAULT 'unknown',
  label TEXT,
  cli TEXT,
  model TEXT,
  parent_session_id TEXT,
  task_id TEXT,
  started_at TEXT NOT NULL,
  terminal_at TEXT,
  status TEXT NOT NULL DEFAULT 'running',
  exit_code INTEGER,
  input_tokens INTEGER,
  output_tokens INTEGER,
  cache_read_tokens INTEGER,
  cache_write_tokens INTEGER,
  cost_usd REAL,
  duration_ms INTEGER,
  turns INTEGER,
  usage_source TEXT
);
-- 时间范围查询（用量页今天/7天/30天/全部）与类型分布聚合的索引
CREATE INDEX IF NOT EXISTS idx_agent_sessions_repo_started ON agent_sessions(repo, started_at);
CREATE INDEX IF NOT EXISTS idx_agent_sessions_kind_started ON agent_sessions(kind, started_at);
