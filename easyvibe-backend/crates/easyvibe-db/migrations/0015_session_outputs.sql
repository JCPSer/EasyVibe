-- 会话输出行落盘（M2，2026-10-05）：运行页历史回放 + 断线补拉的数据层。
-- 每会话 seq 单调（看门任务内 AtomicU64），(session_id, seq) 主键——补拉按 lastSeq 幂等去重。
-- 容量策略：每会话保留最近 50k 行（终态 finalize 时修剪）；行率不高（直播已截断 200 字符），
-- 50k 行 ≈ 10MB/会话上限，可接受。
CREATE TABLE IF NOT EXISTS session_outputs (
  session_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  ts TEXT NOT NULL,
  stream TEXT NOT NULL DEFAULT 'stdout',
  line TEXT NOT NULL,
  PRIMARY KEY (session_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_session_outputs_fetch ON session_outputs(session_id, seq);
