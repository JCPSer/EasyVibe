import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  AlertTriangle, ArrowDownToLine, ArrowUpFromLine, ChevronRight, Copy, GitBranch, Loader2, RefreshCw,
  ScanSearch, ShieldCheck, Sparkles, Trash2,
} from 'lucide-react'
import { toast } from '@/runtime/toast'
import { useLang } from '@/runtime/i18n'
import { absTime, aggregateByModule, moduleOfFile, parseDiffStat, relTime, toMs } from '@/shared/logic/diffStat'
import { healthColor } from '@/shared/logic/layout'
import { onPatrolFinished } from '@/runtime/growthBus'
import { gitStatus, gitLog, commitMessage, postCommit, discard, gitSync, gitCommit } from '@/api/git'
import { freshness as freshnessApi, healthHistory, patrol } from '@/api/canvas'
import { listTasks } from '@/api/task'
import { patrolRuns } from '@/api/repos'
import { DiffDrawer } from '@/components/git/DiffDrawer'
import { SourceStrip } from '@/components/git/SourceStrip'
import type { GitFile, TaskLite } from '@/components/git/types'
import type { CodeMap } from '@/types/map'

// M4-4 Git 工作树（按 ui-mockups/Git工作树原型-v3.png 施工）：
// 「提交把关台」——提交前影响面预检（模块聚合 + 健康色点 + 红线警示）、
// 改动来源归因（任务 diffStat 启发式）、提交说明 AI 生成 + EasyVibe-Task footer 留痕、
// 架构演进对照图（健康分趋势 × 提交时点）、最近提交模块 chips。
// 后端：git.rs 六端点 + git/commit-message（LLM 生成说明）。

interface GitStatus {
  branch: string
  upstream: string | null
  ahead: number
  behind: number
  files: GitFile[]
}

interface GitLogRow {
  hash: string
  short: string
  author: string
  email: string
  at: number
  subject: string
  files: string[]
}

interface HealthPoint {
  runId: string
  score: number
}

const ST_META: Record<string, { label: string; cls: string }> = {
  M: { label: 'M', cls: 'bg-amber-100 text-amber-700' },
  A: { label: 'A', cls: 'bg-emerald-100 text-emerald-700' },
  D: { label: 'D', cls: 'bg-red-100 text-red-600' },
  R: { label: 'R', cls: 'bg-indigo-100 text-indigo-700' },
  '?': { label: '?', cls: 'bg-slate-100 dark:bg-slate-800 text-slate-500 dark:text-slate-400' },
}

