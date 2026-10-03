import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { CheckCircle2, XCircle, Loader2, FileCode2, Lock, ClipboardList, ShieldAlert, Copy, Terminal, Unplug, FileText } from 'lucide-react'
import { toast } from '@/lib/toast'
import { onTaskEvent } from '@/lib/growthBus'
import { absTime, toMs } from '@/lib/diffStat'
import { terminalLines } from '@/lib/terminalBuffer'
import { stageOf, gateLabel, STAGES } from '@/lib/taskStage'
import type { CodeMap } from '@/types/map'
import type { TaskDraft } from '@/lib/taskContext'

// 任务工作流页（方案 v3 §4.2 施工 + 2026-10-03 分阶段流扩展）：
// 五阶段管道头 + 阶段单态主区。分阶段后 ①② 各有三态：
//   plan（任务书待批）/ p:analysis·p:solution（agent 正在产文档，走终端）/
//   analysis·solution（文档待评审，走全文评审卡——通过才进下一阶段）。
// 实时终端（模块级环形缓冲，切页不丢；断线无回放显灰条不造假）。
// 阶段判定走 lib/taskStage 的 status 优先映射（failed 残留 gate 不制造假阶段）。

/** 阶段产物评审卡：拉该阶段产物文档全文 + 通过/打回（需求矩阵/方案设计的评审载体） */
function PhaseDocReview({
  backendRepo,
  taskId,
  dirHint,
  title,
  deciding,
  onDecide,
}: {
  backendRepo: string
  taskId: string
  /** 产物目录特征串：1_requirements_matrix / 2_requirements_solutions */
  dirHint: string
  title: string
  deciding: string | null
  onDecide: (d: 'approved' | 'rejected', note?: string) => void
}) {
  const [doc, setDoc] = useState<{ path: string; content: string } | null>(null)
  const [missing, setMissing] = useState(false)
  const [rejecting, setRejecting] = useState(false)
  const [note, setNote] = useState('')

  useEffect(() => {
    let dead = false
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/dev-docs?taskId=${encodeURIComponent(taskId)}`)
      .then((r) => (r.ok ? r.json() : null))
      .then(async (d: { data?: { docs?: { path: string; mtime: number }[] } } | null) => {
        const hit = (d?.data?.docs ?? [])
          .filter((x) => x.path.includes(dirHint))
          .sort((a, b) => b.mtime - a.mtime)[0]
        if (!hit) {
          if (!dead) setMissing(true)
          return
        }
        const full = await fetch(
          `/api/repos/${encodeURIComponent(backendRepo)}/dev-doc?path=${encodeURIComponent(hit.path)}`,
        ).then((r) => (r.ok ? r.json() : null))
        if (!dead) setDoc({ path: hit.path, content: full?.data?.content ?? '' })
      })
      .catch(() => {})
    return () => {
      dead = true
    }
  }, [backendRepo, taskId, dirHint])

  return (
    <div className="m-4 flex min-h-0 flex-1 flex-col rounded-xl border border-slate-200 bg-white">
      <div className="flex items-center gap-2 border-b border-slate-100 px-4 py-3">
        <FileText size={13} className="text-blue-600" />
        <div className="min-w-0 flex-1">
          <p className="text-[13px] font-bold text-slate-800">{title}</p>
          {doc && <p className="mono mt-0.5 truncate text-[10px] text-slate-400">{doc.path}</p>}
        </div>
        <span className="shrink-0 rounded-full bg-amber-100 px-2 py-0.5 text-micro font-bold text-amber-700">等待你的评审</span>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        {!doc && !missing && (
          <p className="flex items-center gap-2 py-8 text-center text-[12px] text-slate-400">
            <Loader2 size={13} className="animate-spin" /> 正在加载产物文档…
          </p>
        )}
        {missing && (
          <div className="rounded-lg border border-amber-200 bg-amber-50 px-3 py-2.5 text-[12px] leading-5 text-amber-700">
            未找到本阶段产物文档（agent 未按规范路径产出）。你可以打回要求重做，或通过进入下一阶段（实施时将无矩阵/方案可依）。
          </div>
        )}
        {doc && (
          <pre className="whitespace-pre-wrap font-mono text-[11.5px] leading-5 text-slate-700">{doc.content}</pre>
        )}
      </div>
      <div className="border-t border-slate-100 p-3">
        {rejecting ? (
          <div className="space-y-1.5">
            <textarea
              autoFocus
              value={note}
              onChange={(e) => setNote(e.target.value)}
              rows={2}
              placeholder="打回意见（必填）——agent 将带着意见重做本阶段"
              className="w-full resize-none rounded-lg border border-red-200 bg-red-50/40 px-2.5 py-1.5 text-[12px] outline-none focus:border-red-300"
            />
            <div className="flex gap-2">
              <button
                onClick={() => onDecide('rejected', note.trim())}
                disabled={!!deciding || !note.trim()}
                className="flex-1 rounded-lg bg-red-600 px-3 py-1.5 text-[12px] font-bold text-white hover:bg-red-700 disabled:opacity-40"
              >
                确认打回本阶段
              </button>
              <button onClick={() => setRejecting(false)} className="rounded-lg border border-slate-200 px-3 py-1.5 text-[12px] text-slate-500">
                取消
              </button>
            </div>
          </div>
        ) : (
          <div className="flex gap-2">
            <button
              onClick={() => onDecide('approved')}
              disabled={!!deciding}
              className="flex flex-1 items-center justify-center gap-1 rounded-lg bg-blue-600 px-3 py-2 text-[12px] font-bold text-white hover:bg-blue-700 disabled:opacity-40"
            >
              {deciding === 'approved' ? <Loader2 size={12} className="animate-spin" /> : <CheckCircle2 size={12} />} 评审通过，进入下一阶段
            </button>
            <button
              onClick={() => setRejecting(true)}
              disabled={!!deciding}
              className="flex-1 rounded-lg border border-red-200 bg-white px-3 py-2 text-[12px] font-bold text-red-600 hover:bg-red-50 disabled:opacity-40"
            >
              打回重做…
            </button>
          </div>
        )}
      </div>
    </div>
  )
}

interface TaskItem {
  id: string
  title: string
  description: string
  status: string
  gate: string | null
  trust: string
  sessionId?: string | null
  modules?: string[]
  acceptance?: string
  error?: string | null
  createdAt?: string
  updatedAt?: string
  result?: { diffStat?: string; contractViolations?: string[]; warnings?: string[]; review?: { verdict: string; summary: string } } | null
}

interface Approval {
  id: string
  gate: string
  decision: string // approved / rejected / skipped / flagged
  note: string | null
  decidedAt: string
}

interface DevDoc {
  name: string
  path: string
  mtime: number
  excerpt: string
}

const STATUS_LABEL: Record<string, string> = {
  pending: '排队中',
  running: '执行中',
  awaiting_approval: '等待审批',
  done: '已完成',
  failed: '失败',
  rejected: '已驳回',
  interrupted: '已中断',
}

export function TaskWorkflowPage({
  backendRepo,
  map,
  onCreateTask,
  focusTask,
}: {
  backendRepo: string | null
  map: CodeMap
  /** 打回/失败 → 复制为新任务（origin_task_id 血缘，D3 拍板语义） */
  onCreateTask: (d: TaskDraft) => void
  /** v4 修订：看板点卡跳入——按 nonce 选中对应任务（首次挂载也生效） */
  focusTask?: { id: string; nonce: number } | null
}) {
  const [tasks, setTasks] = useState<TaskItem[] | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  // 看板点卡跳入：nonce 变化即选中（含组件常驻后的每次跳入）
  useEffect(() => {
    if (focusTask?.id) setSelected(focusTask.id)
  }, [focusTask])
  // 详情快照（taskId 归属防过期响应，ChangesPage 同模式）
  const [detailFor, setDetailFor] = useState<{ taskId: string; approvals: Approval[]; diffFull: string | null; diffStat: string | null; docs: DevDoc[]; docsAt: number } | null>(null)
  const [fileSel, setFileSel] = useState<{ taskId: string | null; file: string | null }>({ taskId: null, file: null })
  const selKey = selected
  if (fileSel.taskId !== selKey) setFileSel({ taskId: selKey, file: null })
  const [deciding, setDeciding] = useState<string | null>(null)
  const [rejectNote, setRejectNote] = useState('')
  const [rejecting, setRejecting] = useState(false)
  // 终端跟随滚动（用户上翻时暂停跟随）
  const termRef = useRef<HTMLPreElement | null>(null)
  const [follow, setFollow] = useState(true)
  // 终端行数戳：模块级缓冲不触发渲染，靠 1s 节拍与任务事件刷新
  const [termTick, setTermTick] = useState(0)

  const load = useCallback(() => {
    if (!backendRepo) return
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/tasks`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: TaskItem[] } | null) => {
        if (!d?.data) return
        const rank = (t: TaskItem) => (t.status === 'awaiting_approval' ? 0 : t.status === 'running' ? 1 : 2)
        setTasks([...d.data].sort((a, b) => rank(a) - rank(b)))
      })
      .catch(() => {})
  }, [backendRepo])

  useEffect(() => {
    load()
  }, [load])
  useEffect(() => onTaskEvent(load), [load])

  const sel = tasks?.find((t) => t.id === selected) ?? null

  useEffect(() => {
    if (!backendRepo || !sel) return
    const tid = sel.id
    const apply = (patch: Partial<NonNullable<typeof detailFor>>) =>
      setDetailFor((prev) => (prev && prev.taskId === tid ? { ...prev, ...patch } : { taskId: tid, approvals: [], diffFull: null, diffStat: null, docs: [], docsAt: 0, ...patch }))
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/tasks/${encodeURIComponent(tid)}/approvals`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: Approval[] } | null) => apply({ approvals: d?.data ?? [] }))
      .catch(() => {})
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/tasks/${encodeURIComponent(tid)}/diff`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { diff?: string | null; diffStat?: string | null } } | null) =>
        apply({ diffFull: d?.data?.diff ?? null, diffStat: d?.data?.diffStat ?? null }),
      )
      .catch(() => {})
    // 产物文档（方案 v3 §4.4）：终态/审批关才拉，running 期文档窗随 now() 扩张——
    // 带 5s 最小间隔，轮询节拍复用终端 tick
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/dev-docs?taskId=${encodeURIComponent(tid)}`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { docs?: DevDoc[] } } | null) => apply({ docs: d?.data?.docs ?? [], docsAt: Date.now() }))
      .catch(() => {})
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [backendRepo, sel?.id, sel?.status, sel?.gate])

  // 终端 1s 节拍 + 实施中才走（性能：空闲页零开销）
  const isRunning = sel?.status === 'running'
  useEffect(() => {
    if (!isRunning) return
    const t = window.setInterval(() => setTermTick((n) => n + 1), 1000)
    return () => window.clearInterval(t)
  }, [isRunning])
  useEffect(() => {
    if (follow && termRef.current) termRef.current.scrollTop = termRef.current.scrollHeight
  }, [termTick, follow, sel?.sessionId])

  const detail = sel && detailFor?.taskId === sel.id ? detailFor : null
  const approvals = detail?.approvals ?? []
  const diffFull = detail?.diffFull ?? null
  const diffStat = detail?.diffStat ?? null
  const docs = detail?.docs ?? []
  const activeFile = fileSel.taskId === selKey ? fileSel.file : null
  // 评审轮回（方案 §4.2）：approvals 按时间渲染留痕条——"第1轮打回：缺测试矩阵 → 第2轮通过"
  const reviewTrail = useMemo(() => {
    const GLABEL: Record<string, string> = { plan: '任务书', analysis: '需求矩阵', solution: '方案', diff: 'Diff', report: '报告' }
    const DLABEL: Record<string, string> = { approved: '通过', rejected: '打回', skipped: '自动通过', flagged: '风险预评' }
    return [...approvals]
      .sort((a, b) => String(a.decidedAt).localeCompare(String(b.decidedAt)))
      .map((a) => `${GLABEL[a.gate] ?? a.gate}关${DLABEL[a.decision] ?? a.decision}${a.note ? `：${a.note}` : ''}`)
  }, [approvals])

  const files = useMemo(() => {
    if (!diffFull) return []
    const list: string[] = []
    for (const line of diffFull.split('\n')) {
      const m = line.match(/^diff --git a\/(.+?) b\//)
      if (m) list.push(m[1])
      else if (line.startsWith('+++ b/')) list.push(line.slice(6).trim())
    }
    return [...new Set(list)]
  }, [diffFull])

  const activeDiff = useMemo(() => {
    if (!diffFull) return null
    if (!activeFile) return diffFull
    const chunks = diffFull.split(/(?=^diff --git )/m)
    return chunks.find((c) => c.includes(` a/${activeFile} `) || c.includes(` b/${activeFile}`)) ?? diffFull
  }, [diffFull, activeFile])

  const DIFF_PAGE = 600
  const [diffState, setDiffState] = useState<{ src: string | null; limit: number }>({ src: null, limit: DIFF_PAGE })
  if (diffState.src !== activeDiff) setDiffState({ src: activeDiff, limit: DIFF_PAGE })
  const diffLines = useMemo(() => (activeDiff ? activeDiff.split('\n') : []), [activeDiff])

  const impact = useMemo(() => {
    if (!diffStat) return []
    let adds = 0
    let dels = 0
    let n = 0
    for (const line of diffStat.split('\n')) {
      const m = line.match(/^\s*.+?\s*\|\s*\d+\s*([+-]*)\s*$/)
      if (!m) continue
      n += 1
      adds += (m[1].match(/\+/g) ?? []).length
      dels += (m[1].match(/-/g) ?? []).length
    }
    return [{ adds, dels, files: n }]
  }, [diffStat])

  const decide = (decision: 'approved' | 'rejected', noteArg?: string) => {
    if (!backendRepo || !sel || deciding) return
    const note = noteArg ?? rejectNote
    if (decision === 'rejected' && !note.trim()) {
      toast('打回必须填写意见', 'error')
      return
    }
    setDeciding(decision)
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/tasks/${encodeURIComponent(sel.id)}/decide`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ decision, note: decision === 'rejected' ? note.trim() : undefined, gate: sel.gate }),
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        toast(decision === 'approved' ? '已通过' : '已打回')
        // diff 关通过只推进 report、卡留列——把真实状态说给用户（复审口径）
        if (decision === 'approved' && sel.gate === 'diff') toast('已通过 Diff 审批，还差终审（审查报告）', 'info')
        setRejecting(false)
        setRejectNote('')
        load()
      })
      .catch(() => toast('审批操作失败', 'error'))
      .finally(() => setDeciding(null))
  }

  const duration = (t: TaskItem) => {
    const a = toMs(t.createdAt ?? '')
    const b = toMs(t.updatedAt ?? '')
    if (!a || !b) return '—'
    const min = Math.floor(Math.max(0, b - a) / 60000)
    if (min < 1) return '刚刚'
    if (min < 60) return `${min} 分钟`
    return `${Math.floor(min / 60)} 小时 ${min % 60} 分`
  }

  if (!backendRepo) {
    return <p className="p-8 text-center text-[12px] text-slate-400">需要本地后端在线</p>
  }
  if (tasks === null) {
    return (
      <div className="flex h-full items-center justify-center gap-2 text-[12px] text-slate-400">
        <Loader2 size={14} className="animate-spin" /> 加载任务…
      </div>
    )
  }

  const stage = sel ? stageOf(sel.status, sel.gate) : null
  const pendingCount = tasks.filter((t) => t.status === 'awaiting_approval').length

  // 五阶段圆点态（①②同体：plan 关两格同亮；running 时①②直通 done 只亮③）
  // status 优先映射在 stageOf——failed 残留 gate 不进入这里
  const dotState = (i: number): 'done' | 'now' | 'todo' => {
    if (stage === 'error' || stage === null) return 'todo'
    if (stage === 'done') return 'done'
    if (stage === 0) return i <= 1 ? 'now' : 'todo' // ①②同格点亮
    if (i < stage) return 'done'
    if (i === stage) return 'now'
    return 'todo'
  }
  const dotCls = (s: 'done' | 'now' | 'todo') =>
    s === 'done' ? 'bg-emerald-500 text-white' : s === 'now' ? 'bg-blue-600 text-white' : 'bg-slate-100 text-slate-400'

  return (
    <div className="flex h-full">
      {/* 左列：任务列表 */}
      <aside className="flex w-72 shrink-0 flex-col border-r border-slate-200 bg-white">
        <div className="border-b border-slate-100 px-3 py-2.5">
          <span className="text-[13px] font-bold text-slate-700">
            任务列表
            {pendingCount > 0 && (
              <span className="tnum ml-1.5 rounded-full bg-red-500 px-1.5 text-micro font-bold leading-4 text-white">{pendingCount}</span>
            )}
          </span>
          <p className="mt-0.5 text-micro text-slate-400">待你审批的任务在前</p>
        </div>
        <div className="min-h-0 flex-1 space-y-0.5 overflow-y-auto p-1.5">
          {tasks.map((t) => (
            <button
              key={t.id}
              onClick={() => setSelected(t.id)}
              className={`w-full rounded-lg px-2.5 py-2 text-left ${
                t.id === selected ? 'bg-blue-50 ring-1 ring-blue-200' : 'hover:bg-slate-50'
              }`}
            >
              <div className="flex items-center gap-1.5">
                <span className={`min-w-0 flex-1 truncate text-[12px] font-semibold ${t.id === selected ? 'text-blue-700' : 'text-slate-700'}`}>
                  {t.title}
                </span>
                {t.status === 'awaiting_approval' && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-amber-500" />}
              </div>
              <p className="mt-0.5 flex items-center gap-1.5 text-micro text-slate-400">
                <span>{STATUS_LABEL[t.status] ?? t.status}</span>
                {gateLabel(t.status, t.gate) && (
                  <span className="rounded-full bg-slate-100 px-1.5">{gateLabel(t.status, t.gate)}</span>
                )}
                <span className="tnum">{t.trust === 'auto' ? '自动' : t.trust === 'supervised' ? '监督' : '手动'}</span>
              </p>
            </button>
          ))}
          {tasks.length === 0 && <p className="px-2 py-6 text-center text-[11px] text-slate-400">暂无任务——从地图/问题/建议发起一个</p>}
        </div>
      </aside>

      {/* 右列：工作流主体 */}
      <div className="flex min-w-0 flex-1 flex-col">
        {!sel || stage === null ? (
          <p className="flex flex-1 items-center justify-center text-[12px] text-slate-400">选择左侧任务查看工作流</p>
        ) : (
          <>
            {/* 头部：标题 + 五阶段管道 */}
            <div className="border-b border-slate-100 bg-white px-4 py-3">
              <div className="flex items-center gap-2">
                <h2 className="min-w-0 flex-1 truncate text-[14px] font-bold text-slate-800">{sel.title}</h2>
                <span
                  className={`shrink-0 rounded-full px-2 py-0.5 text-micro font-bold ${
                    sel.status === 'awaiting_approval' ? 'bg-amber-100 text-amber-700' : sel.status === 'running' ? 'bg-blue-50 text-blue-600' : sel.status === 'done' ? 'bg-emerald-50 text-emerald-600' : 'bg-slate-100 text-slate-500'
                  }`}
                >
                  {STATUS_LABEL[sel.status] ?? sel.status}
                </span>
                <span className="tnum shrink-0 rounded-full bg-slate-100 px-2 py-0.5 text-micro font-semibold text-slate-500">{duration(sel)}</span>
              </div>
              {/* 五阶段管道：status 优先映射（taskStage），①②同体 */}
              <div className="mt-3 flex items-center gap-1">
                {STAGES.map((s, i) => (
                  <div key={s.key} className="flex flex-1 items-center gap-1">
                    <span className={`flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-micro font-bold ${dotCls(dotState(i))}`}>
                      {dotState(i) === 'done' ? <CheckCircle2 size={11} /> : i + 1}
                    </span>
                    <span className="min-w-0">
                      <span className="block truncate text-micro font-semibold text-slate-600">{s.label}</span>
                      <span className="block truncate text-[9px] text-slate-400">{stage === 'error' ? '—' : s.hint}</span>
                    </span>
                    {i < STAGES.length - 1 && (
                      <span className={`h-px flex-1 ${dotState(i) === 'done' ? 'bg-emerald-400' : 'bg-slate-200'}`} />
                    )}
                  </div>
                ))}
              </div>
              {/* 合约红线：审批必见 */}
              {(sel.result?.contractViolations?.length ?? 0) > 0 && (
                <div className="mt-2 rounded-lg border border-red-200 bg-red-50 px-3 py-2">
                  <p className="flex items-center gap-1 text-micro font-bold text-red-600">
                    <ShieldAlert size={10} /> 影响面合约：{sel.result!.contractViolations!.length} 个文件越出声明边界
                  </p>
                  <ul className="mt-1 space-y-0.5">
                    {sel.result!.contractViolations!.slice(0, 5).map((v) => (
                      <li key={v} className="mono truncate text-micro text-red-500">{v}</li>
                    ))}
                  </ul>
                </div>
              )}
              {/* 评审留痕：迭代到通过的全过程 */}
              {reviewTrail.length > 0 && (
                <p className="mt-2 truncate text-[10px] leading-4 text-slate-400">
                  <span className="font-bold text-slate-500">评审轮回：</span>
                  {reviewTrail.map((r, i) => (
                    <span key={i}>{i > 0 && ' → '}{r}</span>
                  ))}
                </p>
              )}
            </div>

            {/* 主区：阶段单态切换（同一时间只有一个阶段是 now） */}
            <div className="flex min-h-0 flex-1">
              <div className="flex min-w-0 flex-1 flex-col">
                {/* ① 需求分析：三子态——任务书待批 / 分析中终端 / 矩阵评审卡 */}
                {stage === 0 && sel.gate === 'plan' && !isRunning && (
                  <div className="m-4 rounded-xl border border-slate-200 bg-white p-4">
                    <p className="flex items-center gap-1 text-micro font-bold uppercase tracking-wider text-slate-400">
                      <ClipboardList size={10} /> 任务书 · 已就绪，批准后先做需求分析
                    </p>
                    {/* TaskPanel 碎片③：监督模式风险预评（flagged 留痕——审批人必见风险理由） */}
                    {(() => {
                      const flagged = approvals.find((a) => a.decision === 'flagged' && a.note)
                      return flagged ? (
                        <p className="mt-2 rounded-lg border border-amber-200 bg-amber-50 px-3 py-1.5 text-micro leading-4 text-amber-700">⚠ {flagged.note}</p>
                      ) : null
                    })()}
                    <p className="mt-2 line-clamp-6 text-[12px] leading-5 text-slate-700">{sel.description}</p>
                    {(sel.modules?.length ?? 0) > 0 && (
                      <div className="mt-2 flex flex-wrap gap-1">
                        {sel.modules!.map((mid) => (
                          <span key={mid} className="rounded-full bg-slate-50 px-2 py-px text-micro font-semibold text-slate-500 ring-1 ring-slate-200">
                            {map.modules.find((m) => m.id === mid)?.name ?? mid}
                          </span>
                        ))}
                      </div>
                    )}
                    {sel.acceptance && <p className="mt-2 text-micro leading-4 text-slate-400">验收：{sel.acceptance}</p>}
                    <div className="mt-3 flex gap-2">
                      <button
                        onClick={() => decide('approved')}
                        disabled={!!deciding}
                        className="flex flex-1 items-center justify-center gap-1 rounded-lg bg-blue-600 px-3 py-2 text-[12px] font-bold text-white hover:bg-blue-700 disabled:opacity-40"
                      >
                        {deciding === 'approved' ? <Loader2 size={12} className="animate-spin" /> : <CheckCircle2 size={12} />} 批准执行
                      </button>
                      <button
                        onClick={() => setRejecting(true)}
                        disabled={!!deciding}
                        className="flex-1 rounded-lg border border-red-200 bg-white px-3 py-2 text-[12px] font-bold text-red-600 hover:bg-red-50 disabled:opacity-40"
                      >
                        打回…
                      </button>
                    </div>
                    {rejecting && (
                      <div className="mt-2 space-y-1.5">
                        <textarea
                          autoFocus
                          value={rejectNote}
                          onChange={(e) => setRejectNote(e.target.value)}
                          rows={3}
                          placeholder="打回意见（必填）——将作为新任务的上下文"
                          className="w-full resize-none rounded-lg border border-red-200 bg-red-50/40 px-2.5 py-1.5 text-[12px] outline-none focus:border-red-300"
                        />
                        <div className="flex gap-2">
                          <button
                            onClick={() => decide('rejected')}
                            disabled={!!deciding || !rejectNote.trim()}
                            className="flex-1 rounded-lg bg-red-600 px-3 py-1.5 text-[12px] font-bold text-white hover:bg-red-700 disabled:opacity-40"
                          >
                            确认打回
                          </button>
                          <button onClick={() => setRejecting(false)} className="rounded-lg border border-slate-200 px-3 py-1.5 text-[12px] text-slate-500">
                            取消
                          </button>
                        </div>
                      </div>
                    )}
                  </div>
                )}

                {/* ① analysis 关：需求矩阵全文评审 */}
                {stage === 0 && sel.gate === 'analysis' && backendRepo && (
                  <PhaseDocReview
                    backendRepo={backendRepo}
                    taskId={sel.id}
                    dirHint="1_requirements_matrix"
                    title="需求矩阵评审"
                    deciding={deciding}
                    onDecide={(d, note) => decide(d, note)}
                  />
                )}

                {/* ② solution 关：方案设计全文评审 */}
                {stage === 1 && sel.gate === 'solution' && backendRepo && (
                  <PhaseDocReview
                    backendRepo={backendRepo}
                    taskId={sel.id}
                    dirHint="2_requirements_solutions"
                    title="方案设计评审"
                    deciding={deciding}
                    onDecide={(d, note) => decide(d, note)}
                  />
                )}

                {/* ③ 实施（及 ①② 产文档期间）：实时终端——按阶段标记换标题 */}
                {(stage === 2 || isRunning) && (
                  <div className="flex min-h-0 flex-1 flex-col p-4">
                    <div className="mb-2 flex items-center gap-2 text-micro text-slate-400">
                      <Terminal size={11} />
                      <span className="font-bold uppercase tracking-wider">
                        {sel.gate === 'p:analysis' ? '需求分析产出中' : sel.gate === 'p:solution' ? '方案设计产出中' : '实时执行'}
                      </span>
                      <span className="tnum ml-auto flex items-center gap-1.5">
                        <span className="flex h-1.5 w-1.5 animate-pulse rounded-full bg-red-500" /> LIVE · 已运行 {duration(sel)}
                      </span>
                    </div>
                    <pre
                      ref={termRef}
                      onScroll={(e) => {
                        const el = e.currentTarget
                        setFollow(el.scrollHeight - el.scrollTop - el.clientHeight < 24)
                      }}
                      className="mono min-h-0 flex-1 overflow-y-auto rounded-xl bg-slate-900 p-3 text-[11px] leading-5 text-slate-300"
                    >
                      {terminalLines(sel.sessionId ?? '').length === 0 ? (
                        <span className="text-slate-500">等待 agent 输出…（agent 启动可能需要 1-2 分钟）</span>
                      ) : (
                        terminalLines(sel.sessionId ?? '').map((l, i) => (
                          <div key={i} className={l.startsWith('[err]') ? 'text-red-400' : ''}>{l}</div>
                        ))
                      )}
                      {/* WS 断线无回放是已知边界（方案 §6）——明示不造假 */}
                      <div className="mt-1 flex items-center gap-1 text-slate-600">
                        <Unplug size={10} /> 断线期间的输出不可回放（直播通道无缓冲）
                      </div>
                    </pre>
                    <p className="mt-1.5 text-[10px] text-slate-400">
                      {sel.gate === 'p:analysis'
                        ? '需求矩阵将写入 .easyvibe/development_docs/1_requirements_matrix/，产出后在此评审'
                        : sel.gate === 'p:solution'
                          ? '方案设计将写入 2_requirements_solutions/，产出后在此评审'
                          : '改动文件列表在任务完成后由 Diff 呈现（实时全量文件流为后置需求）'}
                    </p>
                  </div>
                )}

                {/* ④ diff 关：子agent初审结论（有则置顶）+ 双栏查看器 */}
                {stage === 3 && (
                  <div className="flex min-h-0 flex-1 flex-col">
                    {sel.result?.review && (
                      <div
                        className={`mx-4 mt-3 flex items-start gap-2 rounded-lg border px-3 py-2 ${
                          sel.result.review.verdict === 'pass'
                            ? 'border-emerald-200 bg-emerald-50/60'
                            : 'border-amber-200 bg-amber-50/60'
                        }`}
                      >
                        <ShieldAlert size={11} className={sel.result.review.verdict === 'pass' ? 'mt-0.5 text-emerald-600' : 'mt-0.5 text-amber-600'} />
                        <div className="min-w-0">
                          <p className={`text-micro font-bold ${sel.result.review.verdict === 'pass' ? 'text-emerald-700' : 'text-amber-700'}`}>
                            子 agent 初审：{sel.result.review.verdict === 'pass' ? '通过' : '未通过'}
                          </p>
                          <p className="mt-0.5 line-clamp-2 text-micro leading-4 text-slate-600">{sel.result.review.summary}</p>
                        </div>
                      </div>
                    )}
                    <div className="flex min-h-0 flex-1">
                    <div className="w-52 shrink-0 overflow-y-auto border-r border-slate-100 bg-slate-50/50 p-2">
                      <p className="flex items-center gap-1 px-1.5 pb-1.5 text-micro font-bold uppercase tracking-wider text-slate-400">
                        <FileCode2 size={10} /> 文件（{files.length}）
                      </p>
                      {files.map((f) => (
                        <button
                          key={f}
                          onClick={() => setFileSel((fs) => ({ taskId: selKey, file: fs.file === f ? null : f }))}
                          className={`block w-full truncate rounded px-1.5 py-1 text-left font-mono text-micro ${
                            f === activeFile ? 'bg-blue-100 text-blue-700' : 'text-slate-500 hover:bg-slate-100'
                          }`}
                          title={f}
                        >
                          {f}
                        </button>
                      ))}
                      {files.length === 0 && <p className="px-1.5 py-2 text-micro text-slate-400">无 diff 数据</p>}
                    </div>
                    <div className="min-w-0 flex-1 overflow-auto bg-white p-3">
                      {activeDiff ? (
                        <pre className="mono text-cap leading-4">
                          {diffLines.slice(0, diffState.limit).map((line, i) => (
                            <div
                              key={i}
                              className={
                                line.startsWith('+') && !line.startsWith('+++')
                                  ? 'bg-emerald-50 text-emerald-700'
                                  : line.startsWith('-') && !line.startsWith('---')
                                    ? 'bg-red-50 text-red-600'
                                    : 'text-slate-600'
                              }
                            >
                              {line || ' '}
                            </div>
                          ))}
                          {diffLines.length > diffState.limit && (
                            <button
                              onClick={() => setDiffState((s) => ({ ...s, limit: s.limit + DIFF_PAGE }))}
                              className="mt-1 w-full rounded-md border border-dashed border-slate-200 py-1 text-cap font-semibold text-slate-400 hover:border-blue-300 hover:text-blue-600"
                            >
                              还有 {diffLines.length - diffState.limit} 行，点击加载更多
                            </button>
                          )}
                        </pre>
                      ) : (
                        <p className="py-8 text-center text-[11px] text-slate-400">该任务无变更归档</p>
                      )}
                    </div>
                    </div>
                  </div>
                )}

                {/* ⑤ report 关：审查报告 */}
                {stage === 4 && (
                  <div className="m-4 overflow-y-auto rounded-xl border border-slate-200 bg-white p-4">
                    <p className="flex items-center gap-1 text-micro font-bold uppercase tracking-wider text-slate-400">
                      <Lock size={10} /> 审查报告 · 通过后锁定归档
                    </p>
                    <div className="mt-2 space-y-1.5 text-[12px] leading-5 text-slate-700">
                      {impact.map((im, i) => (
                        <p key={i} className="tnum text-micro text-slate-500">
                          本次变更：<span className="text-emerald-600">+{im.adds}</span> <span className="text-red-500">−{im.dels}</span> · {im.files} 文件
                        </p>
                      ))}
                      {(sel.result?.warnings?.length ?? 0) > 0 && (
                        <ul className="mt-1 space-y-0.5">
                          {sel.result!.warnings!.map((w, i) => (
                            <li key={i} className="text-micro text-amber-600">⚠ {w}</li>
                          ))}
                        </ul>
                      )}
                      <p className="text-micro text-slate-400">审查报告全文随归档产出；通过后任务锁定，STAR 记忆与操作日志留痕。</p>
                    </div>
                    <div className="mt-3 flex gap-2">
                      <button
                        onClick={() => decide('approved')}
                        disabled={!!deciding}
                        className="flex flex-1 items-center justify-center gap-1 rounded-lg bg-blue-600 px-3 py-2 text-[12px] font-bold text-white hover:bg-blue-700 disabled:opacity-40"
                      >
                        {deciding === 'approved' ? <Loader2 size={12} className="animate-spin" /> : <CheckCircle2 size={12} />} 通过并归档
                      </button>
                      <button
                        onClick={() => setRejecting(true)}
                        disabled={!!deciding}
                        className="flex-1 rounded-lg border border-red-200 bg-white px-3 py-2 text-[12px] font-bold text-red-600 hover:bg-red-50 disabled:opacity-40"
                      >
                        打回…
                      </button>
                    </div>
                    {rejecting && (
                      <div className="mt-2 space-y-1.5">
                        <textarea
                          autoFocus
                          value={rejectNote}
                          onChange={(e) => setRejectNote(e.target.value)}
                          rows={3}
                          placeholder="打回意见（必填）"
                          className="w-full resize-none rounded-lg border border-red-200 bg-red-50/40 px-2.5 py-1.5 text-[12px] outline-none focus:border-red-300"
                        />
                        <div className="flex gap-2">
                          <button
                            onClick={() => decide('rejected')}
                            disabled={!!deciding || !rejectNote.trim()}
                            className="flex-1 rounded-lg bg-red-600 px-3 py-1.5 text-[12px] font-bold text-white hover:bg-red-700 disabled:opacity-40"
                          >
                            确认打回
                          </button>
                          <button onClick={() => setRejecting(false)} className="rounded-lg border border-slate-200 px-3 py-1.5 text-[12px] text-slate-500">
                            取消
                          </button>
                        </div>
                      </div>
                    )}
                  </div>
                )}

                {/* done：归档摘要（迁入 TaskPanel 独有碎片：复检按钮 + archivedPath） */}
                {stage === 'done' && (
                  <div className="m-4 overflow-y-auto rounded-xl border border-emerald-200 bg-emerald-50/40 p-4">
                    <p className="flex items-center gap-1 text-micro font-bold uppercase tracking-wider text-emerald-600">
                      <CheckCircle2 size={10} /> 已归档 · 全程留痕
                    </p>
                    {impact.map((im, i) => (
                      <p key={i} className="tnum mt-2 text-micro text-slate-500">
                        变更：<span className="text-emerald-600">+{im.adds}</span> <span className="text-red-500">−{im.dels}</span> · {im.files} 文件 · 完成于 {absTime(sel.updatedAt ?? '')}
                      </p>
                    ))}
                    {/* TaskPanel 碎片②：归档路径（P1 留在 done 卡，P2 挪治理视图） */}
                    {(sel as unknown as { result?: { archivedPath?: string | null } }).result?.archivedPath && (
                      <p className="mono mt-1 truncate text-micro text-slate-400" title={(sel as unknown as { result?: { archivedPath?: string } }).result?.archivedPath}>
                        已归档：{(sel as unknown as { result?: { archivedPath?: string } }).result?.archivedPath}
                      </p>
                    )}
                    <p className="mt-1 text-micro text-slate-400">STAR 记忆与操作日志见右侧产物文档。</p>
                    {/* TaskPanel 碎片①：重新巡检验证改动效果（治理闭环入口不能丢） */}
                    <button
                      onClick={() => {
                        if (!backendRepo) return
                        fetch(`/api/repos/${encodeURIComponent(backendRepo)}/patrol`, { method: 'POST' }).catch(() => {})
                        toast('巡检已启动——稍后到健康看板验证改善', 'info')
                      }}
                      className="mt-2 rounded-lg border border-emerald-300 bg-white px-3 py-1.5 text-micro font-semibold text-emerald-700 hover:bg-emerald-50"
                    >
                      重新巡检验证改动效果
                    </button>
                  </div>
                )}

                {/* 未启动/终态灰态：error + 复制入口 */}
                {stage === 'error' && (
                  <div className="m-4 rounded-xl border border-slate-200 bg-white p-4">
                    <p className="flex items-center gap-1 text-micro font-bold uppercase tracking-wider text-slate-400">
                      <XCircle size={10} /> {STATUS_LABEL[sel.status] ?? sel.status}
                    </p>
                    {sel.error && <p className="mt-2 rounded-lg bg-red-50 px-3 py-2 text-[12px] leading-5 text-red-600">{sel.error}</p>}
                    <button
                      onClick={() =>
                        onCreateTask({
                          title: `${sel.title}（重提）`,
                          description: sel.description,
                          modules: sel.modules ?? [],
                          acceptance: sel.acceptance ?? '',
                          source: 'manual',
                          context: { origin_task_id: sel.id },
                        })
                      }
                      className="mt-3 flex items-center gap-1 rounded-lg border border-slate-200 px-3 py-1.5 text-micro font-semibold text-slate-500 transition-colors hover:border-blue-300 hover:text-blue-600"
                    >
                      <Copy size={9} /> 复制为新任务
                    </button>
                  </div>
                )}
              </div>

              {/* 产物文档卡（④⑤ 与 done 的侧栏） */}
              {(stage === 3 || stage === 4 || stage === 'done') && (
                <aside className="flex w-64 shrink-0 flex-col border-l border-slate-100 bg-slate-50/40">
                  <p className="px-3 pt-3 text-micro font-bold uppercase tracking-wider text-slate-400">产物文档</p>
                  <div className="min-h-0 flex-1 space-y-1.5 overflow-y-auto p-2.5">
                    {docs.length === 0 && <p className="px-1 py-2 text-micro text-slate-400">本任务暂无产物文档</p>}
                    {docs.map((d) => (
                      <div key={d.path} className="rounded-lg border border-slate-200 bg-white p-2">
                        <p className="truncate text-[11px] font-semibold text-slate-700" title={d.name}>{d.name}</p>
                        <p className="mono mt-0.5 truncate text-[9px] text-slate-400" title={d.path}>{d.path}</p>
                        <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-500">{d.excerpt || '（空文档）'}</p>
                      </div>
                    ))}
                  </div>
                  <p className="px-3 pb-2.5 text-[9px] text-slate-300">路径规范：.easyvibe/development_docs/</p>
                </aside>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  )
}
