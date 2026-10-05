-- 2026-10-05 问题项新旧对照：patrol_runs 挂 concerns_diff（JSON，仅 succeeded 写入）
ALTER TABLE patrol_runs ADD COLUMN concerns_diff TEXT NULL;
