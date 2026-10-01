import { useCallback, useEffect, useMemo, useState } from 'react'
import { Loader2, AlertTriangle, Plus, CheckCircle2, GitBranch } from 'lucide-react'
import { ChatPanel, type ConversationSummary } from '@/components/ChatPanel'
import { onTaskEvent } from '@/lib/growthBus'
import type { TaskDraft } from '@/lib/taskContext'
import type { CodeMap } from '@/types/map'

// M4-2 开发工作台（卖点视图，按 ui-mockups/开发工作台原型.png 施工）：
// 左栏会话列表（AionUI 三态行：⚠待审批 > 🌀运行中 > 闲时）｜
// 中栏对话流（复用 ChatPanel：计划进度条=任务三道关 + 内联审批卡）｜
// 右栏影响面（该会话任务的 git diff 按模块聚合）。
// 会话 = 任务的上位容器：任务从对话升级（带 conversation_id），审批发生在会话流内。

interface TaskItem {
  id: string
  title: string
  status: string
  gate: string | null
  modules: string[]
  updatedAt?: string
}

const GATE_STEPS = [
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

export function WorkbenchPage({
  backendRepo,
  map,
  onCreateTask,
  onLocateModule,
}: {
  backendRepo: string | null
  map: CodeMap
  onCreateTask: (d: TaskDraft) => void
  onLocateModule: (id: string) => void
}) {
  const [convs, setConvs] = useState<ConversationSummary[]>([])
  const [activeConv, setActiveConv] = useState<string | null>(null)
  const [tasks, setTasks] = useState<TaskItem[]>([])
  const [diffStat, setDiffStat] = useState<string | null>(null)
  const [diffTaskId, setDiffTaskId] = useState<string | null>(null)

  // 会话列表（含运行时摘要）
  const loadConvs = useCallback(() => {
    if (!backendRepo) return
    fetch(`/api/repos/${backendRepo}/conversations`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: ConversationSummary[] } | null) => {
        if (d?.data) setConvs(d.data)
      })
      .catch(() => {})
  }, [backendRepo])

  // 该会话的任务（计划进度条 + 影响面数据源）
  const loadTasks = useCallback(() => {
    if (!backendRepo || !activeConv) {
      setTasks([])
      return
    }
    fetch(`/api/repos/${backendRepo}/tasks?conv=${encodeURIComponent(activeConv)}`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: TaskItem[] } | null) => {
        if (d?.data) setTasks(d.data)
      })
      .catch(() => {})
  }, [backendRepo, activeConv])

  // 影响面：最近一个有 diff 的任务
  useEffect(() => {
    if (!backendRepo || !tasks.length) {
      setDiffStat(null)
      setDiffTaskId(null)
      return
    }
    const candidate = tasks.find((t) => t.status === 'done') ?? tasks[0]
    fetch(`/api/repos/${backendRepo}/tasks/${encodeURIComponent(candidate.id)}/diff`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { diffStat?: string | null } } | null) => {
        setDiffStat(d?.data?.diffStat ?? null)
        setDiffTaskId(candidate.id)
      })
      .catch(() => setDiffStat(null))
  }, [backendRepo, tasks])

  useEffect(() => {
    loadConvs()
    loadTasks()
  }, [loadConvs, loadTasks])
  useEffect(() => onTaskEvent(() => { loadConvs(); loadTasks() }), [loadConvs, loadTasks])

  const createConv = () => {
    if (!backendRepo) return
    fetch(`/api/repos/${backendRepo}/conversations`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({}),
    })
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: ConversationSummary }) => {
        setActiveConv(d.data.id)
        loadConvs()
      })
      .catch(() => {})
  }

  // 计划进度条：活跃任务（运行中/等待审批优先，否则最近一个）的三道关进度
  const activeTask = useMemo(
    () =>
      tasks.find((t) => t.status === 'awaiting_approval') ??
      tasks.find((t) => t.status === 'running' || t.status === 'pending') ??
      tasks[0],
    [tasks],
  )
  const gateIndex = useMemo(() => {
    if (!activeTask) return -1
    const i = GATE_STEPS.findIndex((g) => g.key === activeTask.gate)
    if (activeTask.status === 'done') return GATE_STEPS.length
    return i
  }, [activeTask])

  // 影响面：解析 git diff --stat 文本，按模块 files glob 聚合
  const impact = useMemo(() => {
    if (!diffStat) return []
    const perFile: { path: string; adds: number; dels: number }[] = []
    for (const line of diffStat.split('\n')) {
      const m = line.match(/^\s*(.+?)\s*\|\s*\d+\s*([+-]*)\s*$/)
      if (!m) continue
      const marks = m[2] ?? ''
      perFile.push({ path: m[1].trim(), adds: (marks.match(/\+/g) ?? []).length, dels: (marks.match(/-/g) ?? []).length })
    }
    const byModule = new Map<string, { name: string; adds: number; dels: number; files: number }>()
    for (const f of perFile) {
      const mod =
        map.modules.find((mm) =>
          mm.files.some((g) => {
            const base = g.replace(/\*\*.*$/, '').replace(/\/$/, '')
            return f.path.startsWith(base) || f.path.includes('/' + base + '/')
          }),
        ) ?? null
      const key = mod?.id ?? '_other'
      const cur = byModule.get(key) ?? { name: mod?.name ?? '未映射文件', adds: 0, dels: 0, files: 0 }
      cur.adds += f.adds
      cur.dels += f.dels
      cur.files += 1
      byModule.set(key, cur)
    }
    return [...byModule.values()].sort((a, b) => b.adds + b.dels - (a.adds + a.dels))
  }, [diffStat, map])

  const maxImpact = Math.max(1, ...impact.map((i) => i.adds + i.dels))

  return (
    <div className="flex h-full">
      {/* 左栏：会话列表（三态行 + 待审批角标） */}
      <aside className="flex w-60 shrink-0 flex-col border-r border-slate-200 bg-white">
        <div className="flex items-center justify-between border-b border-slate-100 px-3 py-2.5">
          <span className="text-[12.5px] font-bold text-slate-700">会话</span>
          <button
            onClick={createConv}
            disabled={!backendRepo}
            className="flex items-center gap-0.5 rounded-lg border border-slate-200 px-2 py-1 text-[10.5px] font-semibold text-slate-500 hover:border-blue-300 hover:text-blue-600 disabled:opacity-40"
          >
            <Plus size={10} /> 新建会话
          </button>
        </div>
        <div className="min-h-0 flex-1 space-y-0.5 overflow-y-auto p-1.5">
          {convs.map((c) => (
            <button
              key={c.id}
              onClick={() => setActiveConv(c.id)}
              className={`flex w-full items-center gap-1.5 rounded-lg px-2 py-1.5 text-left ${
                c.id === activeConv ? 'bg-blue-50 ring-1 ring-blue-200' : 'hover:bg-slate-50'
              }`}
            >
              {/* AionUI 三态：⚠待审批 > 🌀运行中 > 闲时 */}
              {c.runtime.state === 'running' ? (
                <Loader2 size={12} className="shrink-0 animate-spin text-amber-500" />
              ) : c.runtime.pendingConfirmations > 0 ? (
                <AlertTriangle size={12} className="shrink-0 text-amber-500" />
              ) : (
                <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-slate-300" />
              )}
              <span className={`min-w-0 flex-1 truncate text-[12px] ${c.id === activeConv ? 'font-bold text-blue-700' : 'text-slate-600'}`}>
                {c.title ?? '未命名会话'}
              </span>
              {c.runtime.pendingConfirmations > 0 && (
                <span className="tnum shrink-0 rounded-full bg-red-500 px-1.5 text-[9px] font-bold leading-4 text-white">
                  {c.runtime.pendingConfirmations}
                </span>
              )}
            </button>
          ))}
          {convs.length === 0 && <p className="px-2 py-4 text-center text-[11px] text-slate-400">还没有会话</p>}
        </div>
      </aside>

      {/* 中栏：对话流（计划进度条 + 内联审批 + 对话） */}
      <div className="flex min-w-0 flex-1 flex-col">
        {activeTask && (
          <div className="border-b border-slate-100 bg-white px-4 py-2.5">
            <div className="flex items-center justify-between">
              <p className="truncate text-[11.5px] font-semibold text-slate-600">{activeTask.title}</p>
              <span className="shrink-0 text-[10px] text-slate-400">{STATUS_LABEL[activeTask.status] ?? activeTask.status}</span>
            </div>
            {/* 计划进度条：三道关（gate 状态由任务系统真实驱动） */}
            <div className="mt-1.5 flex items-center gap-1">
              {GATE_STEPS.map((g, i) => (
                <div key={g.key} className="flex flex-1 items-center gap-1">
                  <span
                    className={`flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-[9px] font-bold ${
                      i < gateIndex ? 'bg-emerald-500 text-white' : i === gateIndex ? 'bg-blue-600 text-white' : 'bg-slate-200 text-slate-400'
                    }`}
                  >
                    {i < gateIndex ? <CheckCircle2 size={10} /> : i + 1}
                  </span>
                  <span className={`text-[10px] ${i <= gateIndex ? 'font-semibold text-slate-600' : 'text-slate-400'}`}>{g.label}</span>
                  {i < GATE_STEPS.length - 1 && <span className={`h-px flex-1 ${i < gateIndex ? 'bg-emerald-400' : 'bg-slate-200'}`} />}
                </div>
              ))}
            </div>
          </div>
        )}
        <div className="min-h-0 flex-1 p-3">
          <ChatPanel
            backendRepo={backendRepo}
            map={map}
            embedded
            onConvChange={setActiveConv}
            onLocateModule={onLocateModule}
            onCreateTask={onCreateTask}
          />
        </div>
      </div>

      {/* 右栏：影响面（diff 按模块聚合） */}
      <aside className="flex w-64 shrink-0 flex-col border-l border-slate-200 bg-white">
        <div className="border-b border-slate-100 px-3 py-2.5">
          <span className="text-[12.5px] font-bold text-slate-700">影响面</span>
          <p className="mt-0.5 text-[9.5px] text-slate-400">{diffTaskId ? '来自该会话最近任务的变更' : '任务执行后此处显示模块影响'}</p>
        </div>
        <div className="min-h-0 flex-1 space-y-2.5 overflow-y-auto p-3">
          {impact.length === 0 && (
            <p className="py-8 text-center text-[11px] leading-5 text-slate-400">
              暂无变更数据
              <br />
              <span className="text-[9.5px]">从对话「转为任务」开始一次修复</span>
            </p>
          )}
          {impact.map((im) => (
            <div key={im.name} className="rounded-lg border border-slate-100 p-2.5">
              <div className="flex items-center justify-between">
                <span className="truncate text-[11.5px] font-semibold text-slate-700">{im.name}</span>
                <span className="tnum shrink-0 text-[10px] text-slate-400">{im.files} 文件</span>
              </div>
              <div className="mt-1.5 flex h-1.5 overflow-hidden rounded-full bg-slate-100">
                <span className="bg-emerald-500" style={{ width: `${(im.adds / maxImpact) * 100}%` }} />
                <span className="bg-red-400" style={{ width: `${(im.dels / maxImpact) * 100}%` }} />
              </div>
              <p className="tnum mt-1 text-[10px] text-slate-500">
                <span className="text-emerald-600">+{im.adds}</span> <span className="text-red-500">−{im.dels}</span>
              </p>
            </div>
          ))}
          {impact.length > 0 && diffTaskId && (
            <p className="flex items-center gap-1 pt-1 text-[9.5px] text-slate-300">
              <GitBranch size={9} /> 任务 {diffTaskId.slice(-8)}
            </p>
          )}
        </div>
      </aside>
    </div>
  )
}
