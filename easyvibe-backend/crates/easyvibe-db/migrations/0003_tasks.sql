-- 工作区：任务（M3-2 指哪打哪：地图/问题清单入口，上下文预填；执行引擎 M3-3 接入）
CREATE TABLE IF NOT EXISTS tasks (
  id TEXT PRIMARY KEY,
  repo TEXT NOT NULL,
  title TEXT NOT NULL,
  description TEXT NOT NULL,
  modules TEXT NOT NULL DEFAULT '[]',     -- JSON: 影响模块 id
  acceptance TEXT NOT NULL DEFAULT '',
  source TEXT NOT NULL DEFAULT 'manual',  -- module / concern / layer / manual
  context TEXT NOT NULL DEFAULT '{}',     -- JSON: 组织好的注入上下文（模块职责/边界/concerns/违规边）
  status TEXT NOT NULL DEFAULT 'pending', -- pending / running / succeeded / failed
  trust TEXT NOT NULL DEFAULT 'manual',   -- manual / auto（F5 两档）
  error TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tasks_repo ON tasks(repo, created_at);
