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

// WS 连接状态（2026-10-05 实弹：连接断开时全页面无感知，任务终端/生长直播静默死亡——
// 状态可视化 + 消费端 HTTP 兜底两条防线；TopBar 显示"实时连接重连中"弱提示）
let wsConnected = false
const connListeners = new Set<(connected: boolean) => void>()
export function setWsConnected(c: boolean) {
  if (wsConnected === c) return
  wsConnected = c
  for (const l of connListeners) l(c)
}
export function isWsConnected(): boolean {
  return wsConnected
}
export function onWsConnection(l: (connected: boolean) => void): () => void {
  connListeners.add(l)
  return () => {
    connListeners.delete(l)
  }
}

export type TaskEventListener = (event: { repo: string; taskId: string; status: string; gate?: string }) => void
const taskListeners = new Set<TaskEventListener>()
export function emitTaskEvent(e: { repo: string; taskId: string; status: string; gate?: string }) {
  for (const l of taskListeners) l(e)
}
export function onTaskEvent(l: TaskEventListener): () => void {
  taskListeners.add(l)
  return () => {
    taskListeners.delete(l)
  }
}

// S2：地图保鲜事件（freshness.changed）
export type FreshnessEventListener = (event: {
  repo: string
  status: string
  latestCommitAt?: number | null
  commitsSinceMap?: number | null
}) => void
const freshnessListeners = new Set<FreshnessEventListener>()
export function emitFreshnessEvent(e: { repo: string; status: string; latestCommitAt?: number | null; commitsSinceMap?: number | null }) {
  for (const l of freshnessListeners) l(e)
}
export function onFreshnessEvent(l: FreshnessEventListener): () => void {
  freshnessListeners.add(l)
  return () => {
    freshnessListeners.delete(l)
  }
}

// 改进#2：agent 过程直播（session.output——子图分析/任务执行的 stdout 行）
// M2 扩展：seq = 每会话单调行号（断线补拉锚点）；stream = stdout/stderr
export type SessionOutputListener = (event: { sessionId: string; seq: number; stream: string; line: string }) => void
const outputListeners = new Set<SessionOutputListener>()
export function emitSessionOutput(e: { sessionId: string; seq: number; stream: string; line: string }) {
  for (const l of outputListeners) l(e)
}
export function onSessionOutput(l: SessionOutputListener): () => void {
  outputListeners.add(l)
  return () => {
    outputListeners.delete(l)
  }
}

// 运行会话气泡 + 单会话排队（2026-10-04）：会话队列变更（入队/替换/取消/排空/启动失败）
// WS queue.changed → App 层转发，SessionBubble 消费（重拉 queue 快照）
export type QueueChangedEvent = {
  repo: string
  type: 'enqueued' | 'replaced' | 'cancelled' | 'drained' | 'failed'
  job?: { kind: string; label: string; moduleId?: string }
  started?: boolean
  error?: string
}
export type QueueChangedListener = (event: QueueChangedEvent) => void
const queueListeners = new Set<QueueChangedListener>()
export function emitQueueChanged(e: QueueChangedEvent) {
  for (const l of queueListeners) l(e)
}
export function onQueueChanged(l: QueueChangedListener): () => void {
  queueListeners.add(l)
  return () => {
    queueListeners.delete(l)
  }
}

// R3 C1：巡检终态（patrol.finished）——解除"巡检中"、驱动健康看板刷新
export type PatrolFinishedListener = (event: { repo: string; runId: string; status: string }) => void
const patrolListeners = new Set<PatrolFinishedListener>()
export function emitPatrolFinished(e: { repo: string; runId: string; status: string }) {
  for (const l of patrolListeners) l(e)
}
export function onPatrolFinished(l: PatrolFinishedListener): () => void {
  patrolListeners.add(l)
  return () => {
    patrolListeners.delete(l)
  }
}