export function GitPage({
  backendRepo,
  map,
  onOpenChanges,
  onOpenReview,
}: {
  backendRepo: string | null
  map: CodeMap | null
  onOpenChanges: () => void
  onOpenReview: () => void
}) {
  const [status, setStatus] = useState<GitStatus | null>(null)
  const [commits, setCommits] = useState<GitLogRow[]>([])
  const [tasks, setTasks] = useState<TaskLite[]>([])
  const [freshness, setFreshness] = useState<string | null>(null)
  const [gitError, setGitError] = useState<string | null>(null)
  const [filter, setFilter] = useState('')
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const [confirmDiscard, setConfirmDiscard] = useState<string | null>(null)
  const [confirmDiscardAll, setConfirmDiscardAll] = useState(false) // 重审 P2：全部撤销两步确认
  const [message, setMessage] = useState('')
  const [footer, setFooter] = useState<string | null>(null)
  const [generating, setGenerating] = useState(false)
  const [committing, setCommitting] = useState(false)
  const [busy, setBusy] = useState<'pull' | 'push' | null>(null)
  const [patrolAfter, setPatrolAfter] = useState(true)
  const [patroling, setPatroling] = useState(false)
  // diff 抽屉：点击变更文件后展示该文件统一差异（DiffDrawer 自加载）
  const [diffFile, setDiffFile] = useState<GitFile | null>(null)
  const { t } = useLang()

  const load = useCallback(() => {
    if (!backendRepo) return
    gitStatus(backendRepo)
      .then(async (r) => {
        if (!r.ok) {
          const e = await r.json().catch(() => null)
          setGitError(e?.error ?? `HTTP ${r.status}`)
          setStatus(null)
          return null
        }
        setGitError(null)
        return r.json()
      })
      .then((d: { data?: GitStatus } | null) => setStatus(d?.data ?? null))
      .catch(() => setGitError(t('pages.git.backendUnreachable')))
    gitLog(backendRepo, 30)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: GitLogRow[] } | null) => setCommits(d?.data ?? []))
      .catch(() => {})
    freshnessApi(backendRepo)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { status?: string } } | null) => setFreshness(d?.data?.status ?? null))
      .catch(() => {})
    listTasks(backendRepo)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: TaskLite[] } | null) => setTasks((d?.data ?? []).slice(0, 15)))
      .catch(() => {})
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [backendRepo])

  useEffect(() => {
    load()
  }, [load])

  // R3 C1：提交后巡检跑完会写回 map.json——状态/历史/新鲜度全部重载（此前 toast 之后无下文）
  useEffect(
    () =>
      onPatrolFinished((evt) => {
        if (evt.repo !== backendRepo) return
        load()
      }),
    [backendRepo, load],
  )

  // 改动来源归因：任务 diffStat 文件路径匹配（启发式）+ .easyvibe/ → 巡检/归纳写回
  const taskByPath = useMemo(() => {
    const m = new Map<string, TaskLite>()
    for (const t of tasks) {
      const ds = t.result?.diffStat
      if (!ds) continue
      for (const f of parseDiffStat(ds).files) if (!m.has(f.path)) m.set(f.path, t)
    }
    return m
  }, [tasks])

  const attribOf = useCallback(
    (path: string): { kind: 'task'; task: TaskLite } | { kind: 'ev' } | { kind: 'manual' } => {
      if (path.startsWith('.easyvibe/')) return { kind: 'ev' }
      const t = taskByPath.get(path)
      return t ? { kind: 'task', task: t } : { kind: 'manual' }
    },
    [taskByPath],
  )

  const moduleList = useMemo(
    () => (map?.modules ?? []).map((m) => ({ id: m.id, name: m.name, files: m.files as string[] })),
    [map],
  )

  const groups = useMemo(() => {
    if (!status) return []
    const visible = status.files.filter((f) => !filter || f.path.includes(filter))
    const agg = aggregateByModule(
      visible.map((f) => ({ path: f.path, adds: f.adds ?? 0, dels: f.dels ?? 0 })),
      moduleList,
    )
    return agg.map((g) => ({
      ...g,
      files: visible.filter((f) => (moduleOfFile(f.path, moduleList)?.id ?? '_other') === g.id),
    }))
  }, [status, filter, moduleList])

  const redline = useMemo(
    () =>
      groups
        .filter((g) => g.id !== '_other')
        .map((g) => ({ ...g, score: map?.modules.find((m) => m.id === g.id)?.health.score ?? null }))
        .filter((g): g is typeof g & { score: number } => g.score !== null && g.score < 60),
    [groups, map],
  )

  const dominantTask = useMemo(() => {
    const count = new Map<string, number>()
    for (const f of status?.files ?? []) {
      const a = attribOf(f.path)
      if (a.kind === 'task') count.set(a.task.id, (count.get(a.task.id) ?? 0) + 1)
    }
    const top = [...count.entries()].sort((a, b) => b[1] - a[1])[0]
    return top ? (tasks.find((t) => t.id === top[0]) ?? null) : null
  }, [status, attribOf, tasks])

  const totals = useMemo(() => {
    const t = { M: 0, A: 0, D: 0, R: 0, '?': 0, adds: 0, dels: 0 }
    for (const f of status?.files ?? []) {
      t[f.status as keyof typeof t] = (t[f.status as keyof typeof t] as number) + 1
      t.adds += f.adds ?? 0
      t.dels += f.dels ?? 0
    }
    return t
  }, [status])

  const startPatrol = async () => {
    if (!backendRepo || patroling) return
    setPatroling(true)
    try {
      await patrol(backendRepo)
      toast(t('pages.git.toastPatrolStarted'))
    } catch {
      toast(t('pages.git.toastPatrolFailed'), 'error')
    } finally {
      setPatroling(false)
    }
  }

  const generateMessage = async () => {
    if (!backendRepo || generating) return
    setGenerating(true)
    try {
      const affected = groups.filter((g) => g.id !== '_other').map((g) => g.name)
      const diffStat = status?.files
        .map((f) => `${f.path} | ${(f.adds ?? 0) + (f.dels ?? 0)} ${'+'.repeat(f.adds ?? 0)}${'-'.repeat(f.dels ?? 0)}`)
        .join('\n')
      const r = await commitMessage(backendRepo, { task_id: dominantTask?.id ?? null, modules: affected, diff_stat: diffStat ?? '' })
      const d = await r.json().catch(() => null)
      if (!r.ok || !d?.data?.message) {
        toast(d?.error ?? t('pages.git.toastGenFailed'), 'error')
        return
      }
      setMessage(d.data.message)
      setFooter(d.data.footer ?? null)
    } catch {
      toast(t('pages.git.toastGenFailed'), 'error')
    } finally {
      setGenerating(false)
    }
  }

  const doCommit = async () => {
    if (!backendRepo || committing || !message.trim() || !status || status.files.length === 0) return
    setCommitting(true)
    try {
      const full = footer ? `${message.trim()}\n\n${footer}` : message.trim()
      const r = await postCommit(backendRepo, { message: full })
      const d = await r.json().catch(() => null)
      if (!r.ok) {
        toast(d?.error ?? t('pages.git.toastCommitFailed'), 'error')
        return
      }
      toast(t('pages.git.toastCommitted', { hash: d?.data?.shortHash ?? '', patrolPart: patrolAfter ? t('pages.git.toastPatrolQueued') : '' }))
      setMessage('')
      setFooter(null)
      load()
      if (patrolAfter) await patrol(backendRepo).catch(() => {})
    } finally {
      setCommitting(false)
    }
  }

  const doDiscard = async (path: string) => {
    if (!backendRepo) return
    try {
      const r = await discard(backendRepo, { path })
      const d = await r.json().catch(() => null)
      if (!r.ok) toast(d?.error ?? t('pages.git.toastDiscardFailed'), 'error')
      else toast(path === '*' ? t('pages.git.toastDiscardAll') : t('pages.git.toastDiscard', { path }))
    } finally {
      setConfirmDiscard(null)
      setConfirmDiscardAll(false)
      // 撤销的文件若正开着 diff 抽屉，一并关闭（内容已不存在）
      setDiffFile((cur) => (cur && (path === '*' || cur.path === path) ? null : cur))
      load()
    }
  }

  const sync = async (kind: 'pull' | 'push') => {
    if (!backendRepo || busy) return
    setBusy(kind)
    try {
      const r = await gitSync(backendRepo, kind)
      const d = await r.json().catch(() => null)
      if (!r.ok) toast(d?.error ?? t(kind === 'pull' ? 'pages.git.toastPullFailed' : 'pages.git.toastPushFailed'), 'error')
      else toast(kind === 'pull' ? t('pages.git.toastPulled') : t('pages.git.toastPushed'))
    } finally {
      setBusy(null)
      load()
    }
  }

  if (!backendRepo) {
    return <div className="flex h-full items-center justify-center text-[12px] text-slate-400 dark:text-slate-500">{t('common.pickProject')}</div>
  }

  const freshMeta =
    freshness === 'fresh' ? { color: '#10b981', labelKey: 'pages.git.freshness.fresh' } :
    freshness === 'drifting' ? { color: '#f59e0b', labelKey: 'pages.git.freshness.drifting' } :
    freshness === 'stale' ? { color: '#ef4444', labelKey: 'pages.git.freshness.stale' } :
    { color: '#94a3b8', labelKey: 'pages.git.freshness.unknown' }

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="min-h-0 flex-1 overflow-y-auto p-5">
        <div className="mb-3 flex items-baseline gap-3">
          <h2 className="text-[15px] font-bold text-slate-800 dark:text-slate-100">{t('pages.git.title')}</h2>
          <p className="text-[11px] text-slate-400 dark:text-slate-500">{t('pages.git.subtitle')}</p>
        </div>

        {/* 状态条 */}
        <div className="mb-3 flex items-center gap-3 rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-4 py-2.5">
          <span className="flex items-center gap-1.5 rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-950/70 px-2.5 py-1 text-[12px] font-bold text-slate-800 dark:text-slate-100">
            <GitBranch size={12} className="text-slate-400 dark:text-slate-500" />
            {status?.branch ?? '—'}
          </span>
          {status?.upstream && (
            <span className="tnum flex items-center gap-1.5 text-[12px]">
              <span className="flex items-center gap-0.5 font-bold text-emerald-500"><ArrowUpFromLine size={11} />{status.ahead}</span>
              <span className="flex items-center gap-0.5 font-bold text-amber-500"><ArrowDownToLine size={11} />{status.behind}</span>
              <span className="text-cap text-slate-300 dark:text-slate-600">{t('pages.git.upstream', { name: status.upstream })}</span>
            </span>
          )}
          <span className="flex gap-1.5">
            <span className="tnum rounded-full bg-slate-100 dark:bg-slate-800 px-2 py-0.5 text-cap font-semibold text-slate-500 dark:text-slate-400"><b className="text-amber-600">{totals.M}</b> {t('pages.git.statModified')}</span>
            <span className="tnum rounded-full bg-slate-100 dark:bg-slate-800 px-2 py-0.5 text-cap font-semibold text-slate-500 dark:text-slate-400"><b className="text-emerald-600">{totals.A + totals['?']}</b> {t('pages.git.statAdded')}</span>
            <span className="tnum rounded-full bg-slate-100 dark:bg-slate-800 px-2 py-0.5 text-cap font-semibold text-slate-500 dark:text-slate-400"><b className="text-red-500">{totals.D}</b> {t('pages.git.statDeleted')}</span>
            <span className="tnum rounded-full bg-blue-50 dark:bg-blue-950/40 px-2 py-0.5 text-cap font-semibold text-blue-600">{t('pages.git.statModules', { count: groups.filter((g) => g.id !== '_other').length })}</span>
          </span>
          <span className="flex items-center gap-1.5 rounded-full border border-slate-100 dark:border-slate-800 px-2 py-0.5 text-cap font-semibold" style={{ color: freshMeta.color }}>
            <i className="h-1.5 w-1.5 rounded-full" style={{ backgroundColor: freshMeta.color }} />
            {t(freshMeta.labelKey)}
          </span>
          <span className="ml-auto flex gap-2">
            <button
              onClick={() => sync('pull')}
              disabled={!!busy}
              className="rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-3 py-1.5 text-[11px] font-semibold text-slate-600 dark:text-slate-300 hover:bg-slate-50 dark:hover:bg-slate-800/70 disabled:opacity-40"
            >
              {busy === 'pull' ? <Loader2 size={11} className="animate-spin" /> : t('pages.git.pull', { count: status?.behind ?? 0 })}
            </button>
            <button
              onClick={() => sync('push')}
              disabled={!!busy}
              className="rounded-lg bg-blue-600 px-3 py-1.5 text-[11px] font-semibold text-white hover:bg-blue-700 disabled:opacity-40"
            >
              {busy === 'push' ? <Loader2 size={11} className="animate-spin" /> : t('pages.git.push', { count: status?.ahead ?? 0 })}
            </button>
          </span>
        </div>

        {gitError && (
          <div className="mb-3 flex items-center gap-2 rounded-xl border border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-4 py-3 text-[12px] text-amber-700">
            <AlertTriangle size={14} className="shrink-0" />
            {t('pages.git.error', { msg: gitError })}
          </div>
        )}

        <div className="grid grid-cols-5 gap-3">
          {/* 左列：未提交变更 · 提交前预检 */}
          <div className="col-span-3 overflow-hidden rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
            <div className="flex items-center gap-2 border-b border-slate-100 dark:border-slate-800 px-4 py-3">
              <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">{t('pages.git.sectionChanges')}</span>
              <span className="text-micro text-slate-300 dark:text-slate-600">{t('pages.git.sectionHint')}</span>
              <div className="ml-auto flex items-center gap-1.5">
                <input
                  value={filter}
                  onChange={(e) => setFilter(e.target.value)}
                  placeholder={t('pages.git.filterPlaceholder')}
                  className="w-32 rounded-lg border border-slate-200 dark:border-slate-700 px-2 py-1 text-cap text-slate-600 dark:text-slate-300 focus:border-blue-300 focus:outline-none"
                />
                {/* 重审 P2：全部撤销（此前误改一堆只能逐个点）——两步确认，红色警示语义 */}
                {status && status.files.length > 0 &&
                  (confirmDiscardAll ? (
                    <span className="flex items-center gap-1">
                      <span className="text-[10px] font-bold text-red-500">{t('pages.git.discardAllConfirm', { count: status.files.length })}</span>
                      <button
                        onClick={() => void doDiscard('*')}
                        onMouseLeave={() => setConfirmDiscardAll(false)}
                        className="rounded bg-red-500 px-1.5 py-1 text-micro font-bold text-white hover:bg-red-600"
                      >
                        {t('pages.git.discardAllOk')}
                      </button>
                    </span>
                  ) : (
                    <button
                      onClick={() => setConfirmDiscardAll(true)}
                      className="flex items-center gap-0.5 rounded-lg border border-red-200 dark:border-red-900/60 px-2 py-1 text-cap font-semibold text-red-500 hover:bg-red-50 dark:hover:bg-red-950/40"
                      title={t('pages.git.discardAllTip')}
                    >
                      <Trash2 size={10} /> {t('pages.git.discardAll')}
                    </button>
                  ))}
                <button
                  onClick={load}
                  className="rounded-lg border border-slate-200 dark:border-slate-700 p-1.5 text-slate-400 dark:text-slate-500 hover:bg-slate-50 dark:hover:bg-slate-800/70 hover:text-slate-600"
                  title={t('pages.git.refreshTip')}
                >
                  <RefreshCw size={11} />
                </button>
              </div>
            </div>

            {/* 改动来源 */}
            {status && status.files.length > 0 && (
              <SourceStrip files={status.files} attribOf={attribOf} onOpenChanges={onOpenChanges} />
            )}

            {/* 红线警示 */}
            {redline.length > 0 && (
              <div className="mx-4 mt-3 flex items-center gap-3 rounded-xl border border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-3.5 py-2.5">
                <AlertTriangle size={15} className="shrink-0 text-amber-500" />
                <p className="min-w-0 flex-1 text-[11px] leading-4 text-amber-700">
                  <b>{t('pages.git.redlineTitle')}</b>
                  <span className="text-amber-500">{t('pages.git.redlineBody', {
                    items: redline.map((g) => t('pages.git.redlineScore', { name: g.name, score: g.score })).join('、'),
                    adds: totals.adds,
                    dels: totals.dels,
                  })}</span>
                </p>
                <button
                  onClick={startPatrol}
                  disabled={patroling}
                  className="flex shrink-0 items-center gap-1 rounded-lg border border-amber-300 dark:border-amber-800 bg-white dark:bg-slate-900 px-2.5 py-1 text-cap font-semibold text-amber-600 hover:bg-amber-100 dark:hover:bg-amber-900/40 disabled:opacity-40"
                >
                  {patroling ? <Loader2 size={10} className="animate-spin" /> : <ScanSearch size={10} />} {t('pages.git.patrol')}
                </button>
                <button
                  onClick={onOpenReview}
                  className="flex shrink-0 items-center gap-1 rounded-lg border border-amber-300 dark:border-amber-800 bg-white dark:bg-slate-900 px-2.5 py-1 text-cap font-semibold text-amber-600 hover:bg-amber-100 dark:hover:bg-amber-900/40"
                >
                  <ShieldCheck size={10} /> {t('pages.git.review')}
                </button>
              </div>
            )}

            {/* 模块分组 */}
            <div className="py-2">
              {groups.map((g) => {
                const score = g.id === '_other' ? null : (map?.modules.find((m) => m.id === g.id)?.health.score ?? null)
                const isCollapsed = collapsed.has(g.id)
                return (
                  <div key={g.id} className="border-b border-slate-50 last:border-0">
                    <button
                      onClick={() =>
                        setCollapsed((cs) => {
                          const n = new Set(cs)
                          if (n.has(g.id)) n.delete(g.id)
                          else n.add(g.id)
                          return n
                        })
                      }
                      className="flex w-full items-center gap-2 bg-slate-50/50 dark:bg-slate-900/50 px-4 py-2.5 text-left hover:bg-slate-50 dark:hover:bg-slate-800/70"
                    >
                      {score !== null && <span className="h-2 w-2 rounded-full" style={{ backgroundColor: healthColor(score) }} />}
                      <span className="text-[12px] font-bold text-slate-700 dark:text-slate-200">{g.name}</span>
                      <span className="text-micro text-slate-300 dark:text-slate-600">{t('pages.git.filesCount', { count: g.files.length })}</span>
                      {score !== null && (
                        <span className="tnum text-cap font-bold" style={{ color: healthColor(score) }}>{score}</span>
                      )}
                      <span className="tnum ml-auto text-cap font-bold text-emerald-600">+{g.adds}</span>
                      <span className="tnum text-cap font-bold text-red-500">−{g.dels}</span>
                      <span className="text-micro text-slate-300 dark:text-slate-600">{isCollapsed ? '▸' : '▾'}</span>
                    </button>
                    {!isCollapsed &&
                      g.files.map((f) => {
                        const a = attribOf(f.path)
                        const st = ST_META[f.status] ?? ST_META['?']
                        return (
                          <div
                            key={f.path}
                            onClick={() => setDiffFile(f)}
                            title={t('git.diff.viewTip')}
                            className="group flex cursor-pointer items-center gap-2.5 py-[7px] pl-8 pr-4 hover:bg-slate-50/60 dark:bg-slate-900/60"
                          >
                            <span className={`flex h-[17px] w-[17px] shrink-0 items-center justify-center rounded-[5px] text-micro font-extrabold ${st.cls}`}>
                              {st.label}
                            </span>
                            <span className="mono min-w-0 flex-1 truncate text-cap text-slate-600 dark:text-slate-300">
                              {f.orig ? `${f.orig} → ${f.path}` : f.path}
                            </span>
                            {a.kind === 'task' && (
                              <span className="shrink-0 rounded-full bg-violet-50 px-1.5 py-px text-micro font-semibold text-violet-600">
                                {t('pages.git.taskPrefix', { title: a.task.title.slice(0, 8) })}
                              </span>
                            )}
                            {a.kind === 'ev' && (
                              <span className="shrink-0 rounded-full bg-slate-100 dark:bg-slate-800 px-1.5 py-px text-micro font-semibold text-slate-500 dark:text-slate-400">{t('pages.git.attributionPatrol')}</span>
                            )}
                            <span className="tnum w-[72px] shrink-0 text-right text-micro font-bold">
                              <i className="not-italic text-emerald-600">+{f.adds ?? 0}</i>{' '}
                              <i className="not-italic text-red-400">−{f.dels ?? 0}</i>
                            </span>
                            {confirmDiscard === f.path ? (
                              <button
                                onClick={(e) => {
                                  e.stopPropagation()
                                  doDiscard(f.path)
                                }}
                                onMouseLeave={() => setConfirmDiscard(null)}
                                className="tnum shrink-0 rounded bg-red-500 px-1.5 py-0.5 text-micro font-bold text-white"
                              >
                                {t('pages.git.discardConfirm')}
                              </button>
                            ) : (
                              <button
                                onClick={(e) => {
                                  e.stopPropagation()
                                  setConfirmDiscard(f.path)
                                }}
                                className="shrink-0 rounded p-0.5 text-slate-200 hover:bg-red-50 dark:hover:bg-red-950/40 hover:text-red-500 group-hover:text-slate-300"
                                title={t('pages.git.discardTip')}
                              >
                                <Trash2 size={11} />
                              </button>
                            )}
                          </div>
                        )
                      })}
                  </div>
                )
              })}
              {(!status || status.files.length === 0) && !gitError && (
                <p className="py-10 text-center text-[12px] text-slate-300 dark:text-slate-600">{t('pages.git.clean')}</p>
              )}
            </div>

            <div className="flex gap-4 border-t border-slate-100 dark:border-slate-800 px-4 py-2.5 text-cap text-slate-400 dark:text-slate-500">
              <span>{t('pages.git.footerModules', { count: groups.filter((g) => g.id !== '_other').length })} · <b className="tnum text-slate-600 dark:text-slate-300">+{totals.adds} −{totals.dels}</b></span>
              <span>{t('pages.git.footerUnmapped', { count: groups.find((g) => g.id === '_other')?.files.length ?? 0 })}</span>
              <span className="ml-auto">{t('pages.git.baseline', { base: 'HEAD' })}</span>
            </div>

            {/* 提交框 */}
            <div className="border-t border-slate-100 dark:border-slate-800">
              <div className="flex items-center gap-2 px-4 pt-2.5">
                <span className="text-[11px] font-bold text-slate-500 dark:text-slate-400">{t('pages.git.commitLabel')}</span>
                <button
                  onClick={generateMessage}
                  disabled={generating || !status || status.files.length === 0}
                  className="flex items-center gap-1 rounded-full border border-blue-200 dark:border-blue-900/60 bg-blue-50 dark:bg-blue-950/40 px-2 py-0.5 text-micro font-semibold text-blue-600 hover:bg-blue-100 disabled:opacity-40"
                >
                  {generating ? <Loader2 size={10} className="animate-spin" /> : <Sparkles size={10} />}
                  {generating ? t('pages.git.commitGenerating') : dominantTask ? t('pages.git.commitGenerateTask') : t('pages.git.commitGenerate')}
                </button>
                {dominantTask && (
                  <span className="text-micro text-slate-300 dark:text-slate-600">{t('pages.git.commitLinked')}</span>
                )}
              </div>
              <textarea
                value={message}
                onChange={(e) => setMessage(e.target.value)}
                placeholder={t('pages.git.commitPlaceholder')}
                className="h-14 w-full resize-none px-4 pt-2 text-[12px] text-slate-700 dark:text-slate-200 outline-none"
              />
              <div className="flex items-center gap-3 border-t border-slate-50 bg-slate-50/40 dark:bg-slate-900/40 px-4 py-2.5">
                <span className="text-cap text-slate-400 dark:text-slate-500">
                  {t('pages.git.commitSummary', { count: status?.files.length ?? 0 })}{' '}
                  <b className="text-blue-600">{groups.filter((g) => g.id !== '_other').slice(0, 3).map((g) => g.name).join('、') || '—'}</b>
                </span>
                <label className="ml-auto flex items-center gap-1.5 text-micro text-slate-400 dark:text-slate-500">
                  <input type="checkbox" checked={patrolAfter} onChange={(e) => setPatrolAfter(e.target.checked)} className="accent-blue-600" />
                  {t('pages.git.commitPatrolAfter')}
                </label>
                {redline.length > 0 && (
                  <span className="flex items-center gap-1 rounded-full bg-amber-50 dark:bg-amber-950/40 px-2 py-0.5 text-micro font-semibold text-amber-600">
                    <AlertTriangle size={9} /> {t('pages.git.commitRedlineWarn')}
                  </span>
                )}
                <button
                  onClick={doCommit}
                  disabled={committing || !message.trim() || !status || status.files.length === 0}
                  className="rounded-lg bg-blue-600 px-4 py-1.5 text-[11px] font-bold text-white hover:bg-blue-700 disabled:opacity-40"
                >
                  {committing ? <Loader2 size={11} className="animate-spin" /> : t('pages.git.commitButton', { branch: status?.branch ?? '—' })}
                </button>
              </div>
            </div>
          </div>

          {/* 右列 */}
          <div className="col-span-2 flex flex-col gap-3">
            <EvolutionChart backendRepo={backendRepo} groups={groups} map={map} commits={commits} />
            <CommitHistory commits={commits} moduleList={moduleList} backendRepo={backendRepo} />
          </div>
        </div>
      </div>

      {/* diff 抽屉：点击变更文件滑出（Esc / ✕ / 点遮罩关闭） */}
      {diffFile && backendRepo && (
        <DiffDrawer backendRepo={backendRepo} file={diffFile} onClose={() => setDiffFile(null)} />
      )}
    </div>
  )
}

