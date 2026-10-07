import { useCallback, useEffect, useMemo, useState } from 'react'
import { Loader2, AlertTriangle, Plus, GitBranch, Pencil, Trash2, Check, X, ClipboardList, MessagesSquare } from 'lucide-react'
import { ChatPanel, type ConversationSummary } from '@/components/chat/ChatPanel'
import { StagePipeline } from '@/components/taskworkflow/StagePipeline'
import { conversations as fetchConversations, createConversation, renameConversation, deleteConversation } from '@/api/chat'
import { listTasks, taskDiff } from '@/api/task'
import { onTaskEvent } from '@/runtime/growthBus'
import { toast } from '@/runtime/toast'
import { useLang } from '@/runtime/i18n'
import type { TaskDraft } from '@/shared/logic/taskContext'
import type { CodeMap } from '@/types/map'
import type { ChatAboutTarget } from '@/shared/contract/chat'

// v0.2 定位：「任务对话」——以对话为入口把任务聊出来（孵化视角）。
// 与「任务」页（TaskPage：流程视角，看板+流水线）分工，顶部互指条显式化（方案 3.2）。
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

export function WorkbenchPage({
  backendRepo,
  map,
  onCreateTask,
  onLocateModule,
  pendingChatContext,
  onConsumeChatContext,
  initialIdea,
  onConsumeIdea,
  onNavigate,
}: {
  backendRepo: string | null
  map: CodeMap
  onCreateTask: (d: TaskDraft) => void
  onLocateModule: (id: string) => void
  /** v0.2：地图页「💬 对话/就此对话」带入的上下文（消费即清，后写覆盖先写） */
  pendingChatContext: ChatAboutTarget | null
  onConsumeChatContext: () => void
  /** 新手引导：归纳等待期保存的第一个任务想法（消费即清，自动建会话预填） */
  initialIdea?: string | null
  onConsumeIdea?: () => void
  /** v0.2：互指提示条跳「任务」页 */
  onNavigate: (page: 'tasks') => void
}) {
  const { t } = useLang()
  // 任务状态人话标签（渲染期经模块级 t 自译；枚举值为后端契约）
  const STATUS_LABEL: Record<string, string> = {
    pending: t('task.status.pending'),
    running: t('task.status.running'),
    awaiting_approval: t('task.status.awaiting'),
    done: t('task.status.done'),
    failed: t('task.status.failed'),
    rejected: t('task.status.rejected'),
  }
  const [convs, setConvs] = useState<ConversationSummary[]>([])
  const [activeConv, setActiveConv] = useState<string | null>(null)
  const [renamingId, setRenamingId] = useState<string | null>(null)
  const [renameVal, setRenameVal] = useState('')
  const [confirmDelId, setConfirmDelId] = useState<string | null>(null)
  const [tasks, setTasks] = useState<TaskItem[]>([])
  const [diffStat, setDiffStat] = useState<string | null>(null)
  const [diffTaskId, setDiffTaskId] = useState<string | null>(null)
  // v0.2：跨页预填载荷（convId+nonce 契约，ChatPanel 消费）——新建会话成功后装配
  const [chatDraft, setChatDraft] = useState<{ convId: string; text: string; mention?: { id: string; name: string }; nonce: number } | null>(null)

  // 会话列表（含运行时摘要）
  const loadConvs = useCallback(() => {
    if (!backendRepo) return
    fetchConversations(backendRepo)
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
    listTasks(backendRepo, `?conv=${encodeURIComponent(activeConv)}`)
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
    taskDiff(backendRepo, candidate.id)
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

  // v0.2 消费地图页带入的上下文：自动新建会话 → @模块芯片 + 预填文本（不自动发送——第一句是用户的权力）
  useEffect(() => {
    if (!pendingChatContext || !backendRepo) return
    const ctx = pendingChatContext
    onConsumeChatContext()
    createConversation(backendRepo, {})
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: ConversationSummary }) => {
        setActiveConv(d.data.id)
        loadConvs()
        const mention =
          ctx.kind === 'module' && map.modules.some((m) => m.id === ctx.refId)
            ? { id: ctx.refId, name: ctx.refName }
            : undefined
        setChatDraft({
          convId: d.data.id,
          text: mention
            ? t('chat.askModule')
            : t('chat.askLayer', { name: ctx.refName }),
          mention,
          nonce: Date.now(),
        })
      })
      .catch(() => toast(t('chat.ctxFail'), 'error'))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pendingChatContext])

  // 新手引导：等待期保存的任务想法 → 自动建会话预填（同样的 convId+nonce 契约）
  useEffect(() => {
    if (!initialIdea?.trim() || !backendRepo) return
    onConsumeIdea?.()
    createConversation(backendRepo, {})
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: ConversationSummary }) => {
        setActiveConv(d.data.id)
        loadConvs()
        setChatDraft({ convId: d.data.id, text: initialIdea.trim(), nonce: Date.now() })
      })
      .catch(() => toast(t('chat.ideaFail'), 'error'))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initialIdea])

  const createConv = () => {
    if (!backendRepo) return
    createConversation(backendRepo, {})
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: ConversationSummary }) => {
        setActiveConv(d.data.id)
        loadConvs()
      })
      .catch(() => toast(t('chat.createConvFail'), 'error'))
  }

  // 会话管理（重审 P0）：主入口列表的改名/删除——此前 embedded 模式全藏，误建会话无路可退
  const renameConv = async (cid: string) => {
    if (!backendRepo || !renameVal.trim()) return
    const r = await renameConversation(backendRepo, cid, renameVal.trim()).catch(() => null)
    if (!r?.ok) {
      toast(t('chat.renameConvFail'), 'error')
      return
    }
    setRenamingId(null)
    loadConvs()
  }

  const deleteConv = async (cid: string) => {
    if (!backendRepo) return
    const r = await deleteConversation(backendRepo, cid).catch(() => null)
    if (!r?.ok) {
      const d = await r?.json().catch(() => null)
      toast(d?.error ?? t('chat.deleteConvLast'), 'error')
      return
    }
    setConfirmDelId(null)
    // 删的是当前会话：切到剩余第一个（后端保底至少留一个）
    if (cid === activeConv) {
      const rest = convs.filter((c) => c.id !== cid)
      setActiveConv(rest[0]?.id ?? null)
    }
    loadConvs()
  }

  // 活跃任务（运行中/等待审批优先，否则最近一个）——五阶段进度由 StagePipeline 自判定
  const activeTask = useMemo(
    () =>
      tasks.find((t) => t.status === 'awaiting_approval') ??
      tasks.find((t) => t.status === 'running' || t.status === 'pending') ??
      tasks[0],
    [tasks],
  )

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
      const cur = byModule.get(key) ?? { name: mod?.name ?? t('chat.unmapped'), adds: 0, dels: 0, files: 0 }
      cur.adds += f.adds
      cur.dels += f.dels
      cur.files += 1
      byModule.set(key, cur)
    }
    return [...byModule.values()].sort((a, b) => b.adds + b.dels - (a.adds + a.dels))
  }, [diffStat, map])

  const maxImpact = Math.max(1, ...impact.map((i) => i.adds + i.dels))

  return (
    <div className="flex h-full flex-col">
      {/* v0.2 分工显式化：与「任务」页（流程视角）互指——用户此前反馈两页分工不明、很突兀 */}
      <div className="flex shrink-0 items-center gap-2 border-b border-slate-100 dark:border-slate-800 bg-slate-50/60 dark:bg-slate-900/60 px-3 py-1.5">
        <MessagesSquare size={12} className="shrink-0 text-slate-400 dark:text-slate-500" />
        <p className="min-w-0 flex-1 truncate text-[11px] text-slate-500 dark:text-slate-400">
          {t('chat.banner')}
        </p>
        <button
          onClick={() => onNavigate('tasks')}
          className="flex shrink-0 items-center gap-0.5 rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-0.5 text-micro font-semibold text-slate-500 dark:text-slate-400 hover:border-blue-300 hover:text-blue-600"
        >
          <ClipboardList size={10} /> {t('chat.goTasks')}
        </button>
      </div>
      <div className="flex min-h-0 flex-1">
      {/* 左栏：会话列表（三态行 + 待审批角标） */}
      <aside className="flex w-60 shrink-0 flex-col border-r border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
        <div className="flex items-center justify-between border-b border-slate-100 dark:border-slate-800 px-3 py-2.5">
          <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">{t('chat.convsTitle')}</span>
          <button
            onClick={createConv}
            disabled={!backendRepo}
            className="flex items-center gap-0.5 rounded-lg border border-slate-200 dark:border-slate-700 px-2 py-1 text-cap font-semibold text-slate-500 dark:text-slate-400 hover:border-blue-300 hover:text-blue-600 disabled:opacity-40"
          >
            <Plus size={10} /> {t('chat.createConv')}
          </button>
        </div>
        <div className="min-h-0 flex-1 space-y-0.5 overflow-y-auto p-1.5">
          {convs.map((c) => (
            <div key={c.id} className="group relative">
              <button
                onClick={() => setActiveConv(c.id)}
                className={`flex w-full items-center gap-1.5 rounded-lg px-2 py-1.5 text-left ${
                  c.id === activeConv ? 'bg-blue-50 dark:bg-blue-950/40 ring-1 ring-blue-200' : 'hover:bg-slate-50 dark:hover:bg-slate-800/70'
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
                {renamingId === c.id ? (
                  <input
                    autoFocus
                    value={renameVal}
                    onChange={(e) => setRenameVal(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') void renameConv(c.id)
                      if (e.key === 'Escape') setRenamingId(null)
                    }}
                    onClick={(e) => e.stopPropagation()}
                    className="min-w-0 flex-1 rounded border border-blue-200 dark:border-blue-900/60 bg-white dark:bg-slate-900 px-1 py-px text-[12px] outline-none"
                  />
                ) : (
                  <span className={`min-w-0 flex-1 truncate text-[12px] ${c.id === activeConv ? 'font-bold text-blue-700' : 'text-slate-600 dark:text-slate-300'}`}>
                    {c.title ?? t('chat.untitled')}
                  </span>
                )}
                {c.runtime.pendingConfirmations > 0 && (
                  <span className="tnum shrink-0 rounded-full bg-red-500 px-1.5 text-micro font-bold leading-4 text-white">
                    {c.runtime.pendingConfirmations}
                  </span>
                )}
              </button>
              {/* 重审 P0：主入口的会话管理——悬停出现改名/删除（两步确认） */}
              <span className="absolute right-1 top-1/2 hidden -translate-y-1/2 items-center gap-0.5 group-hover:flex">
                {renamingId === c.id ? (
                  <button
                    onClick={() => void renameConv(c.id)}
                    className="rounded bg-blue-600 p-0.5 text-white"
                    title={t('chat.saveName')}
                  >
                    <Check size={10} />
                  </button>
                ) : confirmDelId === c.id ? (
                  <>
                    <button
                      onClick={() => void deleteConv(c.id)}
                      className="rounded bg-red-500 px-1 py-0.5 text-[9px] font-bold text-white"
                      title={t('chat.confirmDelConvRow')}
                    >
                      {t('common.confirm')}
                    </button>
                    <button onClick={() => setConfirmDelId(null)} className="rounded p-0.5 text-slate-400 dark:text-slate-500 hover:text-slate-600" title={t('common.cancel')}>
                      <X size={10} />
                    </button>
                  </>
                ) : (
                  <>
                    <button
                      onClick={() => {
                        setRenamingId(c.id)
                        setRenameVal(c.title ?? '')
                      }}
                      className="rounded p-0.5 text-slate-300 dark:text-slate-600 hover:text-blue-500"
                      title={t('chat.rename')}
                    >
                      <Pencil size={10} />
                    </button>
                    <button
                      onClick={() => setConfirmDelId(c.id)}
                      className="rounded p-0.5 text-slate-300 dark:text-slate-600 hover:text-red-500"
                      title={t('chat.deleteConvRowTip')}
                    >
                      <Trash2 size={10} />
                    </button>
                  </>
                )}
              </span>
            </div>
          ))}
          {convs.length === 0 && <p className="px-2 py-4 text-center text-[11px] text-slate-400 dark:text-slate-500">{t('chat.noConvsRow')}</p>}
        </div>
      </aside>

      {/* 中栏：对话流（计划进度条 + 内联审批 + 对话） */}
      <div className="flex min-w-0 flex-1 flex-col">
        {activeTask && (
          <div className="border-b border-slate-100 dark:border-slate-800 bg-white dark:bg-slate-900 px-4 py-2.5">
            <div className="flex items-center justify-between">
              <p className="truncate text-[12px] font-semibold text-slate-600 dark:text-slate-300">{activeTask.title}</p>
              <span className="shrink-0 text-micro text-slate-400 dark:text-slate-500">{STATUS_LABEL[activeTask.status] ?? activeTask.status}</span>
            </div>
            {/* 计划进度条：harness 五阶段迷你管道（v4 P2 对齐——与任务页同构同判定，消除语言分裂） */}
            <div className="mt-1.5">
              <StagePipeline status={activeTask.status} gate={activeTask.gate} variant="compact" />
            </div>
          </div>
        )}
        <div className="min-h-0 flex-1 p-3">
          <ChatPanel
            backendRepo={backendRepo}
            map={map}
            embedded
            activeConvId={activeConv}
            onConvChange={setActiveConv}
            onLocateModule={onLocateModule}
            onCreateTask={onCreateTask}
            pendingDraft={chatDraft}
          />
        </div>
      </div>

      {/* 右栏：影响面（diff 按模块聚合） */}
      <aside className="flex w-64 shrink-0 flex-col border-l border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
        <div className="border-b border-slate-100 dark:border-slate-800 px-3 py-2.5">
          <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">{t('chat.impactTitle')}</span>
          <p className="mt-0.5 text-micro text-slate-400 dark:text-slate-500">{diffTaskId ? t('chat.impactFrom') : t('chat.impactIdle')}</p>
        </div>
        <div className="min-h-0 flex-1 space-y-2.5 overflow-y-auto p-3">
          {impact.length === 0 && (
            <p className="py-8 text-center text-[11px] leading-5 text-slate-400 dark:text-slate-500">
              {t('chat.impactEmpty')}
              <br />
              <span className="text-micro">{t('chat.impactEmptyHint')}</span>
            </p>
          )}
          {impact.map((im) => (
            <div key={im.name} className="rounded-lg border border-slate-100 dark:border-slate-800 p-2.5">
              <div className="flex items-center justify-between">
                <span className="truncate text-[12px] font-semibold text-slate-700 dark:text-slate-200">{im.name}</span>
                <span className="tnum shrink-0 text-micro text-slate-400 dark:text-slate-500">{t('chat.filesCount', { n: im.files })}</span>
              </div>
              <div className="mt-1.5 flex h-1.5 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800">
                <span className="bg-emerald-500" style={{ width: `${(im.adds / maxImpact) * 100}%` }} />
                <span className="bg-red-400" style={{ width: `${(im.dels / maxImpact) * 100}%` }} />
              </div>
              <p className="tnum mt-1 text-micro text-slate-500 dark:text-slate-400">
                <span className="text-emerald-600">+{im.adds}</span> <span className="text-red-500">−{im.dels}</span>
              </p>
            </div>
          ))}
          {impact.length > 0 && diffTaskId && (
            <p className="flex items-center gap-1 pt-1 text-micro text-slate-300 dark:text-slate-600">
              <GitBranch size={9} /> {t('chat.taskPrefix', { id: diffTaskId.slice(-8) })}
            </p>
          )}
        </div>
      </aside>
      </div>
    </div>
  )
}
