import { useEffect, useState } from 'react'
import { onTaskEvent } from '@/runtime/growthBus'

export type Attention = { count: number; sample: string } | null

/**
 * 应用壳徽标/注意力条数据（待审批 + 执行中 + 第一条待审批话术）。
 * 初始拉取 + WS 任务事件驱动。从 App 抽出，行为与依赖数组不变。
 */
export function useAttention(backendRepo: string | null) {
  const [pendingApprovals, setPendingApprovals] = useState(0)
  const [runningCount, setRunningCount] = useState(0)
  const [attention, setAttention] = useState<Attention>(null)
  useEffect(() => {
    if (!backendRepo) {
      setPendingApprovals(0)
      setRunningCount(0)
      setAttention(null)
      return
    }
    const GL: Record<string, string> = { plan: '任务书审批', analysis: '需求矩阵评审', solution: '方案评审', diff: 'Diff 审批', report: '审查报告' }
    const load = () =>
      fetch(`/api/repos/${encodeURIComponent(backendRepo)}/tasks`)
        .then((r) => (r.ok ? r.json() : null))
        .then((d: { data?: { status: string; title: string; gate?: string | null }[] } | null) => {
          const ts = d?.data ?? []
          setPendingApprovals(ts.filter((t) => t.status === 'awaiting_approval').length)
          setRunningCount(ts.filter((t) => t.status === 'running').length)
          const waiting = ts.filter((t) => t.status === 'awaiting_approval')
          setAttention(
            waiting.length > 0
              ? { count: waiting.length, sample: `「${waiting[0].title}」停在${GL[waiting[0].gate ?? ''] ?? '审批'}` }
              : null,
          )
        })
        .catch(() => {})
    load()
    return onTaskEvent(load)
  }, [backendRepo])
  return { pendingApprovals, runningCount, attention }
}
