import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react'
import {
  Activity,
  Clock,
  HeartPulse,
  Search,
  Sparkles,
} from 'lucide-react'
import { onQueueChanged, onSessionEvent, onSessionOutput } from '@/runtime/growthBus'
import { toast } from '@/runtime/toast'
import { toMs } from '@/shared/logic/diffStat'
import { formatElapsed, kindFromLabel, type SessionQueueKind } from '@/runtime/sessionQueue'
import { useRunsAutoSelect } from '@/hooks/useRunsAutoSelect'
import { useLang } from '@/runtime/i18n'
import { agentSessions, sessionsOverview } from '@/api/repos'
import { sessionOutput, killSession, cancelSessionQueue } from '@/api/system'
import { RunsStreamView, TIERS, lastTextOf, linesPerMinute, type StreamLine, type Tier } from './RunsStreamView'

// 「运行」页（docs/runs-page-design-v1.md M2 完成体）：
// 左栏三态会话列表（运行中/排队中/历史——历史为 agent_sessions 库表，重启后仍可回放）
// + 右栏单会话流水三档呈现（进展卡/时间线/终端，RunsStreamView）。
// 数据面：GET /session-queue（活动快照）+ WS 实时增量 + GET /sessions/{sid}/output（回放/补拉）。
// M2 补拉语义：每会话 seq 单调（WS 事件带 seq），断线重连后按 lastSeq 拉差集，幂等去重。
// 速度显示 = 60s 滑窗行频；tokens/s 待 usage 解析后接入（用量页同源）。

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
interface HistoryRow {
  id: string
  repo: string
  kind: string
  label?: string | null
  startedAt: string
  terminalAt?: string | null
  status: string
  exitCode?: number | null
}

const MAX_LINES = 2000

function kindIcon(kind: string) {
  return kind === 'patrol' ? HeartPulse : kind === 'submap' ? Search : kind === 'task' || kind.startsWith('subagent') ? Activity : Sparkles
}

