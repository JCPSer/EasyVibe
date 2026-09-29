// 后端事件轻量总线：App 层的 WS 订阅在此转发，Canvas 消费。
// M2-5 引入正式状态管理后取代。
export type GrowthEventListener = (event: Record<string, unknown>) => void
export type SessionEventListener = (event: { repo: string; sessionId: string; status: string }) => void

const growthListeners = new Set<GrowthEventListener>()
const sessionListeners = new Set<SessionEventListener>()

export function emitGrowthEvent(event: Record<string, unknown>) {
  for (const l of growthListeners) l(event)
}

export function onGrowthEvent(listener: GrowthEventListener): () => void {
  growthListeners.add(listener)
  return () => {
    growthListeners.delete(listener)
  }
}

export function emitSessionEvent(event: { repo: string; sessionId: string; status: string }) {
  for (const l of sessionListeners) l(event)
}

export function onSessionEvent(listener: SessionEventListener): () => void {
  sessionListeners.add(listener)
  return () => {
    sessionListeners.delete(listener)
  }
}

// WS 断线通知（R2）：App 层 WS onclose 触发，Canvas 消费（退出生长模式）
let wsCloseListener: (() => void) | null = null
export function setWsCloseListener(fn: (() => void) | null) {
  wsCloseListener = fn
}
export function notifyWsClosed() {
  wsCloseListener?.()
}
