import { useCallback, useEffect, useMemo, useState } from 'react'
import { CheckCircle2, XCircle, ShieldCheck, GitCompareArrows, Loader2, FileCode2, Lock, Copy } from 'lucide-react'
import { toast } from '@/lib/toast'
import { onTaskEvent } from '@/lib/growthBus'
import type { CodeMap } from '@/types/map'
import type { TaskDraft } from '@/lib/taskContext'

// M4-2 评审中心整页（按 ui-mockups/审批中心原型.png 施工）：
// 左列待审批任务（徽标=待审批数）｜右列三道关进度 + 双栏 Diff（文件索引｜diff 内容）
// + 模块聚合影响面 + 通过/驳回（驳回理由必填）+ 报告锁态。
// 数据全部来自任务系统既有端点（tasks/approvals/diff/decide），零新增后端。

interface TaskItem {
  id: string
  title: string
  description: string
  status: string
  gate: string | null
  trust: string
  modules?: string[]
  acceptance?: string
  result?: { diffStat?: string } | null
}

interface Approval {
  id: string
  gate: string
  decision: string // approved / rejected / skipped
  note: string | null
  decidedAt: string
}

const GATES = [
  { key: 'plan', label: '计划审批' },
  { key: 'diff', label: 'Diff 审批' },
  { key: 'report', label: '审查报告' },
]

const STATUS_LABEL: Record<string, string> = {
  pending: '排队中',
  running: '执行中',
  awaiting_approval: '等待审批',
  done: '已完成',
  failed: '失败',
  rejected: '已驳回',
}

