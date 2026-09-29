// 生长事件轻量总线：App 层的 WS 订阅在此转发，Canvas 的生长状态机订阅消费。
// M2-3 引入 session 状态后由正式状态管理取代。
export type GrowthEventListener = (event: Record<string, unknown>) => void

const listeners = new Set<GrowthEventListener>()

export function emitGrowthEvent(event: Record<string, unknown>) {
  for (const l of listeners) l(event)
}

export function onGrowthEvent(listener: GrowthEventListener): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}