export function RunsPage({ backendRepo, initialSessionId, onInitialConsumed, resyncKey = 0 }: {
  backendRepo: string | null
  /** 用量页明细表跳入：打开页面即选中该会话 */
  initialSessionId?: string | null
  onInitialConsumed?: () => void
  /** WS 重连信号：触发全量补拉（断线期间的行按 seq 差集拉回） */
  resyncKey?: number
}) {
  const { t } = useLang()
  // 跨仓库全局视角（2026-10-05）：A 仓库分析中切到 B，A 的会话仍在此可见
  const [overview, setOverview] = useState<{ active: OverviewActive[]; queued: OverviewQueued[] }>({ active: [], queued: [] })
  const sessionsRepoRef = useRef<Map<string, string>>(new Map())
  const [history, setHistory] = useState<HistoryRow[]>([])
  // 各会话流水（内存态；历史会话选中时从库表回放补入；每会话按 seq 去重）
  const [streams, setStreams] = useState<Map<string, StreamLine[]>>(new Map())
  const seqSeenRef = useRef<Map<string, Set<number>>>(new Map())
  const streamsRef = useRef(streams)
  useEffect(() => {
    streamsRef.current = streams
  }, [streams])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [tier, setTier] = useState<Tier>('timeline')

  /** 追加行（seq 幂等去重；每会话内存最多留 MAX_LINES 行，全量在库表） */
  const appendLines = useCallback((sessionId: string, lines: StreamLine[]) => {
    if (lines.length === 0) return
    let seen = seqSeenRef.current.get(sessionId)
    if (!seen) {
      seen = new Set()
      seqSeenRef.current.set(sessionId, seen)
    }
    const fresh = lines.filter((l) => l.seq >= 0 && !seen.has(l.seq))
    if (fresh.length === 0) return
    fresh.forEach((l) => seen.add(l.seq))
    setStreams((prev) => {
      const list = prev.get(sessionId) ?? []
      const next = [...list, ...fresh].sort((a, b) => a.seq - b.seq)
      if (next.length > MAX_LINES) next.splice(0, next.length - MAX_LINES)
      const m = new Map(prev)
      m.set(sessionId, next)
      return m
    })
  }, [])

  /** 回放/补拉：afterSeq 之后的行（历史会话 afterSeq=0 全量回放） */
  const backfill = useCallback(
    (sessionId: string, afterSeq = 0, repoOverride?: string) => {
      const repo = repoOverride ?? sessionsRepoRef.current.get(sessionId) ?? backendRepo
      if (!repo) return
      sessionOutput(repo, sessionId, afterSeq)
        .then((r) => (r.ok ? r.json() : null))
        .then((d: { data?: { seq: number; stream: string; line: string; ts: string }[] } | null) => {
          const rows = d?.data ?? []
          if (rows.length === 0) return
          appendLines(
            sessionId,
            rows.map((r) => ({ seq: r.seq, stream: r.stream, line: r.line, t: toMs(r.ts) ?? Date.now() }))
          )
        })
        .catch(() => {})
    },
    [backendRepo, appendLines]
  )

  const loadHistory = useCallback(() => {
    if (!backendRepo) return
    agentSessions(backendRepo)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: HistoryRow[] } | null) => {
        const rows = d?.data ?? []
        setHistory(rows)
        rows.forEach((r) => sessionsRepoRef.current.set(r.id, r.repo))
      })
      .catch(() => {})
  }, [backendRepo])

  /** 选中会话：无内存流水时从库表回放（按会话所属仓库——支持跨仓库选中） */
  const select = useCallback(
    (id: string, repo?: string) => {
      setSelectedId(id)
      if (repo) sessionsRepoRef.current.set(id, repo)
      if (!seqSeenRef.current.get(id)?.size) backfill(id, 0, repo)
    },
    [backfill]
  )

  const pull = useCallback(() => {
    sessionsOverview()
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { active: OverviewActive[]; queued: OverviewQueued[] } } | null) => {
        const ov = d?.data ?? { active: [], queued: [] }
        setOverview(ov)
        ov.active.forEach((a) => sessionsRepoRef.current.set(a.sessionId, a.repo))
      })
      .catch(() => {})
  }, [])

  useEffect(() => {
    pull()
    loadHistory()
    const offQ = onQueueChanged((e) => {
      if (e.repo !== backendRepo) return
      if (e.type === 'failed') toast(t('pages.runs.queueStartFailed', { reason: e.error ?? t('common.unknownReason') }), 'error')
      pull()
    })
    const offS = onSessionEvent((e) => {
      if (e.repo !== backendRepo) return
      pull()
      loadHistory()
    })
    const offO = onSessionOutput((e) => {
      appendLines(e.sessionId, [{ seq: e.seq, stream: e.stream, line: e.line, t: Date.now() }])
    })
    return () => {
      offQ()
      offS()
      offO()
    }
  }, [backendRepo, pull, loadHistory, appendLines, t])

  // 选中逻辑（deeplink + 自动选中）已抽 hooks/useRunsAutoSelect——保组件体量红线
  useRunsAutoSelect({ selectedId, initialSessionId, active: overview.active, history, select, onInitialConsumed })

  // WS 重连补拉：每个有流水的会话按 lastSeq 拉差集（幂等去重）
  useEffect(() => {
    if (resyncKey === 0) return
    for (const [id, lines] of streamsRef.current) {
      const last = lines[lines.length - 1]
      if (last) backfill(id, last.seq)
    }
    pull()
    loadHistory()
    // eslint-disable-next-line react-hooks/exhaustive-deps -- resyncKey 为脉冲信号
  }, [resyncKey])

  // 已运行时长 tick：interval 只强制重渲染，渲染期现取 wall clock
  const [, forceRender] = useReducer((x: number) => x + 1, 0)
  const activeList = overview.active
  const active = selectedId ? activeList.find((a) => a.sessionId === selectedId) ?? null : null
  const selectedAliveRow = active
  const startedMs = selectedAliveRow?.startedAt ? toMs(selectedAliveRow.startedAt) : null
  const anyTicker = activeList.length > 0 || overview.queued.length > 0
  const needTicker = startedMs !== null || anyTicker
  useEffect(() => {
    if (!needTicker) return
    const timer = window.setInterval(forceRender, 1000)
    return () => window.clearInterval(timer)
  }, [needTicker])

  // eslint-disable-next-line react-hooks/purity -- 时长/行频必须在渲染时读真实时钟：定时器被 webview 节流时状态快照会过期（SessionBubble 同款，2026-10-04 实弹）
  const now = Date.now()

  const selectedLines = useMemo(() => (selectedId ? streams.get(selectedId) ?? [] : []), [selectedId, streams])
  const selectedMeta: { label: string; status: 'alive' | 'succeeded' | 'failed' } | null = useMemo(() => {
    if (!selectedId) return null
    const ovRow = overview.active.find((a) => a.sessionId === selectedId)
    if (ovRow) return { label: ovRow.label, status: 'alive' }
    const row = history.find((h) => h.id === selectedId)
    if (row) return { label: row.label ?? row.id, status: row.status === 'failed' ? 'failed' : 'succeeded' }
    return { label: selectedId, status: 'alive' }
  }, [selectedId, active, history])
  const rate = linesPerMinute(selectedLines, now)

  if (!backendRepo) {
    return <div className="flex h-full items-center justify-center text-[12px] text-slate-400 dark:text-slate-500">{t('common.pickProject')}</div>
  }

  const kill = () => {
    if (!active) return
    if (!window.confirm(t('pages.runs.killConfirm', { label: active.label }))) return
    killSession(backendRepo, active.sessionId)
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        toast(t('pages.runs.killOk', { label: active.label }))
        pull()
      })
      .catch(() => toast(t('pages.runs.killErr'), 'error'))
  }

  const cancelQueue = (q: OverviewQueued) => {
    cancelSessionQueue(q.repo)
      .then(async (r) => {
        if (r.status === 404) toast(t('shell.bubble.queueNone'), 'error')
        else if (!r.ok) toast(t('shell.bubble.queueCancelFailed', { status: r.status }), 'error')
        else toast(t('shell.bubble.queueCancelled'))
        pull()
      })
      .catch(() => toast(t('shell.bubble.queueCancelErr'), 'error'))
  }

  const elapsed = startedMs !== null ? formatElapsed(now - startedMs) : null

  const histKind = (row: HistoryRow): SessionQueueKind =>
    row.kind === 'patrol' ? 'patrol' : row.kind === 'submap' ? 'submap' : 'reinduce'

  // 历史行 kind 标签：kinds.* 字典 + 未知 kind 回退原值
  const kindName = (kind: string) => {
    const key = `kinds.${kind}`
    const s = t(key)
    return s === key ? t('pages.runs.kindUnknown') : s
  }

  return (
    <div className="flex h-full flex-col p-5">
      {/* 头部 */}
      <div className="mb-4 flex items-start justify-between">
        <div>
          <h2 className="flex items-center gap-1.5 text-[15px] font-bold text-slate-800 dark:text-slate-100">
            <Activity size={15} className="text-blue-500" /> {t('pages.runs.title')}
          </h2>
          <p className="mt-0.5 text-[11px] text-slate-400 dark:text-slate-500">
            {t('pages.runs.subtitle')}
          </p>
        </div>
        <div className="flex items-center gap-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-0.5">
          {TIERS.map(({ id, labelKey, icon: Icon }) => (
            <button
              key={id}
              onClick={() => setTier(id)}
              className={`flex items-center gap-1 rounded-md px-2.5 py-1.5 text-[12px] font-semibold transition-colors ${
                tier === id ? 'bg-blue-50 dark:bg-blue-950/40 text-blue-600' : 'text-slate-400 dark:text-slate-500 hover:text-slate-600'
              }`}
            >
              <Icon size={12} />
              {t(labelKey)}
            </button>
          ))}
        </div>
      </div>

      <div className="grid min-h-0 flex-1 grid-cols-4 gap-3">
        {/* 左栏：会话列表 */}
        <div className="col-span-1 min-h-0 space-y-3 overflow-y-auto pr-1">
          {/* 运行中（跨仓库——A 仓库分析中切到 B 仍可见） */}
          <section>
            <p className="mb-1.5 flex items-center gap-1.5 text-cap font-semibold text-slate-400 dark:text-slate-500">
              <span className="h-1.5 w-1.5 rounded-full bg-blue-500" /> {t('pages.runs.running')}{activeList.length > 0 ? `（${activeList.length}）` : ''}
            </p>
            {activeList.length > 0 ? (
              <div className="space-y-1.5">
                {activeList.map((a) => {
                  const lines = streams.get(a.sessionId) ?? []
                  const st = a.startedAt ? toMs(a.startedAt) : null
                  const foreign = backendRepo && a.repo !== backendRepo
                  return (
                    <button
                      key={a.sessionId}
                      onClick={() => select(a.sessionId, a.repo)}
                      className={`w-full rounded-xl border p-3 text-left transition-colors ${
                        selectedId === a.sessionId
                          ? 'border-blue-400 dark:border-blue-700 bg-blue-50 dark:bg-blue-950/30'
                          : 'border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 hover:border-slate-300 dark:hover:border-slate-600'
                      }`}
                    >
                      <div className="flex items-center gap-2">
                        {(() => {
                          const Icon = kindIcon(kindFromLabel(a.label) ?? 'task')
                          return <Icon size={13} className="shrink-0 text-blue-500" />
                        })()}
                        <span className="min-w-0 flex-1 truncate text-[12px] font-bold text-slate-700 dark:text-slate-200">{a.label}</span>
                        {foreign && (
                          <span className="shrink-0 rounded bg-blue-50 dark:bg-blue-950/40 px-1 py-px text-micro font-semibold text-blue-600 dark:text-blue-300">
                            {a.repo}
                          </span>
                        )}
                        <span className="tnum shrink-0 text-micro text-slate-400 dark:text-slate-500">
                          {st !== null ? formatElapsed(now - st) : '…'}
                        </span>
                      </div>
                      <p className="mt-1 truncate text-micro text-slate-400 dark:text-slate-500">{lastTextOf(lines) || t('pages.runs.waitingOutput')}</p>
                      <p className="tnum mt-0.5 text-micro text-slate-400 dark:text-slate-500">{t('pages.runs.rate', { count: linesPerMinute(lines, now) })}</p>
                    </button>
                  )
                })}
              </div>
            ) : (
              <p className="rounded-xl border border-dashed border-slate-200 dark:border-slate-700 px-3 py-3 text-center text-[11px] text-slate-400 dark:text-slate-500">
                {t('pages.runs.runningEmpty')}
              </p>
            )}
          </section>

          {/* 排队中（跨仓库） */}
          {overview.queued.length > 0 && (
            <section>
              <p className="mb-1.5 flex items-center gap-1.5 text-cap font-semibold text-slate-400 dark:text-slate-500">
                <span className="h-1.5 w-1.5 rounded-full bg-amber-500" /> {t('pages.runs.queued', { count: overview.queued.length })}
              </p>
              <div className="space-y-1.5">
                {overview.queued.map((q) => {
                  const qt = q.enqueuedAt ? toMs(q.enqueuedAt) : null
                  const foreign = backendRepo && q.repo !== backendRepo
                  return (
                    <div key={q.repo} className="rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-3">
                      <div className="flex items-center gap-2">
                        <Clock size={12} className="shrink-0 text-amber-500" />
                        <span className="min-w-0 flex-1 truncate text-[12px] font-semibold text-slate-600 dark:text-slate-300">{q.label}</span>
                        {foreign && (
                          <span className="shrink-0 rounded bg-amber-50 dark:bg-amber-950/40 px-1 py-px text-micro font-semibold text-amber-600 dark:text-amber-300">
                            {q.repo}
                          </span>
                        )}
                        <button onClick={() => cancelQueue(q)} className="rounded px-1 text-slate-300 dark:text-slate-600 hover:text-red-500" title={t('pages.runs.cancelQueueTip')}>
                          ✕
                        </button>
                      </div>
                      <p className="mt-1 text-micro text-slate-400 dark:text-slate-500">
                        {foreign ? `${q.repo} · ` : ''}{t('pages.runs.queuedMeta', { elapsed: qt !== null ? formatElapsed(now - qt) : '…' })}
                      </p>
                    </div>
                  )
                })}
              </div>
            </section>
          )}

          {/* 历史（agent_sessions 库表——M2 转正，重启后仍可回放） */}
          <section>
            <p className="mb-1.5 flex items-center gap-1.5 text-cap font-semibold text-slate-400 dark:text-slate-500">
              <span className="h-1.5 w-1.5 rounded-full bg-slate-400" /> {t('pages.runs.history')}
            </p>
            {history.length > 0 ? (
              <div className="space-y-1.5">
                {history.slice(0, 12).map((h) => {
                  const Icon = kindIcon(histKind(h))
                  const durMs = h.terminalAt ? (toMs(h.terminalAt) ?? 0) - (toMs(h.startedAt) ?? 0) : null
                  return (
                    <button
                      key={h.id}
                      onClick={() => select(h.id)}
                      className={`w-full rounded-lg border px-3 py-2 text-left transition-colors ${
                        selectedId === h.id
                          ? 'border-blue-400 dark:border-blue-700 bg-blue-50 dark:bg-blue-950/30'
                          : 'border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 hover:border-slate-300 dark:hover:border-slate-600'
                      }`}
                    >
                      <div className="flex items-center gap-2">
                        <Icon size={12} className={`shrink-0 ${h.status === 'failed' ? 'text-red-400' : 'text-slate-400 dark:text-slate-500'}`} />
                        <span className={`min-w-0 flex-1 truncate text-[12px] ${h.status === 'failed' ? 'font-semibold text-red-500' : 'text-slate-600 dark:text-slate-300'}`}>
                          {h.label ?? kindName(h.kind)}
                        </span>
                        <span className={`shrink-0 rounded-full px-1.5 py-px text-micro font-semibold ${h.status === 'failed' ? 'bg-red-50 dark:bg-red-950/40 text-red-500' : h.status === 'succeeded' ? 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-600' : 'bg-slate-100 dark:bg-slate-800 text-slate-500'}`}>
                          {h.status === 'failed' ? t('common.status.failed') : h.status === 'succeeded' ? t('common.status.succeeded') : t('common.status.running')}
                        </span>
                      </div>
                      <p className="tnum mt-0.5 text-micro text-slate-400 dark:text-slate-500">
                        {h.startedAt.slice(5, 16).replace('T', ' ')}
                        {durMs !== null && durMs >= 0 ? ` · ${t('pages.runs.duration', { elapsed: formatElapsed(durMs) })}` : ''} · {h.id}
                      </p>
                    </button>
                  )
                })}
              </div>
            ) : (
              <div className="rounded-xl border border-dashed border-slate-200 dark:border-slate-700 px-3 py-3 text-center">
                <p className="text-[11px] text-slate-400 dark:text-slate-500">{t('pages.runs.historyEmpty')}</p>
                <p className="mt-0.5 text-micro leading-4 text-slate-300 dark:text-slate-600">{t('pages.runs.historyHint')}</p>
              </div>
            )}
          </section>
        </div>

        {/* 右栏：流水（RunsStreamView 三档呈现） */}
        <div className="col-span-3 flex min-h-0 flex-col rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
          {selectedId ? (
            <RunsStreamView
              tier={tier}
              selectedId={selectedId}
              selectedLines={selectedLines}
              selectedMeta={selectedMeta}
              elapsed={elapsed !== null && active?.sessionId === selectedId ? elapsed : null}
              rate={rate}
              empty={overview.active.length === 0 && overview.queued.length === 0 && history.length === 0}
              onKill={active ? kill : undefined}
            />
          ) : (
            <RunsStreamView
              tier={tier}
              selectedId={null}
              selectedLines={[]}
              selectedMeta={null}
              elapsed={null}
              rate={0}
              empty={overview.active.length === 0 && overview.queued.length === 0 && history.length === 0}
            />
          )}
        </div>
      </div>
    </div>
  )
}
