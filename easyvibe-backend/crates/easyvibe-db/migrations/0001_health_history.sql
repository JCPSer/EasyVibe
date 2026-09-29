-- 域 2（IDE 自有状态）：健康度历史——巡检的持久化记录
CREATE TABLE IF NOT EXISTS patrol_runs (
  id TEXT PRIMARY KEY,
  repo TEXT NOT NULL,
  started_at TEXT NOT NULL,
  finished_at TEXT,
  status TEXT NOT NULL,               -- running / succeeded / failed
  model TEXT,
  arch_score INTEGER,
  error TEXT
);

CREATE TABLE IF NOT EXISTS module_health_history (
  run_id TEXT NOT NULL REFERENCES patrol_runs(id),
  module_id TEXT NOT NULL,
  name TEXT,
  score INTEGER NOT NULL,
  coupling TEXT,
  complexity TEXT,
  churn TEXT,
  decay_flags TEXT NOT NULL DEFAULT '[]',   -- JSON array
  review_note TEXT,
  concerns TEXT NOT NULL DEFAULT '[]',      -- JSON array
  PRIMARY KEY (run_id, module_id)
);

CREATE INDEX IF NOT EXISTS idx_patrol_runs_repo ON patrol_runs(repo, started_at);
CREATE INDEX IF NOT EXISTS idx_mhh_module ON module_health_history(module_id);
