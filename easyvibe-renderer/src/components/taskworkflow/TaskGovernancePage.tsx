import { useCallback, useEffect, useMemo, useState } from 'react'
import { Copy, Loader2 } from 'lucide-react'
import { onPatrolFinished, onTaskEvent } from '@/runtime/growthBus'
import { absTime, toMs } from '@/shared/logic/diffStat'
import { stageOf, gateLabel } from '@/components/taskworkflow/taskStage'
import { TaskAdminButtons } from '@/components/taskworkflow/TaskAdminButtons'
import type { TaskDraft } from '@/shared/logic/taskContext'

// 治理视图（v4 P2）：历史检索 + 失败治理 + 返工链可见。
// 看板管"现在"，治理管"全部"。返工链：origin_task_id 血缘沿链缩进渲染。
// 点行 → 流水线视图只读回看；失败/驳回行内 [复制重提]。

interface GovTask {
  id: string
  title: string
  description?: string
  status: string
  gate: string | null
  trust: string
  modules?: string[]
  createdAt: string
  updatedAt: string
  error?: string | null
  originTaskId?: string | null
  successorTaskId?: string | null
  result?: { contractViolations?: string[] } | null
}

type StatusFilter = 'all' | 'active' | 'awaiting' | 'done' | 'failed'
type ChainFilter = 'all' | 'rework' | 'root'
type TimeFilter = 'all' | '7d' | '30d'

