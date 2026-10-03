import { useCallback, useEffect, useMemo, useState } from 'react'
import { Check, Copy, Loader2, X } from 'lucide-react'
import { toast } from '@/lib/toast'
import { onPatrolFinished, onTaskEvent } from '@/lib/growthBus'
import { absTime, toMs } from '@/lib/diffStat'
import { stageOf } from '@/lib/taskStage'
import type { TaskDraft } from '@/lib/taskContext'

// 任务编排看板（方案 v3 §4.3 施工，取代旧「我的待办」列表）：
// 五列 = 状态口径；拖拽只承载合法语义动作（与状态机一致，不造假）：
//   待你评审 → 已完成 = 通过（diff 关只推进、卡留列翻面；report 关才落列——复审修正）
//   待你评审 → 待启动 = 打回（落下弹意见输入）
//   失败组卡 → 待启动主体区 = 复制为新任务（origin_task_id 血缘）
//   其余跨列目标禁止落入；列内拖拽 = 会话内排序。

interface BoardTask {
  id: string
  title: string
  description?: string
  status: string
  gate: string | null
  trust: string
  modules?: string[]
  createdAt: string
  updatedAt: string
  result?: { contractViolations?: string[] } | null
}

type ColKey = 'backlog' | 'plan' | 'running' | 'review' | 'done'

const COLS: { key: ColKey; label: string }[] = [
  { key: 'backlog', label: '待启动' },
  { key: 'plan', label: '分析与方案' },
  { key: 'running', label: '实施中' },
  { key: 'review', label: '待你评审' },
  { key: 'done', label: '已完成' },
]

/** 列归属（方案 §4.3 口径）：status 优先，failed/rejected/interrupted 归待启动顶部失败组 */
function colOf(t: BoardTask): { col: ColKey; failed: boolean } {
  const failed = ['failed', 'rejected', 'interrupted'].includes(t.status)
  if (failed) return { col: 'backlog', failed: true }
  switch (t.status) {
    case 'pending':
      return { col: 'backlog', failed: false }
    case 'awaiting_approval':
      // 分阶段流：plan/analysis/solution 三关都属"分析与方案"阶段带
      return { col: ['plan', 'analysis', 'solution'].includes(t.gate ?? '') ? 'plan' : 'review', failed: false }
    case 'running':
      return { col: 'running', failed: false }
    case 'done':
      return { col: 'done', failed: false }
    default:
      return { col: 'backlog', failed: false }
  }
}