/** 架构演进对照：受影响模块健康分趋势 × 提交时点竖线（纯前端组合，零后端增量） */
function EvolutionChart({
  backendRepo,
  groups,
  map,
  commits,
}: {
  backendRepo: string
  groups: { id: string; name: string }[]
  map: CodeMap | null
  commits: GitLogRow[]
}) {
  const { t } = useLang()
  const [series, setSeries] = useState<{ id: string; name: string; color: string; points: { t: number; s: number }[] }[]>([])
  const affected = useMemo(() => groups.filter((g) => g.id !== '_other').slice(0, 2), [groups])

  useEffect(() => {
    let alive = true
    const loadSeries = async () => {
      const runsRes = await patrolRuns(backendRepo)
      const runs: { data?: { id: string; startedAt: string }[] } | null = runsRes.ok ? await runsRes.json() : null
      const timeByRun = new Map((runs?.data ?? []).map((r) => [r.id, toMs(r.startedAt) ?? 0]))
      const out: typeof series = []
      for (const g of affected) {
        const res = await healthHistory(backendRepo, g.id)
        const d: { data?: HealthPoint[] } | null = res.ok ? await res.json() : null
        const points = (d?.data ?? [])
          .map((h) => ({ t: timeByRun.get(h.runId) ?? 0, s: h.score }))
          .filter((p) => p.t > 0)
          .sort((a, b) => a.t - b.t)
        const score = map?.modules.find((m) => m.id === g.id)?.health.score ?? 60
        out.push({ id: g.id, name: g.name, color: healthColor(score), points })
      }
      if (alive) setSeries(out.filter((s) => s.points.length > 0))
    }
    loadSeries().catch(() => {})
    return () => {
      alive = false
    }
  }, [backendRepo, affected, map])

  const W = 460
  const H = 170
  const PAD = { l: 26, r: 8, t: 10, b: 18 }
  const [now] = useState(() => Date.now()) // 渲染期纯度：挂载时锚定一次"当前时间"
  const allT = series.flatMap((s) => s.points.map((p) => p.t))
  const commitMarks = commits.map((c) => c.at * 1000).filter((t) => allT.length > 0 && t >= Math.min(...allT) && t <= now)
  const minT = allT.length > 0 ? Math.min(...allT, ...commitMarks) : 0
  const maxT = Math.max(now, ...allT)

  return (
    <div className="overflow-hidden rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
      <div className="flex items-center gap-2 border-b border-slate-100 dark:border-slate-800 px-4 py-3">
        <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">{t('pages.git.chartTitle')}</span>
        <span className="text-micro text-slate-300 dark:text-slate-600">{t('pages.git.chartHint')}</span>
      </div>
      {series.length > 0 && allT.length > 0 ? (
        <div className="px-3 pb-1 pt-2">
          <svg viewBox={`0 0 ${W} ${H}`} className="w-full">
            {[0, 60, 100].map((g) => {
              const y = PAD.t + (1 - g / 100) * (H - PAD.t - PAD.b)
              return (
                <g key={g}>
                  <line x1={PAD.l} x2={W - PAD.r} y1={y} y2={y} stroke={g === 60 ? '#fca5a5' : '#f1f5f9'} strokeWidth={1} strokeDasharray={g === 60 ? '4 3' : undefined} />
                  <text x={PAD.l - 4} y={y + 3} textAnchor="end" fontSize={8} fill={g === 60 ? '#f87171' : '#cbd5e1'} className="tnum">{g === 60 ? t('pages.git.redline60') : g}</text>
                </g>
              )
            })}
            {commitMarks.map((t, i) => {
              const x = PAD.l + ((t - minT) / Math.max(1, maxT - minT)) * (W - PAD.l - PAD.r)
              return <line key={i} x1={x} x2={x} y1={PAD.t} y2={H - PAD.b} stroke="#93c5fd" strokeWidth={1} strokeDasharray="2 3" />
            })}
            {series.map((s) => (
              <path
                key={s.id}
                d={s.points.map((p, i) => `${i === 0 ? 'M' : 'L'}${(PAD.l + ((p.t - minT) / Math.max(1, maxT - minT)) * (W - PAD.l - PAD.r)).toFixed(1)},${(PAD.t + (1 - p.s / 100) * (H - PAD.t - PAD.b)).toFixed(1)}`).join(' ')}
                fill="none"
                stroke={s.color}
                strokeWidth={1.8}
              />
            ))}
            {series[0]?.points[0] && (
              <text x={PAD.l + 2} y={H - 4} fontSize={8} fill="#cbd5e1" className="tnum">
                {absTime(new Date(minT).toISOString()).slice(5, 10)} → {absTime(new Date(maxT).toISOString()).slice(5, 10)}
              </text>
            )}
          </svg>
          <div className="flex items-center gap-3 px-2 pb-2 pt-0.5">
            {series.map((s) => (
              <span key={s.id} className="flex items-center gap-1 text-micro font-semibold" style={{ color: s.color }}>
                <i className="h-[3px] w-3 rounded-full" style={{ backgroundColor: s.color }} />
                {s.name}
              </span>
            ))}
            <span className="ml-auto flex items-center gap-1 text-micro text-slate-300 dark:text-slate-600">{t('pages.git.chartLegend')}</span>
          </div>
        </div>
      ) : (
        <p className="py-8 text-center text-cap text-slate-300 dark:text-slate-600">{t('pages.git.chartEmpty')}</p>
      )}
    </div>
  )
}

