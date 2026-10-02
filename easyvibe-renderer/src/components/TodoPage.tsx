import { useCallback, useEffect, useMemo, useState } from 'react'
import { Check, ChevronDown, ChevronRight, CircleDot, Copy, Loader2, X } from 'lucide-react'
import { toast } from '@/lib/toast'
import { onTaskEvent } from '@/lib/growthBus'
import { absTime } from '@/lib/diffStat'
import type { TaskDraft } from '@/lib/taskContext'

// M4-2「我的待办」治理收件箱转正（按 ui-mockups/我的待办原型.png 施工，五步图纸裁定为三道关）：
// 全部把关闭环的汇流点——状态过滤丸（带计数）+ 任务行（内联展开三步关进度 +
// 快速通过/驳回）+ 越界红线徽标 + 失败组返工入口。深审跳评审中心，轻处分在收件箱完成。

interface TodoTask {
  id: string
  title: string
  status: string
  gate: string | null
  trust: string
  modules?: string[]
  createdAt: string
  updatedAt: string
  result?: { contractViolations?: string[]; warnings?: string[] } | null
}

interface Approval {
  id: string
  gate: string
  decision: string
  note: string | null
  decidedAt: string
}

type FilterKey = 'all' | 'running' | 'awaiting' | 'done' | 'failed'

const FILTERS: { key: FilterKey; label: string; match: (t: TodoTask) => boolean }[] = [
  { key: 'all', label: '全部', match: () => true },
  { key: 'running', label: '运行中', match: (t) => t.status === 'running' || t.status === 'pending' },
  { key: 'awaiting', label: '等待审批', match: (t) => t.status === 'awaiting_approval' },
  { key: 'done', label: '已完成', match: (t) => t.status === 'done' },
  { key: 'failed', label: '失败', match: (t) => ['failed', 'interrupted', 'rejected'].includes(t.status) },
]

const GATES = [
  { key: 'plan', label: '计划审批' },
  { key: 'diff', label: 'Diff 审批' },
  { key: 'report', label: '审查报告' },
]

const STATUS_META: Record<string, { label: string; cls: string }> = {
  pending: { label: '排队中', cls: 'bg-slate-100 text-slate-500' },
  running: { label: '运行中', cls: 'bg-amber-50 text-amber-600' },
  awaiting_approval: { label: '等待审批', cls: 'bg-blue-50 text-blue-600' },
  done: { label: '已完成', cls: 'bg-emerald-50 text-emerald-600' },
  failed: { label: '失败', cls: 'bg-red-50 text-red-500' },
  interrupted: { label: '已中断', cls: 'bg-slate-100 text-slate-500' },
  rejected: { label: '已驳回', cls: 'bg-red-50 text-red-500' },
}

