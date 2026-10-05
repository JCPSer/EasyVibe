// 终端环形缓冲（方案 v3 §4.2）：模块级缓存——组件卸载/切页不丢，切回重渲染。
//  keyed by sessionId（session.output 事件的归属键）；每会话封顶 200 行，保头丢尾。
//  WS 断线期间后端 broadcast 无回放是已知边界（方案 §6），由页面显示灰条明示。

const CAP = 200
const buffers = new Map<string, string[]>()

export function pushTerminalLine(sessionId: string, line: string) {
  const buf = buffers.get(sessionId)
  if (buf) {
    if (buf.length >= CAP) buf.shift()
    buf.push(line)
  } else {
    buffers.set(sessionId, [line])
  }
}

export function terminalLines(sessionId: string): string[] {
  return buffers.get(sessionId) ?? []
}

/** 终态后保留最近一会话供回看；任务重跑（新 sessionId）时旧缓冲自然孤儿化，由容量兜底 */
export function clearTerminal(sessionId: string) {
  buffers.delete(sessionId)
}
