import { useCallback, useEffect, useReducer, useRef, useState } from 'react'
import { Clock, HeartPulse, Search, Sparkles, X } from 'lucide-react'
import { onQueueChanged, onSessionEvent } from '@/lib/growthBus'
import { toast } from '@/lib/toast'
import { absTime, toMs } from '@/lib/diffStat'
import { formatElapsed, kindFromLabel } from '@/lib/sessionQueue'

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

export function SessionBubble({ backendRepo, resyncKey = 0, onOpenRuns }: { backendRepo: string | null; resyncKey?: number; onOpenRuns?: () => void }) {
  const [ov, setOv] = useState<Overview | null>(null)
  // 终态红态：failed → 记住 label，红态 5s 后消失
  const [failed, setFailed] = useState<{ label: string } | null>(null)
  const ovRef = useRef<Overview | null>(null)
  useEffect(() => {
    ovRef.current = ov
  }, [ov])

  const pull = useCallback(() => {
    fetch('/sessions/overview')
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: Overview } | null) => setOv(d?.data ?? { active: [], queued: [] }))
      .catch(() => {})
  }, [])

  // I5 刷新三挂钩：① 会话/队列事件；② WS 重连（resyncKey）；③ 20s 低频兜底
  useEffect(() => {
    pull()
    const offQ = onQueueChanged(() => pull())
    const offS = onSessionEvent(() => pull())
    const t = window.setInterval(pull, 20000)
    return () => {
      offQ()
      offS()
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
        toast(`「${label}」执行失败`, 'error')
        window.setTimeout(() => setFailed((f) => (f?.label === label ? null : f)), 5000)
      }),
    [],
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
  const Icon = kind === 'patrol' ? HeartPulse : kind === 'submap' ? Search : Sparkles

  const kill = (a: OverviewActive) => {
    if (!window.confirm(`确定取消「${a.label}」（${a.repo}）？该操作不可撤销。`)) return
    fetch(`/api/repos/${encodeURIComponent(a.repo)}/sessions/${encodeURIComponent(a.sessionId)}/kill`, {
      method: 'POST',
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        toast(`已取消「${a.label}」`)
        pull()
      })
      .catch(() => toast('取消失败（请确认后端在线后重试）。', 'error'))
  }

  const cancelQueue = (q: OverviewQueued) => {
    fetch(`/api/repos/${encodeURIComponent(q.repo)}/session-queue`, { method: 'DELETE' })
      .then(async (r) => {
        if (r.status === 404) {
          const b = (await r.json().catch(() => null)) as { error?: string } | null
          toast(b?.error ?? '没有排队任务', 'error')
        } else if (!r.ok) {
          toast(`取消排队失败（HTTP ${r.status}）`, 'error')
        } else {
          toast('已取消排队')
        }
        pull()
      })
      .catch(() => toast('取消排队失败（请确认后端在线后重试）。', 'error'))
  }

  const foreign = (repo: string) => backendRepo && repo !== backendRepo

  return (
    <div className="group relative select-none" data-no-drag>
      {/* 顶部细条状态丸：26px 高单行——迷你转环 + 脉冲点 + label·进行中 + 已运行时长。
          点击整丸直达「运行」页看完整流水（跨仓库）；多会话时聚合计数 */}
      <div
        onClick={onOpenRuns}
        role={onOpenRuns ? 'button' : undefined}
        title={onOpenRuns ? '查看 agent 流水（跨仓库）' : undefined}
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
            ? `${failed.label} · 失败`
            : primary
              ? activeList.length > 1
                ? `${activeList.length} 个会话进行中`
                : `${primary.label}${foreign(primary.repo) ? ` · ${primary.repo}` : ''} · 进行中`
              : `排队中：${queuedPrimary?.label ?? ''}${foreign(queuedPrimary?.repo ?? '') ? ` · ${queuedPrimary?.repo}` : ''}`}
        </span>
        <span className="tnum whitespace-nowrap text-micro leading-none text-slate-400 dark:text-slate-500">
          {failed ? '会话已结束' : primary ? (elapsed !== null ? `已运行 ${elapsed}` : '进行中…') : '结束后自动开始'}
        </span>
        {/* 排队 chip：琥珀小胶囊，跨仓库逐个列出，行内 × 随时取消 */}
        {queuedList.slice(0, 2).map((q) => (
          <span
            key={q.repo}
            className="flex items-center gap-1 rounded-full border border-amber-200 bg-amber-50 py-0.5 pl-1.5 pr-0.5 text-micro font-medium leading-none text-amber-700 dark:border-amber-900/60 dark:bg-amber-950/50 dark:text-amber-300"
          >
            <Clock size={9} />
            <span className="max-w-[110px] truncate">
              排队:{q.label}
              {foreign(q.repo) ? `·${q.repo}` : ''}
            </span>
            <button
              onClick={(e) => {
                e.stopPropagation()
                cancelQueue(q)
              }}
              className="rounded-full p-0.5 text-amber-400 transition-colors hover:bg-amber-100 hover:text-amber-700 dark:text-amber-600 dark:hover:bg-amber-900/50 dark:hover:text-amber-300"
              title="取消排队"
            >
              <X size={9} />
            </button>
          </span>
        ))}
      </div>

      {/* 悬停详情卡：跨仓库列出全部活动会话（仓库 · label · 起止 · 各自取消） */}
      {activeList.length > 0 && !failed && (
        <div className="pointer-events-none invisible absolute left-1/2 top-full z-50 mt-1.5 w-72 -translate-x-1/2 opacity-0 transition-opacity duration-150 group-hover:pointer-events-auto group-hover:visible group-hover:opacity-100">
          <div className="glass elev-3 anim-scale-in rounded-xl border border-slate-200 p-3 dark:border-slate-700">
          <p className="text-cap font-bold text-slate-700 dark:text-slate-200">
            运行中的会话（{activeList.length}）
          </p>
          <div className="mt-1.5 space-y-2">
            {activeList.map((a) => {
              const st = a.startedAt ? toMs(a.startedAt) : null
              return (
                <div key={a.sessionId} className="rounded-lg bg-slate-50 dark:bg-slate-950/70 px-2.5 py-2">
                  <div className="flex items-center gap-1.5">
                    <span className="min-w-0 flex-1 truncate text-cap font-semibold text-slate-700 dark:text-slate-200">{a.label}</span>
                    <span className="shrink-0 rounded bg-blue-50 dark:bg-blue-950/40 px-1 py-px text-micro font-semibold text-blue-600 dark:text-blue-300">
                      {a.repo}
                    </span>
                  </div>
                  <div className="mt-0.5 flex items-center justify-between text-micro text-slate-500 dark:text-slate-400">
                    <span className="mono">{a.sessionId}</span>
                    <span className="tnum">
                      {st !== null ? `已运行 ${formatElapsed(now - st)}` : '…'}
                    </span>
                  </div>
                  <button
                    onClick={() => kill(a)}
                    className="mt-1.5 flex w-full items-center justify-center gap-1 rounded-md border border-red-200 bg-red-50 px-2 py-1 text-micro font-semibold text-red-600 transition-colors hover:bg-red-100 dark:border-red-900/60 dark:bg-red-950/40 dark:text-red-300 dark:hover:bg-red-900/40"
                    title="取消该会话（二次确认）"
                  >
                    <X size={10} />
                    取消该会话
                  </button>
                </div>
              )
            })}
          </div>
          {primaryStartedMs !== null && (
            <p className="mt-2 flex justify-between text-micro text-slate-400 dark:text-slate-500">
              <span>启动时间</span>
              <span className="tnum">{absTime(new Date(primaryStartedMs).toISOString()).slice(5)}</span>
            </p>
          )}
          </div>
        </div>
      )}
    </div>
  )
}