const STATUS_META: Record<string, { label: string; cls: string }> = {
  pending: { label: '排队中', cls: 'bg-slate-100 dark:bg-slate-800 text-slate-500 dark:text-slate-400' },
  running: { label: '运行中', cls: 'bg-amber-50 dark:bg-amber-950/40 text-amber-600' },
  awaiting_approval: { label: '等待审批', cls: 'bg-blue-50 dark:bg-blue-950/40 text-blue-600' },
  done: { label: '已完成', cls: 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-600' },
  failed: { label: '失败', cls: 'bg-red-50 dark:bg-red-950/40 text-red-500' },
  interrupted: { label: '已中断', cls: 'bg-slate-100 dark:bg-slate-800 text-slate-500 dark:text-slate-400' },
  rejected: { label: '已驳回', cls: 'bg-red-50 dark:bg-red-950/40 text-red-500' },
}

function MiniDots({ status, gate }: { status: string; gate: string | null | undefined }) {
  const s = stageOf(status, gate)
  return (
    <span className="inline-flex items-center gap-[3px]" title="五阶段进度">
      {[0, 1, 2, 3, 4].map((i) => {
        let cls = 'bg-slate-100 dark:bg-slate-800'
        if (s === 'done') cls = 'bg-emerald-500'
        else if (s !== 'error') {
          if (i < s) cls = 'bg-emerald-500'
          else if (i === s) cls = 'bg-blue-600'
        }
        return <span key={i} className={`h-1.5 w-1.5 rounded-full ${cls}`} />
      })}
    </span>
  )
}

export function TaskGovernancePage({
  backendRepo,
  onOpenTask,
  onCreateTask,
}: {
  backendRepo: string | null
  onOpenTask: (taskId: string) => void
  onCreateTask: (d: TaskDraft) => void
}) {
  const [tasks, setTasks] = useState<GovTask[] | null>(null)
  const [statusF, setStatusF] = useState<StatusFilter>('all')
  const [chainF, setChainF] = useState<ChainFilter>('all')
  const [timeF, setTimeF] = useState<TimeFilter>('all')

  const load = useCallback(() => {
    if (!backendRepo) return
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/tasks`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: GovTask[] } | null) => setTasks(d?.data ?? []))
      .catch(() => {})
  }, [backendRepo])

  useEffect(() => {
    load()
  }, [load])
  useEffect(() => onTaskEvent(load), [load])
  useEffect(() => onPatrolFinished((evt) => { if (evt.repo === backendRepo) load() }), [backendRepo, load])

  // 血缘索引：返工次数（该任务被重提了几回）+ 父子关系
  const chain = useMemo(() => {
    const childrenOf = new Map<string, GovTask[]>()
    const reworkCount = new Map<string, number>()
    for (const t of tasks ?? []) {
      if (t.originTaskId) {
        const list = childrenOf.get(t.originTaskId) ?? []
        list.push(t)
        childrenOf.set(t.originTaskId, list)
        // 沿链累计：每个返工任务给整条祖先链 +1
        let cur: string | null = t.originTaskId
        const seen = new Set<string>()
        while (cur && !seen.has(cur)) {
          seen.add(cur)
          reworkCount.set(cur, (reworkCount.get(cur) ?? 0) + 1)
          cur = tasks?.find((x) => x.id === cur)?.originTaskId ?? null
        }
      }
    }
    return { childrenOf, reworkCount }
  }, [tasks])

  const rows = useMemo(() => {
    let list = tasks ?? []
    // 时间窗
    if (timeF !== 'all') {
      const now = Date.now()
      const span = timeF === '7d' ? 7 : 30
      const cutoff = now - span * 24 * 3600 * 1000
      list = list.filter((t) => (toMs(t.createdAt) ?? 0) >= cutoff)
    }
    // 状态
    if (statusF === 'active') list = list.filter((t) => t.status === 'running' || t.status === 'pending')
    else if (statusF === 'awaiting') list = list.filter((t) => t.status === 'awaiting_approval')
    else if (statusF === 'done') list = list.filter((t) => t.status === 'done')
    else if (statusF === 'failed') list = list.filter((t) => ['failed', 'rejected', 'interrupted'].includes(t.status))
    // 返工链归属
    if (chainF === 'rework') list = list.filter((t) => t.originTaskId || (chain.reworkCount.get(t.id) ?? 0) > 0)
    else if (chainF === 'root') list = list.filter((t) => !t.originTaskId)
    // 根任务在前、按更新时间倒序；返工母本带缩进行
    const sorted = [...list].sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
    return sorted
  }, [tasks, statusF, chainF, timeF, chain.reworkCount])

  const duration = (t: GovTask) => {
    const a = toMs(t.createdAt)
    const b = toMs(t.updatedAt)
    if (!a || !b) return '—'
    const min = Math.floor(Math.max(0, b - a) / 60000)
    if (min < 1) return '刚刚'
    if (min < 60) return `${min} 分`
    return `${Math.floor(min / 60)} 时 ${min % 60} 分`
  }

  if (!backendRepo) {
    return <div className="flex h-full items-center justify-center text-[12px] text-slate-400 dark:text-slate-500">先在左侧选择一个项目。</div>
  }

  const renderRow = (t: GovTask, depth: number, child = false) => {
    const meta = STATUS_META[t.status] ?? STATUS_META.pending
    const reworks = chain.reworkCount.get(t.id) ?? 0
    const violations = t.result?.contractViolations?.length ?? 0
    const failed = ['failed', 'rejected', 'interrupted'].includes(t.status)
    return (
      <div key={`${depth}-${t.id}`}>
        <div
          onClick={() => onOpenTask(t.id)}
          className={`flex cursor-pointer items-center gap-3 border-b border-slate-50 px-4 py-2.5 text-[11px] transition-colors hover:bg-slate-50 dark:hover:bg-slate-800/70 ${
            child ? 'bg-slate-50/50 dark:bg-slate-900/50 dark:bg-slate-900/50' : ''
          }`}
          style={{ paddingLeft: child ? undefined : undefined }}
        >
          {child && <span className="w-5 shrink-0 text-slate-300 dark:text-slate-600">└</span>}
          <span className="min-w-0 flex-1">
            <span className={`block truncate text-[12px] ${child ? 'font-normal text-slate-500 dark:text-slate-400' : 'font-semibold text-slate-700 dark:text-slate-200'}`}>
              {t.title}
              {violations > 0 && (
                <span className="tnum ml-1.5 rounded-full bg-red-50 dark:bg-red-950/40 px-1.5 py-px text-[9px] font-bold text-red-500">越界 {violations}</span>
              )}
            </span>
            {failed && t.error && <span className="mt-0.5 block truncate text-[10px] text-red-400">{t.error}</span>}
          </span>
          <span className="hidden shrink-0 sm:block"><MiniDots status={t.status} gate={t.gate} /></span>
          <span className="tnum w-14 shrink-0 text-slate-400 dark:text-slate-500">{duration(t)}</span>
          <span className="tnum hidden w-32 shrink-0 text-slate-400 dark:text-slate-500 md:block">{absTime(t.createdAt)}</span>
          {reworks > 0 && (
            <span className="tnum w-14 shrink-0 rounded-full bg-amber-50 dark:bg-amber-950/40 px-1.5 py-px text-center text-[9px] font-bold text-amber-600" title="被返工重提的次数">
              重提 ×{reworks}
            </span>
          )}
          {failed && (
            <button
              onClick={(e) => {
                e.stopPropagation()
                onCreateTask({
                  title: `${t.title}（重提）`,
                  description: t.description ?? t.title,
                  modules: t.modules ?? [],
                  acceptance: '',
                  source: 'manual',
                  context: { origin_task_id: t.id },
                })
              }}
              className="shrink-0 rounded-md border border-red-200 dark:border-red-900/60 px-2 py-0.5 text-[10px] font-bold text-red-500 hover:bg-red-50 dark:hover:bg-red-950/40"
            >
              <Copy size={9} className="mr-0.5 inline" /> 复制重提
            </button>
          )}
          <span className={`shrink-0 rounded-full px-2 py-px text-[10px] font-bold ${meta.cls}`}>
            {gateLabel(t.status, t.gate) ?? meta.label}
          </span>
          {/* 管理三操作（重审 P0）：治理页是"管全部"的地方——终止/重试/删除必须在场 */}
          <span className="shrink-0"><TaskAdminButtons repo={backendRepo} taskId={t.id} status={t.status} onDone={load} /></span>
        </div>
        {/* 返工子任务沿链缩进 */}
        {(chain.childrenOf.get(t.id) ?? [])
          .filter((c) => rows.some((r) => r.id === c.id))
          .map((c) => renderRow(c, depth + 1, true))}
      </div>
    )
  }

  const FILTER_CLS = 'rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1 text-[11px] font-semibold text-slate-500 dark:text-slate-400 hover:border-blue-300 hover:text-blue-600'
  const FILTER_ON = 'rounded-lg border border-blue-600 bg-blue-600 px-2.5 py-1 text-[11px] font-bold text-white'

  return (
    <div className="flex h-full flex-col p-4">
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <span className="text-[12px] font-bold text-slate-600 dark:text-slate-300">治理</span>
        <span className="text-[10px] text-slate-400 dark:text-slate-500">全部任务的历史、失败与返工链</span>
        <span className="ml-2 flex gap-1.5">
          {(
            [
              ['all', '全部状态'],
              ['active', '进行中'],
              ['awaiting', '待审批'],
              ['done', '已完成'],
              ['failed', '失败·驳回'],
            ] as [StatusFilter, string][]
          ).map(([k, label]) => (
            <button key={k} onClick={() => setStatusF(k)} className={statusF === k ? FILTER_ON : FILTER_CLS}>{label}</button>
          ))}
        </span>
        <span className="flex gap-1.5">
          {(
            [
              ['all', '全部时间'],
              ['7d', '近 7 天'],
              ['30d', '近 30 天'],
            ] as [TimeFilter, string][]
          ).map(([k, label]) => (
            <button key={k} onClick={() => setTimeF(k)} className={timeF === k ? FILTER_ON : FILTER_CLS}>{label}</button>
          ))}
        </span>
        <span className="flex gap-1.5">
          {(
            [
              ['all', '返工链：全部'],
              ['root', '仅源头'],
              ['rework', '仅返工相关'],
            ] as [ChainFilter, string][]
          ).map(([k, label]) => (
            <button key={k} onClick={() => setChainF(k)} className={chainF === k ? FILTER_ON : FILTER_CLS}>{label}</button>
          ))}
        </span>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto rounded-xl border border-slate-100 dark:border-slate-800 bg-white dark:bg-slate-900">
        <div className="flex items-center gap-3 border-b border-slate-100 dark:border-slate-800 bg-slate-50/60 dark:bg-slate-900/60 px-4 py-2 text-[9px] font-bold uppercase tracking-wider text-slate-400 dark:text-slate-500">
          <span className="min-w-0 flex-1">任务</span>
          <span className="hidden w-16 shrink-0 sm:block">阶段</span>
          <span className="w-14 shrink-0">耗时</span>
          <span className="hidden w-32 shrink-0 md:block">创建于</span>
          <span className="w-14 shrink-0">血缘</span>
          <span className="w-20 shrink-0 text-right">状态</span>
        </div>
        {tasks === null && (
          <p className="flex items-center justify-center gap-2 py-14 text-[12px] text-slate-400 dark:text-slate-500">
            <Loader2 size={13} className="animate-spin" /> 加载任务…
          </p>
        )}
        {tasks !== null && rows.length === 0 && (
          <p className="py-14 text-center text-[12px] text-slate-300 dark:text-slate-600">这个筛选条件下没有任务</p>
        )}
        {/* 只渲染根任务（无 origin）；子任务由父行内联渲染，避免重复 */}
        {rows.filter((t) => !t.originTaskId || !rows.some((r) => r.id === t.originTaskId)).map((t) => renderRow(t, 0))}
      </div>
      <p className="mt-1.5 text-[10px] text-slate-300 dark:text-slate-600">点任意行 → 流水线视图回看该任务全程</p>
    </div>
  )
}