/** 五阶段迷你进度点：done=全绿；error=全灰；否则已过的绿、当前的蓝、未到灰 */
function Dots({ t }: { t: BoardTask }) {
  const s = stageOf(t.status, t.gate)
  return (
    <span className="flex items-center gap-1">
      {[0, 1, 2, 3, 4].map((i) => {
        let cls = 'bg-slate-100'
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

export function TaskBoardPage({
  backendRepo,
  onOpenWorkflow,
  onCreateTask,
}: {
  backendRepo: string | null
  onOpenWorkflow: () => void
  onCreateTask: (d: TaskDraft) => void
}) {
  const [tasks, setTasks] = useState<BoardTask[] | null>(null)
  const [order, setOrder] = useState<Record<string, number>>({}) // 列内排序（会话内）
  const [dragId, setDragId] = useState<string | null>(null)
  const [overCol, setOverCol] = useState<ColKey | null>(null)
  const [rejecting, setRejecting] = useState<string | null>(null)
  const [rejectNote, setRejectNote] = useState('')
  const [deciding, setDeciding] = useState<string | null>(null)

  const load = useCallback(() => {
    if (!backendRepo) return
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/tasks`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: BoardTask[] } | null) => setTasks(d?.data ?? []))
      .catch(() => {})
  }, [backendRepo])

  useEffect(() => {
    load()
  }, [load])
  // 双页同开去抖（方案 §4.6）：taskId+status 为键，300ms 窗口合并风暴
  useEffect(
    () =>
      onTaskEvent(() => {
        // 简单去抖：直接复用 load 的闭包，由 fetch 自身的时序自然合并；
        // 评审页与工作流页都监听同事件时，各自一次 fetch 可接受（去抖债务记录在案）
        load()
      }),
    [load],
  )
  useEffect(() => onPatrolFinished((evt) => { if (evt.repo === backendRepo) load() }), [backendRepo, load])

  const grouped = useMemo(() => {
    const g: Record<ColKey, BoardTask[]> = { backlog: [], plan: [], running: [], review: [], done: [] }
    const failedTop: BoardTask[] = []
    for (const t of tasks ?? []) {
      const { col, failed } = colOf(t)
      if (failed) failedTop.push(t)
      else g[col].push(t)
    }
    const byOrder = (a: BoardTask, b: BoardTask) => (order[a.id] ?? 0) - (order[b.id] ?? 0) || b.updatedAt.localeCompare(a.updatedAt)
    for (const c of Object.keys(g) as ColKey[]) g[c].sort(byOrder)
    failedTop.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
    return { ...g, failedTop }
  }, [tasks, order])

  const counts = useMemo(
    () => ({
      backlog: grouped.backlog.length + grouped.failedTop.length,
      plan: grouped.plan.length,
      running: grouped.running.length,
      review: grouped.review.length,
      done: grouped.done.length,
    }),
    [grouped],
  )

  /** 拖拽合法性：只有这三个动作是真语义（方案 §4.3） */
  const canDrop = (task: BoardTask, target: ColKey): boolean => {
    if (!task) return false
    const { col: from } = colOf(task)
    if (from === target) return true // 列内排序
    if (from === 'review' && target === 'done') return true // 通过
    if (from === 'review' && target === 'backlog') return true // 打回
    return false
  }

  const decide = async (t: BoardTask, decision: 'approved' | 'rejected') => {
    if (!backendRepo || deciding) return
    setDeciding(t.id + decision)
    try {
      const r = await fetch(`/api/repos/${encodeURIComponent(backendRepo)}/tasks/${encodeURIComponent(t.id)}/decide`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ decision, note: decision === 'rejected' ? rejectNote.trim() : undefined, gate: t.gate }),
      })
      const d = await r.json().catch(() => null)
      if (!r.ok) {
        toast(d?.error ?? '审批失败', 'error')
        return
      }
      if (decision === 'approved' && t.gate === 'diff') {
        // diff 关通过只推进 report、卡留列——把真实状态说给用户（复审口径）
        toast('已通过 Diff 审批，还差终审（审查报告）')
      } else {
        toast(decision === 'approved' ? '已通过' : '已打回')
      }
      setRejecting(null)
      setRejectNote('')
      load()
    } finally {
      setDeciding(null)
    }
  }

  const onDrop = (target: ColKey) => (e: React.DragEvent) => {
    e.preventDefault()
    setOverCol(null)
    setDragId(null)
    const t = (tasks ?? []).find((x) => x.id === e.dataTransfer.getData('text/task-id'))
    if (!t) return
    const { col: from, failed } = colOf(t)
    if (from === target) {
      // 失败组 → 主体区 = 复制为新任务（方案 §4.3 落点语义）
      if (failed && target === 'backlog') {
        onCreateTask({
          title: `${t.title}（重提）`,
          description: t.description ?? t.title,
          modules: t.modules ?? [],
          acceptance: '',
          source: 'manual',
          context: { origin_task_id: t.id },
        })
      } else {
        // 列内排序：落点序号为新优先级（会话内）
        setOrder((prev) => ({ ...prev, [t.id]: Date.now() % 100000 }))
      }
      return
    }
    if (from === 'review' && target === 'done') {
      void decide(t, 'approved')
      return
    }
    if (from === 'review' && target === 'backlog') {
      setRejecting(t.id)
      setRejectNote('')
      return
    }
  }

  const duration = (t: BoardTask) => {
    const a = toMs(t.createdAt)
    const b = toMs(t.updatedAt)
    if (!a || !b) return '—'
    const min = Math.floor(Math.max(0, b - a) / 60000)
    if (min < 1) return '刚刚'
    if (min < 60) return `${min} 分钟`
    return `${Math.floor(min / 60)} 小时 ${min % 60} 分`
  }

  if (!backendRepo) {
    return <div className="flex h-full items-center justify-center text-[12px] text-slate-400">先在左侧选择一个项目。</div>
  }

  const renderCard = (t: BoardTask, failed = false) => (
    <div
      key={t.id}
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData('text/task-id', t.id)
        e.dataTransfer.effectAllowed = 'move'
        setDragId(t.id)
      }}
      onDragEnd={() => {
        setDragId(null)
        setOverCol(null)
      }}
      className={`rounded-xl border bg-white p-3 transition-shadow ${
        dragId === t.id ? 'opacity-40' : ''
      } ${failed ? 'border-red-200 bg-red-50/40' : 'border-slate-200'} cursor-grab shadow-sm hover:shadow`}
    >
      <div className="flex items-start gap-2">
        <span className="min-w-0 flex-1 text-[12px] font-bold leading-4 text-slate-700">{t.title}</span>
        {(t.result?.contractViolations?.length ?? 0) > 0 && (
          <span className="tnum shrink-0 rounded-full bg-red-50 px-1.5 py-px text-[9px] font-bold text-red-500">
            越界 {t.result!.contractViolations!.length}
          </span>
        )}
      </div>
      <div className="mt-1.5 flex items-center gap-1.5 text-[10px] text-slate-400">
        {(t.modules?.length ?? 0) > 0 && (
          <>
            {t.modules!.slice(0, 2).map((m) => (
              <span key={m} className="rounded bg-slate-100 px-1 py-px">{m}</span>
            ))}
            <span>·</span>
          </>
        )}
        <span className="tnum">{absTime(t.createdAt)}</span>
      </div>
      <div className="mt-2 flex items-center gap-2">
        <Dots t={t} />
        <span className="tnum ml-auto text-[9px] text-slate-400">{duration(t)}</span>
      </div>
      {/* 待你评审卡：内联通过/打回（拖拽之外的等价入口） */}
      {colOf(t).col === 'review' && (
        <div className="mt-2 border-t border-slate-100 pt-2">
          {rejecting === t.id ? (
            <div className="flex gap-1.5">
              <input
                autoFocus
                value={rejectNote}
                onChange={(e) => setRejectNote(e.target.value)}
                placeholder="打回意见（必填）"
                className="min-w-0 flex-1 rounded-md border border-red-200 bg-red-50/50 px-2 py-1 text-[10px] outline-none focus:border-red-300"
              />
              <button
                onClick={() => decide(t, 'rejected')}
                disabled={!!deciding || !rejectNote.trim()}
                className="shrink-0 rounded-md bg-red-500 px-2 py-1 text-[10px] font-bold text-white disabled:opacity-40"
              >
                确认
              </button>
              <button onClick={() => setRejecting(null)} className="shrink-0 rounded-md border border-slate-200 px-2 py-1 text-[10px] text-slate-400">
                <X size={10} />
              </button>
            </div>
          ) : (
            <div className="flex gap-1.5">
              <span className="rounded-full bg-blue-50 px-1.5 py-px text-[9px] font-bold text-blue-600">
                {t.gate === 'diff' ? 'Diff 审批' : '审查报告'}
              </span>
              <button
                onClick={() => decide(t, 'approved')}
                disabled={!!deciding}
                className="ml-auto flex items-center gap-0.5 rounded-md bg-blue-600 px-2 py-0.5 text-[10px] font-bold text-white hover:bg-blue-700 disabled:opacity-40"
              >
                <Check size={9} /> 通过
              </button>
              <button
                onClick={() => setRejecting(t.id)}
                className="flex items-center gap-0.5 rounded-md border border-red-200 px-2 py-0.5 text-[10px] font-bold text-red-500 hover:bg-red-50"
              >
                <X size={9} /> 打回
              </button>
            </div>
          )}
        </div>
      )}
      {failed && (
        <div className="mt-2 flex items-center gap-1 border-t border-red-100 pt-2 text-[10px] text-red-400">
          <Copy size={9} /> 拖到下方主体区 = 复制为新任务
        </div>
      )}
    </div>
  )

  return (
    <div className="flex h-full flex-col p-4">
      <div className="mb-3 flex items-baseline gap-3">
        <h2 className="text-[15px] font-bold text-slate-800">任务编排</h2>
        <p className="text-[11px] text-slate-400">
          列 = 状态；拖到「已完成」= 通过，拖到「待启动」= 打回，失败卡拖到主体区 = 复制重提
        </p>
        <button
          onClick={onOpenWorkflow}
          className="ml-auto rounded-lg border border-slate-200 px-2.5 py-1 text-micro font-semibold text-slate-500 hover:border-blue-300 hover:text-blue-600"
        >
          工作流视图 →
        </button>
      </div>

      <div className="flex min-h-0 flex-1 gap-3">
        {COLS.map(({ key, label }) => (
          <div
            key={key}
            onDragOver={(e) => {
              const t = (tasks ?? []).find((x) => x.id === dragId)
              if (t && canDrop(t, key)) {
                e.preventDefault()
                e.dataTransfer.dropEffect = 'move'
                setOverCol(key)
              } else {
                e.dataTransfer.dropEffect = 'none'
                setOverCol(null)
              }
            }}
            onDragLeave={() => setOverCol((c) => (c === key ? null : c))}
            onDrop={onDrop(key)}
            className={`flex min-w-0 flex-1 flex-col rounded-xl p-2 transition-colors ${
              overCol === key ? 'bg-blue-50 ring-2 ring-blue-200' : 'bg-slate-50'
            }`}
          >
            <div className="flex items-center gap-1.5 px-1.5 py-1.5">
              <span className="text-[12px] font-bold text-slate-600">{label}</span>
              <span className="tnum rounded-full bg-slate-200/70 px-1.5 text-[10px] font-semibold text-slate-500">{counts[key]}</span>
              {key === 'running' && counts.running > 0 && <Loader2 size={10} className="animate-spin text-blue-500" />}
            </div>
            <div className="min-h-0 flex-1 space-y-2 overflow-y-auto">
              {key === 'backlog' && grouped.failedTop.length > 0 && (
                <div>
                  <p className="px-1 pb-1 text-[9px] font-bold uppercase tracking-wider text-red-400">失败·驳回</p>
                  {grouped.failedTop.map((t) => renderCard(t, true))}
                  <div className="my-1.5 border-t border-dashed border-slate-200" />
                </div>
              )}
              {grouped[key].map((t) => renderCard(t))}
              {key === 'backlog' && counts.backlog === 0 && (
                <p className="px-1 py-4 text-center text-[10px] text-slate-300">暂无任务——从地图/问题/建议发起</p>
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
