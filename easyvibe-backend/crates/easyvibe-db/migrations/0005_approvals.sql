-- M3-4 审批流：三道关（计划→Diff→审查报告）全程留痕；F5 两档（manual 拦截 / auto 直通记 skipped）
CREATE TABLE IF NOT EXISTS approvals (
  id TEXT PRIMARY KEY,
  task_id TEXT NOT NULL REFERENCES tasks(id),
  gate TEXT NOT NULL,               -- plan / diff / report
  decision TEXT NOT NULL,           -- approved / rejected / skipped（auto 直通留痕）
  note TEXT,
  decided_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_approvals_task ON approvals(task_id);
ALTER TABLE tasks ADD COLUMN gate TEXT;  -- 当前关卡（plan/diff/report/done）
