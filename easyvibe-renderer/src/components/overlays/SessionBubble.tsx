import { useCallback, useEffect, useReducer, useRef, useState } from 'react'
import { Activity, Clock, HeartPulse, Search, Sparkles, X } from 'lucide-react'
import { onQueueChanged, onSessionEvent, onTaskEvent } from '@/runtime/growthBus'
import { toast } from '@/runtime/toast'
import { toMs } from '@/shared/logic/diffStat'
import { formatElapsed, kindFromLabel } from '@/runtime/sessionQueue'
import { useLang } from '@/lib/i18n'
import { SessionBubbleCard } from './SessionBubbleCard'
import { sessionsOverview } from '@/api/repos'
import { listTasks } from '@/api/task'
import { killSession, cancelSessionQueue } from '@/api/system'

type QueueKind = 'patrol' | 'reinduce' | 'submap'

// 全局运行指示器（2026-10-05 跨仓库视角）：
// 单仓库互斥、跨仓库并行合法（用户明示接受并发 agent）——A 仓库分析中切到 B，
// A 的会话必须仍全局可见。数据源 GET /sessions/overview（全仓库活动会话 + 全队列）。
// 顶栏正中央细条状态丸：1 个活动 → 常规展示（非当前仓库的附仓库名）；多个 → 聚合计数；
// 排队 chip 同理跨仓库。悬停卡列出全部活动会话（含仓库），可分别终止。

interface OverviewActive {
  sessionId: string
  repo: string
  label: string
  status: string
  startedAt?: string | null
}

interface OverviewQueued {
  repo: string
  kind: string
  label: string
  enqueuedAt?: string | null
}

interface Overview {
  active: OverviewActive[]
  queued: OverviewQueued[]
}