export function TodoPage({
  backendRepo,
  onOpenReview,
  onCreateTask,
}: {
  backendRepo: string | null
  onOpenReview: () => void
  onCreateTask: (d: TaskDraft) => void
}) {
  const [tasks, setTasks] = useState<TodoTask[] | null>(null)
  const [loadError, setLoadError] = useState(false)
  const [filter, setFilter] = useState<FilterKey>('all')
  const [expanded, setExpanded] = useState<string | null>(null)
  // 审批记录快照（同 ChangesPage 模式）；驳回理由按任务记录
  const [approvalsFor, setApprovalsFor] = useState<{ taskId: string; list: Approval[] } | null>(null)
  const [rejecting, setRejecting] = useState<string | null>(null)
  const [rejectNote, setRejectNote] = useState('')
  const [deciding, setDeciding] = useState<string | null>(null)

  const load = useCallback(() => {
    if (!backendRepo) return
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/tasks`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data?: TodoTask[] } | null) => {
        setTasks(d?.data ?? [])
        setLoadError(false)
      })
      .catch(() => {
        setTasks([])
        setLoadError(true)
      })
  }, [backendRepo])

  useEffect(() => {
    load()
  }, [load])
  useEffect(() => onTaskEvent(load), [load])

  const counts = useMemo(() => {
    const c: Record<FilterKey, number> = { all: tasks?.length ?? 0, running: 0, awaiting: 0, done: 0, failed: 0 }
    for (const t of tasks ?? []) {
      for (const f of FILTERS) if (f.key !== 'all' && f.match(t)) c[f.key] += 1
    }
    return c
  }, [tasks])

  const rows = useMemo(() => {
    const f = FILTERS.find((x) => x.key === filter)!
    return [...(tasks ?? [])].filter(f.match).sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
  }, [tasks, filter])

  const expand = (t: TodoTask) => {
    const open = expanded === t.id
    setExpanded(open ? null : t.id)
    setRejecting(null)
    if (!open && backendRepo) {
      fetch(`/api/repos/${encodeURIComponent(backendRepo)}/tasks/${encodeURIComponent(t.id)}/approvals`)
        .then((r) => (r.ok ? r.json() : null))
        .then((d: { data?: Approval[] } | null) => setApprovalsFor({ taskId: t.id, list: d?.data ?? [] }))
        .catch(() => {})
    }
  }

  const decide = async (t: TodoTask, decision: 'approved' | 'rejected') => {
    if (!backendRepo || deciding) return
    if (decision === 'rejected' && !rejectNote.trim()) {
      setRejecting(t.id)
      return
    }
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
      toast(decision === 'approved' ? '已通过' : '已驳回')
      setRejecting(null)
      setRejectNote('')
      load()
    } finally {
      setDeciding(null)
    }
  }

  if (!backendRepo) {
    return <div className="flex h-full items-center justify-center text-[12px] text-slate-400">先在左侧选择一个项目。</div>
  }

  const duration = (t: TodoTask) => {
    const ms = Math.max(0, Date.parse(t.updatedAt) - Date.parse(t.createdAt))
    if (ms <= 0) return '—'
    const min = Math.floor(ms / 60000)
    if (min < 1) return '刚刚'
    if (min < 60) return `${min} 分钟`
    return `${Math.floor(min / 60)} 小时 ${min % 60} 分`
  }

  return (
    <div className="h-full overflow-y-auto p-5">
      <div className="mb-4">
        <h2 className="text-[15px] font-bold text-slate-800">我的待办</h2>
        <p className="mt-0.5 text-[11px] text-slate-400">查看并处理分配给你的任务，推动架构治理与变更落地。</p>
      </div>

      {/* 状态过滤丸（带计数） */}
      <div className="mb-4 flex gap-2">
        {FILTERS.map((f) => (
          <button
            key={f.key}
            onClick={() => setFilter(f.key)}
            className={`flex items-center gap-1.5 rounded-full border px-3 py-1.5 text-[12px] font-semibold transition-colors ${
              filter === f.key ? 'border-blue-600 bg-blue-600 text-white' : 'border-slate-200 bg-white text-slate-500 hover:border-blue-300 hover:text-blue-600'
            }`}
          >
            {f.label}
            <span className={`tnum rounded-full px-1.5 text-micro ${filter === f.key ? 'bg-white/20' : 'bg-slate-100'}`}>{counts[f.key]}</span>
          </button>
        ))}
        {loadError && <span className="ml-auto self-center text-micro text-red-400">后端不在线，数据可能不是最新</span>}
      </div>

      {/* 任务行 */}
      <div className="space-y-2">
        {rows.map((t) => {
          const open = expanded === t.id
          const meta = STATUS_META[t.status] ?? STATUS_META.pending
          const approvals = approvalsFor?.taskId === t.id ? approvalsFor.list : []
          const violations = t.result?.contractViolations?.length ?? 0
          const gateIdx = GATES.findIndex((g) => g.key === t.gate)
          return (
            <div key={t.id} className={`overflow-hidden rounded-xl border bg-white ${open ? 'border-blue-200' : 'border-slate-200'}`}>
              <div className="flex items-center gap-3 px-4 py-3">
                <button onClick={() => expand(t)} className="flex min-w-0 flex-1 items-center gap-2.5 text-left">
                  {open ? <ChevronDown size={14} className="shrink-0 text-slate-400" /> : <ChevronRight size={14} className="shrink-0 text-slate-300" />}
                  <span className="min-w-0 flex-1">
                    <span className="flex items-center gap-2">
                      <span className="truncate text-[13px] font-bold text-slate-700">{t.title}</span>
                      {/* 越界红线徽标：收件箱是合约告警的汇流点 */}
                      {violations > 0 && (
                        <span className="tnum shrink-0 rounded-full bg-red-50 px-1.5 py-px text-micro font-bold text-red-500">
                          越界 {violations}
                        </span>
                      )}
                    </span>
                    <span className="mt-0.5 flex items-center gap-2 text-micro text-slate-400">
                      <span>{backendRepo}</span>
                      {(t.modules?.length ?? 0) > 0 && <span>{t.modules!.length} 个模块</span>}
                      <span className="tnum">{absTime(t.createdAt)}</span>
                    </span>
                  </span>
                </button>
                <span className={`flex shrink-0 items-center gap-1 rounded-full px-2 py-0.5 text-micro font-semibold ${meta.cls}`}>
                  {t.status === 'running' && <Loader2 size={9} className="animate-spin" />}
                  {meta.label}
                </span>
                <span className="tnum w-20 shrink-0 text-right text-micro text-slate-400">{duration(t)}</span>
                <button
                  onClick={onOpenReview}
                  className="shrink-0 rounded-lg border border-slate-200 px-2.5 py-1 text-micro font-semibold text-slate-500 transition-colors hover:border-blue-300 hover:text-blue-600"
                >
                  查看
                </button>
              </div>

              {open && (
                <div className="anim-scale-in border-t border-slate-100 px-4 py-3">
                  {/* 三道关进度（图纸五步已裁定为三关） */}
                  <p className="mb-2 text-micro font-bold uppercase tracking-wider text-slate-400">当前步骤</p>
                  <div className="flex items-center gap-1">
                    {GATES.map((g, i) => {
                      const d = approvals.find((a) => a.gate === g.key)?.decision
                      const active = i === (t.status === 'done' ? GATES.length : gateIdx) && (t.status === 'awaiting_approval' || t.status === 'done')
                      return (
                        <div key={g.key} className="flex flex-1 items-center gap-1.5">
                          <span
                            className={`flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-micro font-bold ${
                              d === 'approved' || d === 'skipped'
                                ? 'bg-blue-600 text-white'
                                : d === 'rejected'
                                  ? 'bg-red-500 text-white'
                                  : active
                                    ? 'bg-blue-600 text-white'
                                    : 'bg-slate-100 text-slate-400'
                            }`}
                          >
                            {d === 'approved' || d === 'skipped' ? <Check size={10} /> : d === 'rejected' ? <X size={10} /> : i + 1}
                          </span>
                          <span className={`text-micro ${active ? 'font-bold text-blue-600' : 'text-slate-500'}`}>{g.label}</span>
                          {i < GATES.length - 1 && <span className={`h-px flex-1 ${d ? 'bg-blue-400' : 'bg-slate-100'}`} />}
                        </div>
                      )
                    })}
                  </div>

                  {/* 行内动作：等待审批 → 快速通过/驳回（深审去评审中心）；驳回 → 返工入口 */}
                  <div className="mt-3 flex items-center gap-2">
                    <CircleDot size={11} className="text-slate-300" />
                    {t.status === 'awaiting_approval' ? (
                      rejecting === t.id ? (
                        <>
                          <input
                            autoFocus
                            value={rejectNote}
                            onChange={(e) => setRejectNote(e.target.value)}
                            placeholder="驳回理由（必填，留痕可追溯）"
                            className="min-w-0 flex-1 rounded-md border border-red-200 bg-red-50/50 px-2 py-1 text-micro text-slate-700 outline-none focus:border-red-300"
                          />
                          <button
                            onClick={() => decide(t, 'rejected')}
                            disabled={!!deciding || !rejectNote.trim()}
                            className="shrink-0 rounded-lg bg-red-500 px-3 py-1 text-micro font-bold text-white disabled:opacity-40"
                          >
                            确认驳回
                          </button>
                          <button onClick={() => setRejecting(null)} className="shrink-0 rounded-lg border border-slate-200 px-2 py-1 text-micro text-slate-400">
                            取消
                          </button>
                        </>
                      ) : (
                        <>
                          <button
                            onClick={() => decide(t, 'approved')}
                            disabled={!!deciding}
                            className="flex shrink-0 items-center gap-1 rounded-lg bg-blue-600 px-4 py-1.5 text-[12px] font-bold text-white transition-colors hover:bg-blue-700 disabled:opacity-40"
                          >
                            <Check size={11} /> 通过
                          </button>
                          <button
                            onClick={() => setRejecting(t.id)}
                            className="flex shrink-0 items-center gap-1 rounded-lg border border-red-200 px-4 py-1.5 text-[12px] font-bold text-red-500 transition-colors hover:bg-red-50"
                          >
                            <X size={11} /> 驳回
                          </button>
                          <span className="text-micro ml-1 text-slate-300">快速处分；看 diff 请点「查看」去评审中心</span>
                        </>
                      )
                    ) : t.status === 'rejected' ? (
                      <button
                        onClick={() =>
                          onCreateTask({
                            title: `${t.title}（重提）`,
                            description: t.title,
                            modules: t.modules ?? [],
                            acceptance: '',
                            source: 'manual',
                            context: { origin_task_id: t.id },
                          })
                        }
                        className="flex shrink-0 items-center gap-1 rounded-lg border border-slate-200 px-3 py-1 text-micro font-semibold text-slate-500 transition-colors hover:border-blue-300 hover:text-blue-600"
                      >
                        <Copy size={9} /> 复制为新任务
                      </button>
                    ) : (
                      <span className="text-micro text-slate-300">
                        {t.status === 'running' ? 'agent 执行中，终态后回到这里处分' : '该任务当前无需处分'}
                      </span>
                    )}
                  </div>
                </div>
              )}
            </div>
          )
        })}
        {rows.length === 0 && (
          <p className="py-14 text-center text-[12px] text-slate-300">
            {tasks === null ? '加载中…' : loadError ? '后端不在线，稍后自动重试。' : '这个分类下没有任务。'}
          </p>
        )}
      </div>
    </div>
  )
}