export function ReviewPage({
  backendRepo,
  map,
  onCreateTask,
}: {
  backendRepo: string | null
  map: CodeMap
  /** 把关台范式：驳回不=任务死亡——以此为基础复制新任务（D3 拍板语义） */
  onCreateTask: (d: TaskDraft) => void
}) {
  const [tasks, setTasks] = useState<TaskItem[] | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  // 详情快照（ChangesPage 同模式）：切换任务时旧数据天然失效，effect 内不再有同步 setState
  const [detailFor, setDetailFor] = useState<{ taskId: string; approvals: Approval[]; diffFull: string | null; diffStat: string | null } | null>(null)
  // 当前选中文件的渲染期派生状态（React 官方模式：切任务即重置）
  const [fileSel, setFileSel] = useState<{ taskId: string | null; file: string | null }>({ taskId: null, file: null })
  const selKey = selected
  if (fileSel.taskId !== selKey) setFileSel({ taskId: selKey, file: null })
  const [deciding, setDeciding] = useState<string | null>(null)
  const [rejectNote, setRejectNote] = useState('')
  const [rejecting, setRejecting] = useState(false)

  const load = useCallback(() => {
    if (!backendRepo) return
    fetch(`/api/repos/${backendRepo}/tasks`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: TaskItem[] } | null) => {
        if (!d?.data) return
        // 待审批在前，其余按状态
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
      setDetailFor((prev) => (prev && prev.taskId === tid ? { ...prev, ...patch } : { taskId: tid, approvals: [], diffFull: null, diffStat: null, ...patch }))
    fetch(`/api/repos/${backendRepo}/tasks/${encodeURIComponent(tid)}/approvals`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: Approval[] } | null) => apply({ approvals: d?.data ?? [] }))
      .catch(() => {})
    fetch(`/api/repos/${backendRepo}/tasks/${encodeURIComponent(tid)}/diff`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { diff?: string | null; diffStat?: string | null } } | null) =>
        apply({ diffFull: d?.data?.diff ?? null, diffStat: d?.data?.diffStat ?? null }),
      )
      .catch(() => {})
    // 过期响应防护：快照按 taskId 归属，切换后旧响应落在旧 taskId 上、渲染端不读
  }, [backendRepo, sel?.id]) // eslint-disable-line react-hooks/exhaustive-deps

  // 渲染端按当前选中任务取值（快照不匹配 = 加载中）
  const detail = sel && detailFor?.taskId === sel.id ? detailFor : null
  const approvals = detail?.approvals ?? []
  const diffFull = detail?.diffFull ?? null
  const diffStat = detail?.diffStat ?? null
  const activeFile = fileSel.taskId === selKey ? fileSel.file : null

  // 双栏 Diff：解析 unified diff 的文件索引
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
    const hit = chunks.find((c) => c.includes(` a/${activeFile} `) || c.includes(` b/${activeFile}`))
    return hit ?? diffFull
  }, [diffFull, activeFile])

  // P1 审查 2#14 性能：超大 diff 全量渲染 = 上万 DOM 节点。首屏渲染前 600 行，渐进展开。
  const DIFF_PAGE = 600
  // 切换文件时重置展开进度——渲染期派生状态（React 官方模式，避免 effect 内同步 setState）
  const [diffState, setDiffState] = useState<{ src: string | null; limit: number }>({ src: null, limit: DIFF_PAGE })
  if (diffState.src !== activeDiff) setDiffState({ src: activeDiff, limit: DIFF_PAGE })
  const diffLimit = diffState.limit
  const diffLines = useMemo(() => (activeDiff ? activeDiff.split('\n') : []), [activeDiff])
  const visibleDiffLines = diffLines.slice(0, diffLimit)

  // 模块聚合影响面（与 WorkbenchPage 同口径）
  const impact = useMemo(() => {
    if (!diffStat) return []
    const perFile: { adds: number; dels: number }[] = []
    for (const line of diffStat.split('\n')) {
      const m = line.match(/^\s*.+?\s*\|\s*\d+\s*([+-]*)\s*$/)
      if (!m) continue
      perFile.push({ adds: (m[1].match(/\+/g) ?? []).length, dels: (m[1].match(/-/g) ?? []).length })
    }
    // diffStat 是纯文本，无文件路径明细时给整体口径
    const adds = perFile.reduce((s, f) => s + f.adds, 0)
    const dels = perFile.reduce((s, f) => s + f.dels, 0)
    return [{ name: '本次变更合计', adds, dels, files: perFile.length }]
  }, [diffStat])

  const decide = (decision: 'approved' | 'rejected') => {
    if (!backendRepo || !sel || deciding) return
    if (decision === 'rejected' && !rejectNote.trim()) {
      toast('驳回必须填写理由', 'error')
      return
    }
    setDeciding(decision)
    fetch(`/api/repos/${backendRepo}/tasks/${encodeURIComponent(sel.id)}/decide`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ decision, note: decision === 'rejected' ? rejectNote.trim() : undefined }),
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        toast(decision === 'approved' ? '已通过' : '已驳回')
        setRejecting(false)
        setRejectNote('')
        load()
      })
      .catch(() => toast('审批操作失败', 'error'))
      .finally(() => setDeciding(null))
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

  const pendingCount = tasks.filter((t) => t.status === 'awaiting_approval').length
  const gateDecision = (key: string) => approvals.find((a) => a.gate === key)?.decision ?? null

  return (
    <div className="flex h-full">
      {/* 左列：任务列表 */}
      <aside className="flex w-72 shrink-0 flex-col border-r border-slate-200 bg-white">
        <div className="border-b border-slate-100 px-3 py-2.5">
          <span className="text-[13px] font-bold text-slate-700">
            评审
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
                {t.gate && <span className="rounded-full bg-slate-100 px-1.5">{GATES.find((g) => g.key === t.gate)?.label ?? t.gate}</span>}
                <span className="tnum">{t.trust === 'auto' ? '自动' : '手动'}</span>
              </p>
            </button>
          ))}
          {tasks.length === 0 && <p className="px-2 py-6 text-center text-[11px] text-slate-400">暂无任务——从地图/问题/建议发起一个</p>}
        </div>
      </aside>

      {/* 右列：审批详情 */}
      <div className="flex min-w-0 flex-1 flex-col">
        {!sel ? (
          <p className="flex flex-1 items-center justify-center text-[12px] text-slate-400">选择左侧任务进行评审</p>
        ) : (
          <>
            {/* 头部：标题 + 三道关进度 + 锁态 */}
            <div className="border-b border-slate-100 bg-white px-4 py-3">
              <div className="flex items-center gap-2">
                <h2 className="min-w-0 flex-1 truncate text-[14px] font-bold text-slate-800">{sel.title}</h2>
                {sel.status === 'awaiting_approval' ? (
                  <span className="shrink-0 rounded-full bg-amber-100 px-2 py-0.5 text-micro font-bold text-amber-700">等待审批</span>
                ) : (
                  <span className="shrink-0 rounded-full bg-slate-100 px-2 py-0.5 text-micro font-semibold text-slate-500">
                    {STATUS_LABEL[sel.status] ?? sel.status}
                  </span>
                )}
                {/* 把关台范式：驳回不=任务死亡——以此为基础复制新任务，原驳回记录保留可追溯 */}
                {sel.status === 'rejected' && (
                  <button
                    onClick={() =>
                      onCreateTask({
                        title: `${sel.title}（重提）`,
                        description: sel.description,
                        modules: sel.modules ?? [],
                        acceptance: sel.acceptance ?? '',
                        source: 'manual',
                        context: {},
                      })
                    }
                    className="flex shrink-0 items-center gap-1 rounded-md border border-slate-200 px-2 py-0.5 text-micro font-semibold text-slate-500 transition-colors hover:border-blue-300 hover:text-blue-600"
                    title="以本任务为模板创建新任务（原驳回记录保留）"
                  >
                    <Copy size={9} /> 复制为新任务
                  </button>
                )}
              </div>
              {/* 三道关：每关的决策状态（含报告锁态——报告门审批后锁定） */}
              <div className="mt-2 flex items-center gap-1">
                {GATES.map((g, i) => {
                  const d = gateDecision(g.key)
                  return (
                    <div key={g.key} className="flex flex-1 items-center gap-1">
                      <span
                        className={`flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-micro font-bold ${
                          d === 'approved' || d === 'skipped'
                            ? 'bg-emerald-500 text-white'
                            : d === 'rejected'
                              ? 'bg-red-500 text-white'
                              : i === GATES.findIndex((x) => x.key === sel.gate) && sel.status === 'awaiting_approval'
                                ? 'bg-blue-600 text-white'
                                : 'bg-slate-200 text-slate-400'
                        }`}
                      >
                        {d === 'approved' || d === 'skipped' ? <CheckCircle2 size={10} /> : d === 'rejected' ? <XCircle size={10} /> : i + 1}
                      </span>
                      <span className="flex items-center gap-0.5 text-micro text-slate-500">
                        {g.label}
                        {g.key === 'report' && (d === 'approved' || sel.status === 'done') && <Lock size={8} className="text-emerald-500" />}
                      </span>
                      {i < GATES.length - 1 && <span className={`h-px flex-1 ${d ? 'bg-emerald-400' : 'bg-slate-200'}`} />}
                    </div>
                  )
                })}
              </div>
            </div>

            <div className="flex min-h-0 flex-1">
              {/* 双栏 Diff：文件索引 ｜ diff 内容 */}
              <div className="flex min-w-0 flex-1 border-r border-slate-100">
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
                      {visibleDiffLines.map((line, i) => (
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
                      {diffLines.length > diffLimit && (
                        <button
                          onClick={() => setDiffState((s) => ({ ...s, limit: s.limit + DIFF_PAGE }))}
                          className="mt-1 w-full rounded-md border border-dashed border-slate-200 py-1 text-cap font-semibold text-slate-400 hover:border-blue-300 hover:text-blue-600"
                        >
                          还有 {diffLines.length - diffLimit} 行，点击加载更多
                        </button>
                      )}
                    </pre>
                  ) : (
                    <p className="py-8 text-center text-[11px] text-slate-400">
                      {sel.status === 'running' || sel.status === 'pending' ? '任务执行中，尚无 diff' : '该任务无变更归档'}
                    </p>
                  )}
                </div>
              </div>

              {/* 影响面 + 审批动作 */}
              <aside className="flex w-64 shrink-0 flex-col">
                <div className="border-b border-slate-100 p-3">
                  <p className="flex items-center gap-1 text-micro font-bold uppercase tracking-wider text-slate-400">
                    <GitCompareArrows size={10} /> 影响面
                  </p>
                  {impact.map((im) => (
                    <div key={im.name} className="mt-2">
                      <div className="flex h-1.5 overflow-hidden rounded-full bg-slate-100">
                        <span className="bg-emerald-500" style={{ width: `${(im.adds / Math.max(1, im.adds + im.dels)) * 100}%` }} />
                        <span className="bg-red-400" style={{ width: `${(im.dels / Math.max(1, im.adds + im.dels)) * 100}%` }} />
                      </div>
                      <p className="tnum mt-1 text-micro text-slate-500">
                        <span className="text-emerald-600">+{im.adds}</span> <span className="text-red-500">−{im.dels}</span> · {im.files} 文件
                      </p>
                    </div>
                  ))}
                  {/* 受影响模块 */}
                  <div className="mt-2 flex flex-wrap gap-1">
                    {sel && (sel as unknown as { modules?: string[] }).modules?.map((mid) => {
                      const m = map.modules.find((x) => x.id === mid)
                      return (
                        <span key={mid} className="rounded-full bg-slate-100 px-1.5 py-px text-micro text-slate-500">
                          {m?.name ?? mid}
                        </span>
                      )
                    })}
                  </div>
                </div>
                <div className="mt-auto space-y-2 p-3">
                  {sel.status === 'awaiting_approval' ? (
                    <>
                      {rejecting ? (
                        <div className="space-y-1.5">
                          <textarea
                            autoFocus
                            value={rejectNote}
                            onChange={(e) => setRejectNote(e.target.value)}
                            rows={3}
                            placeholder="驳回理由（必填）——将反馈给任务系统留痕"
                            className="w-full resize-none rounded-lg border border-red-200 bg-red-50/40 px-2.5 py-1.5 text-[12px] outline-none focus:border-red-300"
                          />
                          <div className="flex gap-2">
                            <button
                              onClick={() => decide('rejected')}
                              disabled={!!deciding || !rejectNote.trim()}
                              className="flex-1 rounded-lg bg-red-600 px-3 py-1.5 text-[12px] font-bold text-white hover:bg-red-700 disabled:opacity-40"
                            >
                              {deciding === 'rejected' ? '提交中…' : '确认驳回'}
                            </button>
                            <button
                              onClick={() => setRejecting(false)}
                              className="rounded-lg border border-slate-200 px-3 py-1.5 text-[12px] text-slate-500 hover:bg-slate-50"
                            >
                              取消
                            </button>
                          </div>
                        </div>
                      ) : (
                        <div className="flex gap-2">
                          <button
                            onClick={() => decide('approved')}
                            disabled={!!deciding}
                            className="flex flex-1 items-center justify-center gap-1 rounded-lg bg-blue-600 px-3 py-2 text-[12px] font-bold text-white hover:bg-blue-700 disabled:opacity-40"
                          >
                            {deciding === 'approved' ? <Loader2 size={12} className="animate-spin" /> : <ShieldCheck size={12} />}
                            通过
                          </button>
                          <button
                            onClick={() => setRejecting(true)}
                            disabled={!!deciding}
                            className="flex-1 rounded-lg border border-red-200 bg-white px-3 py-2 text-[12px] font-bold text-red-600 hover:bg-red-50 disabled:opacity-40"
                          >
                            驳回…
                          </button>
                        </div>
                      )}
                    </>
                  ) : (
                    <p className="rounded-lg bg-slate-50 px-3 py-2 text-center text-cap text-slate-400">
                      该任务当前无需审批（{STATUS_LABEL[sel.status] ?? sel.status}）
                    </p>
                  )}
                </div>
              </aside>
            </div>
          </>
        )}
      </div>
    </div>
  )
}