export function SessionBubble({ backendRepo, resyncKey = 0, onOpenRuns, onOpenTasks }: { backendRepo: string | null; resyncKey?: number; onOpenRuns?: (sessionId?: string) => void; onOpenTasks?: () => void }) {
  const { t } = useLang()
  const [ov, setOv] = useState<Overview | null>(null)
  // 任务槽排队（与会话队列并列的第二类排队：任务 permit 满退回 pending，从未进会话队列）
  const [pendingTasks, setPendingTasks] = useState(0)
  // 终态红态：failed → 记住 label，红态 5s 后消失
  const [failed, setFailed] = useState<{ label: string } | null>(null)
  const ovRef = useRef<Overview | null>(null)
  useEffect(() => {
    ovRef.current = ov
  }, [ov])

  const pull = useCallback(() => {
    sessionsOverview()
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: Overview } | null) => setOv(d?.data ?? { active: [], queued: [] }))
      .catch(() => {})
    // 任务槽排队数：当前仓库 pending 任务（任务 permit 满退回 pending，不在会话队列里）
    if (backendRepo) {
      listTasks(backendRepo)
        .then((r) => (r.ok ? r.json() : null))
        .then((d: { data?: { status?: string }[] } | null) =>
          setPendingTasks((d?.data ?? []).filter((t) => t.status === 'pending').length),
        )
        .catch(() => {})
    }
  }, [backendRepo])

  // I5 刷新三挂钩：① 会话/队列事件；② WS 重连（resyncKey）；③ 20s 低频兜底
  useEffect(() => {
    pull()
    const offQ = onQueueChanged(() => pull())
    const offS = onSessionEvent(() => pull())
    // 任务事件（排队/执行/终态）也驱动任务槽排队数刷新——否则只靠 20s 兜底滞后
    const offT = onTaskEvent(() => pull())
    const t = window.setInterval(pull, 20000)
    return () => {
      offQ()
      offS()
      offT()
      window.clearInterval(t)
    }
  }, [pull, resyncKey])

  // 会话终态：failed → 记住 label，红态 5s 后消失
  useEffect(
    () =>
      onSessionEvent((e) => {
        if (e.status !== 'failed') return
        const hit = ovRef.current?.active.find((a) => a.sessionId === e.sessionId)
        const label = hit?.label ?? e.sessionId
        setFailed({ label })
        toast(t('shell.bubble.sessionFailed', { label }), 'error')
        window.setTimeout(() => setFailed((f) => (f?.label === label ? null : f)), 5000)
      }),
    // t 为模块级稳定函数，仅作翻译取词（含它仅为满足 exhaustive-deps）
    [t],
  )

  const activeList = failed ? [] : ov?.active ?? []
  const queuedList = ov?.queued ?? []
  const primary = activeList[0] ?? null
  const primaryStartedMs = primary?.startedAt ? toMs(primary.startedAt) : null
  const queuedPrimary = queuedList[0] ?? null
  const kind = primary
    ? kindFromLabel(primary.label)
    : queuedPrimary
      ? (queuedPrimary.kind as QueueKind)
      : kindFromLabel(failed?.label ?? '')

  // 已运行时长 tick：interval 只强制重渲染，渲染期现取 wall clock（SessionBubble 同款实弹修复）
  const [, forceRender] = useReducer((x: number) => x + 1, 0)
  const needTicker = activeList.length > 0 || queuedList.length > 0 || !!failed
  useEffect(() => {
    if (!needTicker) return
    const t = window.setInterval(forceRender, 1000)
    return () => window.clearInterval(t)
  }, [needTicker])

  if (!failed && activeList.length === 0 && queuedList.length === 0) return null

  // 每次渲染现取 wall clock（非 hook，可在 early return 之后）
  // eslint-disable-next-line react-hooks/purity -- 时长必须在渲染时读真实时钟：定时器被 webview 节流时状态快照会过期导致"从0重计"（2026-10-04 实弹修复，见 git log）
  const now = Date.now()
  const elapsed = primaryStartedMs !== null ? formatElapsed(now - primaryStartedMs) : null
  // 任务执行/审查等未知 label（kindFromLabel 返回 null）→ Activity 图标
  const Icon = kind === 'patrol' ? HeartPulse : kind === 'submap' ? Search : kind === 'reinduce' ? Sparkles : Activity

  const kill = (a: Pick<OverviewActive, 'sessionId' | 'repo' | 'label'>) => {
    if (!window.confirm(t('shell.bubble.killConfirm', { label: a.label, repo: a.repo }))) return
    killSession(a.repo, a.sessionId)
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        toast(t('shell.bubble.killOk', { label: a.label }))
        pull()
      })
      .catch(() => toast(t('shell.bubble.killErr'), 'error'))
  }

  const cancelQueue = (q: OverviewQueued) => {
    cancelSessionQueue(q.repo)
      .then(async (r) => {
        if (r.status === 404) {
          const b = (await r.json().catch(() => null)) as { error?: string } | null
          toast(b?.error ?? t('shell.bubble.queueNone'), 'error')
        } else if (!r.ok) {
          toast(t('shell.bubble.queueCancelFailed', { status: r.status }), 'error')
        } else {
          toast(t('shell.bubble.queueCancelled'))
        }
        pull()
      })
      .catch(() => toast(t('shell.bubble.queueCancelErr'), 'error'))
  }

  const foreign = (repo: string) => backendRepo && repo !== backendRepo

  return (
    <div className="group relative select-none" data-no-drag>
      {/* 顶部细条状态丸：26px 高单行——迷你转环 + 脉冲点 + label·进行中 + 已运行时长。
          点击整丸直达「运行」页看完整流水（跨仓库）；多会话时聚合计数 */}
      <div
        onClick={() => onOpenRuns?.(primary?.sessionId)}
        role={onOpenRuns ? 'button' : undefined}
        title={onOpenRuns ? t('shell.bubble.viewRunsTip') : undefined}
        className={`glass elev-2 flex h-[26px] items-center gap-1.5 rounded-full border pl-1 pr-2.5 ${onOpenRuns ? 'cursor-pointer' : ''} ${
          failed ? 'border-red-300 dark:border-red-800' : 'border-slate-200 dark:border-slate-700'
        }`}
      >
        <span className="relative flex h-[18px] w-[18px] shrink-0 items-center justify-center">
          {primary && (
            <svg className="absolute inset-0 animate-spin" style={{ animationDuration: '2.4s' }} viewBox="0 0 18 18" aria-hidden>
              <circle cx="9" cy="9" r="7.5" fill="none" stroke={failed ? '#fecaca' : '#e0e7ff'} strokeWidth="2" />
              <circle
                cx="9"
                cy="9"
                r="7.5"
                fill="none"
                stroke={failed ? '#ef4444' : '#6366f1'}
                strokeWidth="2"
                strokeLinecap="round"
                strokeDasharray="12 35"
              />
            </svg>
          )}
          {primary ? (
            <Icon size={10} className={failed ? 'text-red-500' : 'text-indigo-500'} />
          ) : (
            <Clock size={10} className="text-amber-500" />
          )}
        </span>
        <span
          className={`h-1.5 w-1.5 shrink-0 rounded-full ${
            failed ? 'bg-red-500' : primary ? 'animate-pulse bg-indigo-500' : 'animate-pulse bg-amber-500'
          }`}
        />
        <span className="max-w-[220px] truncate whitespace-nowrap text-cap font-semibold leading-none text-slate-700 dark:text-slate-200">
          {failed
            ? t('shell.bubble.failed', { label: failed.label })
            : primary
              ? activeList.length > 1
                ? t('shell.bubble.manyRunning', { count: activeList.length })
                : t('shell.bubble.running', {
                    label: foreign(primary.repo) ? `${primary.label} · ${primary.repo}` : primary.label,
                  })
              : t('shell.bubble.queued', {
                  label: `${queuedPrimary?.label ?? ''}${foreign(queuedPrimary?.repo ?? '') ? ` · ${queuedPrimary?.repo}` : ''}`,
                })}
        </span>
        <span className="tnum whitespace-nowrap text-micro leading-none text-slate-400 dark:text-slate-500">
          {failed
            ? t('shell.bubble.sessionEnded')
            : primary
              ? elapsed !== null
                ? t('shell.bubble.elapsed', { elapsed })
                : t('shell.bubble.runningShort')
              : t('shell.bubble.autoStart')}
        </span>
        {/* 排队 chip：琥珀小胶囊，跨仓库逐个列出，行内 × 随时取消 */}
        {queuedList.slice(0, 2).map((q) => (
          <span
            key={q.repo}
            className="flex items-center gap-1 rounded-full border border-amber-200 bg-amber-50 py-0.5 pl-1.5 pr-0.5 text-micro font-medium leading-none text-amber-700 dark:border-amber-900/60 dark:bg-amber-950/50 dark:text-amber-300"
          >
            <Clock size={9} />
            <span className="max-w-[110px] truncate">
              {t('shell.bubble.queuedChip', { label: `${q.label}${foreign(q.repo) ? ` · ${q.repo}` : ''}` })}
            </span>
            <button
              onClick={(e) => {
                e.stopPropagation()
                cancelQueue(q)
              }}
              className="rounded-full p-0.5 text-amber-400 transition-colors hover:bg-amber-100 hover:text-amber-700 dark:text-amber-600 dark:hover:bg-amber-900/50 dark:hover:text-amber-300"
              title={t('shell.bubble.cancelQueueTip')}
            >
              <X size={9} />
            </button>
          </span>
        ))}
        {/* 任务槽排队 chip：任务并发 permit 满退回 pending 的任务（不在会话队列，单列） */}
        {pendingTasks > 0 && (
          <span
            role={onOpenTasks ? 'button' : undefined}
            onClick={(e) => {
              if (!onOpenTasks) return
              e.stopPropagation()
              onOpenTasks()
            }}
            className={`flex items-center gap-1 rounded-full border border-amber-200 bg-amber-50 py-0.5 pl-1.5 pr-2 text-micro font-medium leading-none text-amber-700 dark:border-amber-900/60 dark:bg-amber-950/50 dark:text-amber-300 ${onOpenTasks ? 'cursor-pointer hover:border-amber-400' : ''}`}
            title={t('shell.bubble.tasksQueuedTip')}
          >
            <Clock size={9} />
            {t('shell.bubble.tasksQueued', { count: pendingTasks })}
          </span>
        )}
      </div>

      {/* 悬停详情卡：跨仓库列出全部活动会话（含仓库），可分别终止——纯展示拆件见 ./SessionBubbleCard */}
      {activeList.length > 0 && !failed && (
        <SessionBubbleCard activeList={activeList} primaryStartedMs={primaryStartedMs} now={now} onKill={kill} />
      )}
    </div>
  )
}
