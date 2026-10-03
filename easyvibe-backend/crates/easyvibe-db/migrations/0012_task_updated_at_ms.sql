-- 2026-10-03 实弹 bug 修复：tasks.updated_at 单位混用——
-- create_task/try_advance_gate 写 epoch 毫秒，update_status/interrupt_running/reset_for_retry
-- 写 epoch 秒。dev-docs 时间窗把 updated_at 当毫秒解析，右端被压到 1970+20 天内，
-- 产物文档全部漏检（评审卡"未找到本阶段产物文档"实报）。
-- 修复：把 10 位的秒值统一升级为 13 位毫秒（幂等：只动长度 ≤10 的行）。
UPDATE tasks SET updated_at = CAST(updated_at AS INTEGER) * 1000 WHERE length(updated_at) <= 10;
