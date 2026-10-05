import { toast } from '@/runtime/toast'

// 会话排队入队 helper（运行会话气泡 + 单会话排队，2026-10-04）：
// 409（单会话纪律）的各入口统一走这里——POST /repos/{id}/session-queue 一次原子裁决：
//   - 有活动会话 → 排队成功（queued；同槽旧排队被替换时 outcome='replaced' 且带旧 label）
//   - 无活动会话 → 后端直接代为执行（started）
// 网络/非 2xx 错误在此统一 toast，调用点只需定制成功文案。
// 契约见 docs/requirements-session-bubble-queue.md §5。

export type SessionQueueKind = 'patrol' | 'reinduce' | 'submap'

/** GET /repos/{id}/session-queue 的快照（契约 §5）——SessionBubble 的数据源 */
export interface SessionQueueSnapshot {
  active: { sessionId: string; label: string; status: string; startedAt?: string | null } | null
  queued: { kind: SessionQueueKind; label: string; moduleId?: string; enqueuedAt?: string } | null
}

/** 已运行时长格式化（纯函数）：mm:ss；满 1 小时 h:mm:ss；负值钳到 0 */
export function formatElapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000))
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const sec = s % 60
  const mm = String(m).padStart(2, '0')
  const ss = String(sec).padStart(2, '0')
  return h > 0 ? `${h}:${mm}:${ss}` : `${m}:${ss}`
}

/** 活动会话图标类型推断：契约里 active 无 kind 字段（只有 queued 带），从 label 文本推导 */
export function kindFromLabel(label: string): SessionQueueKind {
  if (label.includes('巡检')) return 'patrol'
  if (label.includes('分析')) return 'submap'
  return 'reinduce'
}

/** 空态判定（纯函数）：无活动且无排队 → 不渲染 */
export function isEmptyState(s: SessionQueueSnapshot | null): boolean {
  return !s || (!s.active && !s.queued)
}

export type EnqueueOutcome = 'queued' | 'started' | 'replaced'

export interface EnqueueResult {
  outcome: EnqueueOutcome
  /** outcome='replaced' 时被替换掉的旧排队 label（S3） */
  replacedLabel: string | null
}

export async function enqueue(
  repo: string,
  kind: SessionQueueKind,
  moduleId?: string,
): Promise<EnqueueResult | null> {
  try {
    const r = await fetch(`/api/repos/${encodeURIComponent(repo)}/session-queue`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(moduleId ? { kind, moduleId } : { kind }),
    })
    if (r.status === 202) {
      const d = (await r.json().catch(() => null)) as {
        data?: { queued?: boolean; started?: boolean; replaced?: { label?: string } | null }
      } | null
      const data = d?.data
      if (data?.started) return { outcome: 'started', replacedLabel: null }
      // replaced 非空即视为替换（S3：响应带被替换项 label）
      if (data?.replaced) return { outcome: 'replaced', replacedLabel: data.replaced.label ?? null }
      return { outcome: 'queued', replacedLabel: null }
    }
    const body = (await r.json().catch(() => null)) as { error?: string } | null
    toast(`加入队列失败：${body?.error ?? `HTTP ${r.status}`}`, 'error')
    return null
  } catch {
    toast('加入队列失败（请确认后端在线后重试）。', 'error')
    return null
  }
}