/** 最近提交：模块 chips（log --name-only 文件映射）+ 作者头像 + hash 复制。
 *  用户反馈补交互：点击行展开提交详情（正文 + 逐文件增删，git show --numstat） */
interface CommitDetail {
  hash: string
  subject: string
  body: string
  files: { path: string; adds: number; dels: number }[]
}

function CommitHistory({ commits, moduleList, backendRepo }: { commits: GitLogRow[]; moduleList: { id: string; name: string; files: string[] }[]; backendRepo: string | null }) {
  const { t } = useLang()
  const [q, setQ] = useState('')
  const [expanded, setExpanded] = useState<string | null>(null)
  const [detail, setDetail] = useState<CommitDetail | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const rows = useMemo(() => {
    const list = q
      ? commits.filter((c) => c.subject.includes(q) || c.author.includes(q) || c.short.includes(q))
      : commits
    return list.slice(0, 8)
  }, [commits, q])

  const toggle = (hash: string) => {
    if (expanded === hash) {
      setExpanded(null)
      return
    }
    setExpanded(hash)
    setDetail(null)
    if (!backendRepo) return
    setDetailLoading(true)
    gitCommit(backendRepo, hash)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data?: CommitDetail }) => setDetail(d.data ?? null))
      .catch(() => toast(t('pages.git.toastDetailFailed'), 'error'))
      .finally(() => setDetailLoading(false))
  }

  const modsOf = (files: string[]) => {
    const names = new Map<string, string>()
    for (const f of files) {
      const m = moduleOfFile(f, moduleList)
      if (m) names.set(m.id, m.name)
    }
    return [...names.values()]
  }

  const AV_COLORS = ['#2563eb', '#059669', '#d97706', '#dc2626', '#7c3aed', '#0891b2']
  const avatarColor = (s: string) => AV_COLORS[[...s].reduce((a, c) => a + c.charCodeAt(0), 0) % AV_COLORS.length]

  return (
    <div className="flex-1 overflow-hidden rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
      <div className="flex items-center gap-2 border-b border-slate-100 dark:border-slate-800 px-4 py-3">
        <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">{t('pages.git.commitsTitle')}</span>
        <span className="text-micro text-slate-300 dark:text-slate-600">{t('pages.git.commitsHint')}</span>
      </div>
      <div className="px-4 pt-2.5">
        <input
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder={t('pages.git.searchPlaceholder')}
          className="w-full rounded-lg border border-slate-200 dark:border-slate-700 px-2.5 py-1.5 text-cap text-slate-600 dark:text-slate-300 focus:border-blue-300 focus:outline-none"
        />
      </div>
      <div className="py-1.5">
        {rows.map((c) => {
          const mods = modsOf(c.files)
          const open = expanded === c.hash
          return (
            <div key={c.hash} className={`px-4 py-2.5 ${open ? 'bg-blue-50/40' : 'hover:bg-slate-50/60 dark:bg-slate-900/60'}`}>
              {/* 整行可点：展开提交详情（此前纯展示，用户反馈） */}
              <button onClick={() => toggle(c.hash)} className="flex w-full items-start gap-2 text-left" title={open ? t('pages.git.collapseTip') : t('pages.git.expandTip')}>
                <div className="min-w-0 flex-1">
                  <p className="truncate text-[12px] font-bold text-slate-700 dark:text-slate-200">{c.subject}</p>
                  <p className="mt-0.5 flex items-center gap-1.5 text-micro text-slate-400 dark:text-slate-500">
                    <span className="mono rounded bg-slate-100 dark:bg-slate-800 px-1 py-px text-micro text-slate-500 dark:text-slate-400">{c.short}</span>
                    <span className="truncate">{c.author}</span>
                    <span className="shrink-0">{relTime(new Date(c.at * 1000).toISOString(), Date.now(), t)}</span>
                  </p>
                  {mods.length > 0 && (
                    <div className="mt-1 flex flex-wrap gap-1">
                      {mods.slice(0, 3).map((m) => (
                        <span key={m} className="rounded-full bg-slate-100 dark:bg-slate-800 px-1.5 py-px text-micro font-semibold text-slate-500 dark:text-slate-400">{m}</span>
                      ))}
                      {mods.length > 3 && <span className="rounded-full border border-dashed border-slate-200 dark:border-slate-700 px-1.5 py-px text-micro text-slate-300 dark:text-slate-600">+{mods.length - 3}</span>}
                    </div>
                  )}
                </div>
                <div className="flex shrink-0 flex-col items-end gap-1">
                  <span
                    className="flex h-6 w-6 items-center justify-center rounded-full text-micro font-extrabold text-white"
                    style={{ backgroundColor: avatarColor(c.author) }}
                  >
                    {c.author.slice(0, 2).toUpperCase()}
                  </span>
                </div>
                <span className={`mt-1 shrink-0 self-start text-slate-300 dark:text-slate-600 transition-transform ${open ? 'rotate-90' : ''}`}>
                  <ChevronRight size={12} />
                </span>
              </button>
              {open && (
                <div className="ml-1 mt-2 border-l-2 border-blue-100 pl-3">
                  {detailLoading && <p className="py-1 text-micro text-slate-400 dark:text-slate-500">{t('pages.git.loadingDetail')}</p>}
                  {!detailLoading && detail && (
                    <>
                      {detail.body && <p className="mb-1.5 whitespace-pre-wrap text-[11px] leading-4 text-slate-500 dark:text-slate-400">{detail.body}</p>}
                      {detail.files.length > 0 ? (
                        <div className="space-y-0.5">
                          {detail.files.slice(0, 12).map((f) => (
                            <div key={f.path} className="flex items-center gap-2 text-micro">
                              <span className="min-w-0 flex-1 truncate font-mono text-slate-600 dark:text-slate-300">{f.path}</span>
                              <span className="tnum shrink-0 font-semibold text-emerald-600">+{f.adds}</span>
                              <span className="tnum shrink-0 font-semibold text-red-400">−{f.dels}</span>
                            </div>
                          ))}
                          {detail.files.length > 12 && <p className="text-micro text-slate-300 dark:text-slate-600">{t('pages.git.moreFiles', { count: detail.files.length })}</p>}
                        </div>
                      ) : (
                        <p className="text-micro text-slate-400 dark:text-slate-500">{t('pages.git.noFiles')}</p>
                      )}
                    </>
                  )}
                  <div className="mt-1.5 flex items-center gap-2">
                    <button
                      onClick={() => void navigator.clipboard?.writeText(c.hash)}
                      className="flex items-center gap-0.5 rounded px-1 py-0.5 text-micro text-slate-400 dark:text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-600"
                      title={t('pages.git.copyTip')}
                    >
                      <Copy size={9} /> {t('pages.git.copy')}
                    </button>
                  </div>
                </div>
              )}
            </div>
          )
        })}
        {rows.length === 0 && <p className="py-8 text-center text-cap text-slate-300 dark:text-slate-600">{t('pages.git.commitsEmpty')}</p>}
      </div>
      <div className="flex border-t border-slate-100 dark:border-slate-800 px-4 py-2 text-micro text-slate-400 dark:text-slate-500">
        <span>{t('pages.git.commitsTotal', { count: commits.length })}</span>
      </div>
    </div>
  )
}
