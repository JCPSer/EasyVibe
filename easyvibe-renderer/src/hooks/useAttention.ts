import { useEffect, useState } from 'react'
import { onTaskEvent } from '@/runtime/growthBus'
import { t } from '@/runtime/i18n'
import { listTasks } from '@/api/task'

export type Attention = { count: number; sample: string } | null

/** 关卡话术映射（审批门人话标签；枚举值为后端契约，不随语言变） */
const GATE_WORDING: Record<string, string> = {
  plan: 'task.gate.plan',
  analysis: 'task.gate.analysis',
  solution: 'task.gate.solution',
  diff: 'task.gate.diff',
  report: 'task.gate.report',
}

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
    const load = () =>
      listTasks(backendRepo)
        .then((r) => (r.ok ? r.json() : null))
        .then((d: { data?: { status: string; title: string; gate?: string | null }[] } | null) => {
          const ts = d?.data ?? []
          setPendingApprovals(ts.filter((t) => t.status === 'awaiting_approval').length)
          setRunningCount(ts.filter((t) => t.status === 'running').length)
          const waiting = ts.filter((t) => t.status === 'awaiting_approval')
          setAttention(
            waiting.length > 0
              ? {
                  count: waiting.length,
                  sample: t('task.attentionSample', {
                    title: waiting[0].title,
                    gate: t(GATE_WORDING[waiting[0].gate ?? ''] ?? 'chat.gate.fallback'),
                  }),
                }
              : null,
          )
        })
        .catch(() => {})
    load()
    return onTaskEvent(load)
  }, [backendRepo])
  return { pendingApprovals, runningCount, attention }
}
