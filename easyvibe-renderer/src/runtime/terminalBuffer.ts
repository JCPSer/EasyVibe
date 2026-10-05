// 终端环形缓冲（方案 v3 §4.2）：模块级缓存——组件卸载/切页不丢，切回重渲染。
//  keyed by sessionId（session.output 事件的归属键）；每会话封顶 200 行，保头丢尾。
//  行携带 seq/stream：WS 推送与 HTTP 补拉（TerminalStage 断线兜底）可交错幂等去重。
//  WS 断线期间后端 broadcast 无回放是已知边界（方案 §6），由页面显示灰条明示。

export type TerminalLine = { seq: number; stream: string; line: string }

const CAP = 200
const buffers = new Map<string, TerminalLine[]>()
const seen = new Map<string, Set<number>>()

/** 追加行（seq 幂等去重——WS 与 HTTP 补拉交错时防重；每会话封顶 CAP 行） */
export function pushTerminalLine(sessionId: string, seq: number, stream: string, line: string) {
  let dup = seen.get(sessionId)
  if (!dup) {
    dup = new Set()
    seen.set(sessionId, dup)
  }
  if (dup.has(seq)) return
  dup.add(seq)
  const buf = buffers.get(sessionId)
  if (buf) {
    if (buf.length >= CAP) {
      // 与缓冲同步驱逐：去重集不无限增长
      const evicted = buf.splice(0, buf.length - CAP + 1)
      for (const l of evicted) dup.delete(l.seq)
    }
    buf.push({ seq, stream, line })
  } else {
    buffers.set(sessionId, [{ seq, stream, line }])
  }
}

export function terminalLines(sessionId: string): TerminalLine[] {
  return buffers.get(sessionId) ?? []
}

/** 已收到的最大 seq（HTTP 增量补拉的 afterSeq 锚点；无行时返回 -1 从头拉） */
export function terminalLastSeq(sessionId: string): number {
  const buf = buffers.get(sessionId)
  return buf && buf.length > 0 ? buf[buf.length - 1].seq : -1
}

/** 终态后保留最近一会话供回看；任务重跑（新 sessionId）时旧缓冲自然孤儿化，由容量兜底 */
export function clearTerminal(sessionId: string) {
  buffers.delete(sessionId)
  seen.delete(sessionId)
}
