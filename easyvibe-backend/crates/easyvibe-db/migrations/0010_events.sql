-- R3 D1：使用证据埋点（战略报告 P0-2）——一切门控（L3 三道门、Harness 验证指标、
-- 复检点消费率）的秤。纯事件行 + 计数聚合，SQLite 足以支撑百万级。
-- 事件名 dot.case，payload 为 JSON 计数维度。
CREATE TABLE IF NOT EXISTS events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo TEXT NOT NULL,
  name TEXT NOT NULL,
  payload TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_repo_name ON events(repo, name, created_at);
