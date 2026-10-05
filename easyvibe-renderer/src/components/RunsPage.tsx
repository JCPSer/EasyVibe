import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react'
import {
  Activity,
  ChevronDown,
  ChevronRight,
  Clock,
  HeartPulse,
  Loader2,
  Search,
  Sparkles,
  Square,
  Terminal,
  ListTree,
  LayoutList,
} from 'lucide-react'
import { onQueueChanged, onSessionEvent, onSessionOutput } from '@/runtime/growthBus'
import { toast } from '@/runtime/toast'
import { toMs } from '@/shared/logic/diffStat'
import { formatElapsed, kindFromLabel, type SessionQueueKind } from '@/runtime/sessionQueue'
import { useRunsAutoSelect } from '@/hooks/useRunsAutoSelect'

// 「运行」页（docs/runs-page-design-v1.md M2 完成体）：
// 左栏三态会话列表（运行中/排队中/历史——历史为 agent_sessions 库表，重启后仍可回放）
// + 右栏单会话流水三档呈现（进展卡/时间线/终端）。
// 数据面：GET /session-queue（活动快照）+ WS 实时增量 + GET /sessions/{sid}/output（回放/补拉）。
// M2 补拉语义：每会话 seq 单调（WS 事件带 seq），断线重连后按 lastSeq 拉差集，幂等去重。
// 速度显示 = 60s 滑窗行频；tokens/s 待 usage 解析后接入（用量页同源）。

interface StreamLine {
  seq: number
  stream: string
  line: string
  t: number
}

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

const KIND_NAME: Record<string, string> = {
  induce: '归纳', patrol: '巡检', submap: '深入分析', task: '任务执行',
  'subagent-review': '任务执行 · 初审', 'subagent-audit': '任务执行 · 审查', unknown: '会话',
}
const MAX_LINES = 2000
const RATE_WINDOW_MS = 60_000

function kindIcon(kind: string) {
  return kind === 'patrol' ? HeartPulse : kind === 'submap' ? Search : kind === 'task' || kind.startsWith('subagent') ? Activity : Sparkles
}

/** 60s 滑窗行频 → 「N 行/分」 */
function linesPerMinute(lines: StreamLine[], now: number): number {
  const cutoff = now - RATE_WINDOW_MS
  let n = 0
  for (let i = lines.length - 1; i >= 0; i--) {
    if (lines[i].t < cutoff) break
    n++
  }
  return n
}

/** 时间线分段：思考块(可折叠)/err/result/文本 */
type Block = { kind: 'think' | 'text' | 'err' | 'result'; lines: string[] }

function toBlocks(lines: StreamLine[]): Block[] {
  const blocks: Block[] = []
  for (const { line, stream } of lines) {
    const isThink = line.startsWith('[思考]')
    const isErr = stream === 'stderr' || line.startsWith('[err]')
    const isResult = line.includes('[EASYVIBE-RESULT]')
    const kind: Block['kind'] = isErr ? 'err' : isResult ? 'result' : isThink ? 'think' : 'text'
    const clean = isThink ? line.slice('[思考]'.length).trim() : isErr && line.startsWith('[err]') ? line.slice('[err]'.length).trim() : line
    const last = blocks[blocks.length - 1]
    if (last && last.kind === kind && kind !== 'result') last.lines.push(clean)
    else blocks.push({ kind, lines: [clean] })
  }
  return blocks
}

type Tier = 'card' | 'timeline' | 'terminal'
const TIERS: { id: Tier; label: string; icon: typeof Terminal }[] = [
  { id: 'card', label: '进展卡', icon: LayoutList },
  { id: 'timeline', label: '时间线', icon: ListTree },
  { id: 'terminal', label: '终端', icon: Terminal },
]

/** 会话最后一条有效输出（进展卡与运行卡共用） */
function lastTextOf(lines: StreamLine[]): string {
  for (let i = lines.length - 1; i >= 0; i--) {
    const l = lines[i].line
    if (l && !l.startsWith('[err]')) return l.startsWith('[思考]') ? l.slice(4).trim() : l
  }
  return ''
}

export function RunsPage({ backendRepo, initialSessionId, onInitialConsumed, resyncKey = 0 }: {
  backendRepo: string | null
  /** 用量页明细表跳入：打开页面即选中该会话 */
  initialSessionId?: string | null
  onInitialConsumed?: () => void
  /** WS 重连信号：触发全量补拉（断线期间的行按 seq 差集拉回） */
  resyncKey?: number
}) {
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
  const [collapsed, setCollapsed] = useState<Set<number>>(new Set())
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const followRef = useRef(true)

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
      fetch(
        `/api/repos/${encodeURIComponent(repo)}/sessions/${encodeURIComponent(sessionId)}/output?afterSeq=${afterSeq}&limit=5000`
      )
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
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/agent-sessions`)
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
    fetch('/api/sessions/overview')
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
      if (e.type === 'failed') toast(`排队任务启动失败：${e.error ?? '未知原因'}`, 'error')
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
  }, [backendRepo, pull, loadHistory, appendLines])

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
    const t = window.setInterval(forceRender, 1000)
    return () => window.clearInterval(t)
  }, [needTicker])

  // 自动跟随底部
  useEffect(() => {
    const el = scrollRef.current
    if (el && followRef.current) el.scrollTop = el.scrollHeight
  })

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
  const blocks = useMemo(() => (tier === 'timeline' ? toBlocks(selectedLines) : []), [tier, selectedLines])
  const rate = linesPerMinute(selectedLines, now)
  const lastText = useMemo(() => lastTextOf(selectedLines), [selectedLines])

  if (!backendRepo) {
    return <div className="flex h-full items-center justify-center text-[12px] text-slate-400 dark:text-slate-500">先在左侧选择一个项目。</div>
  }

  const kill = () => {
    if (!active) return
    if (!window.confirm(`确定终止「${active.label}」？该操作不可撤销。`)) return
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/sessions/${encodeURIComponent(active.sessionId)}/kill`, { method: 'POST' })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        toast(`已终止「${active.label}」`)
        pull()
      })
      .catch(() => toast('终止失败（请确认后端在线后重试）。', 'error'))
  }

  const cancelQueue = (q: OverviewQueued) => {
    fetch(`/api/repos/${encodeURIComponent(q.repo)}/session-queue`, { method: 'DELETE' })
      .then(async (r) => {
        if (r.status === 404) toast('没有排队任务', 'error')
        else if (!r.ok) toast(`取消排队失败（HTTP ${r.status}）`, 'error')
        else toast('已取消排队')
        pull()
      })
      .catch(() => toast('取消排队失败（请确认后端在线后重试）。', 'error'))
  }

  const elapsed = startedMs !== null ? formatElapsed(now - startedMs) : null

  const histKind = (row: HistoryRow): SessionQueueKind =>
    row.kind === 'patrol' ? 'patrol' : row.kind === 'submap' ? 'submap' : 'reinduce'

  return (
    <div className="flex h-full flex-col p-5">
      {/* 头部 */}
      <div className="mb-4 flex items-start justify-between">
        <div>
          <h2 className="flex items-center gap-1.5 text-[15px] font-bold text-slate-800 dark:text-slate-100">
            <Activity size={15} className="text-blue-500" /> 运行
          </h2>
          <p className="mt-0.5 text-[11px] text-slate-400 dark:text-slate-500">
            每个正在跑的 agent：它在做什么、做到哪一步、出了什么错。
          </p>
        </div>
        <div className="flex items-center gap-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-0.5">
          {TIERS.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              onClick={() => setTier(id)}
              className={`flex items-center gap-1 rounded-md px-2.5 py-1.5 text-[12px] font-semibold transition-colors ${
                tier === id ? 'bg-blue-50 dark:bg-blue-950/40 text-blue-600' : 'text-slate-400 dark:text-slate-500 hover:text-slate-600'
              }`}
            >
              <Icon size={12} />
              {label}
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
              <span className="h-1.5 w-1.5 rounded-full bg-blue-500" /> 运行中{activeList.length > 0 ? `（${activeList.length}）` : ''}
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
                          const Icon = kindIcon(kindFromLabel(a.label))
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
                      <p className="mt-1 truncate text-micro text-slate-400 dark:text-slate-500">{lastTextOf(lines) || '等待输出…'}</p>
                      <p className="tnum mt-0.5 text-micro text-slate-400 dark:text-slate-500">输出 {linesPerMinute(lines, now)} 行/分</p>
                    </button>
                  )
                })}
              </div>
            ) : (
              <p className="rounded-xl border border-dashed border-slate-200 dark:border-slate-700 px-3 py-3 text-center text-[11px] text-slate-400 dark:text-slate-500">
                没有运行中的会话
              </p>
            )}
          </section>

          {/* 排队中（跨仓库） */}
          {overview.queued.length > 0 && (
            <section>
              <p className="mb-1.5 flex items-center gap-1.5 text-cap font-semibold text-slate-400 dark:text-slate-500">
                <span className="h-1.5 w-1.5 rounded-full bg-amber-500" /> 排队中（{overview.queued.length}）
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
                        <button onClick={() => cancelQueue(q)} className="rounded px-1 text-slate-300 dark:text-slate-600 hover:text-red-500" title="取消排队">
                          ✕
                        </button>
                      </div>
                      <p className="mt-1 text-micro text-slate-400 dark:text-slate-500">
                        {foreign ? `${q.repo} · ` : ''}排队 {qt !== null ? formatElapsed(now - qt) : '…'} · 当前会话结束后自动开始
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
              <span className="h-1.5 w-1.5 rounded-full bg-slate-400" /> 历史
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
                          {h.label ?? KIND_NAME[h.kind] ?? h.kind}
                        </span>
                        <span className={`shrink-0 rounded-full px-1.5 py-px text-micro font-semibold ${h.status === 'failed' ? 'bg-red-50 dark:bg-red-950/40 text-red-500' : h.status === 'succeeded' ? 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-600' : 'bg-slate-100 dark:bg-slate-800 text-slate-500'}`}>
                          {h.status === 'failed' ? '失败' : h.status === 'succeeded' ? '成功' : '进行中'}
                        </span>
                      </div>
                      <p className="tnum mt-0.5 text-micro text-slate-400 dark:text-slate-500">
                        {h.startedAt.slice(5, 16).replace('T', ' ')}
                        {durMs !== null && durMs >= 0 ? ` · 耗时 ${formatElapsed(durMs)}` : ''} · {h.id}
                      </p>
                    </button>
                  )
                })}
              </div>
            ) : (
              <div className="rounded-xl border border-dashed border-slate-200 dark:border-slate-700 px-3 py-3 text-center">
                <p className="text-[11px] text-slate-400 dark:text-slate-500">暂无历史会话</p>
                <p className="mt-0.5 text-micro leading-4 text-slate-300 dark:text-slate-600">归纳 / 巡检 / 任务执行后会留下可回放的流水。</p>
              </div>
            )}
          </section>
        </div>

        {/* 右栏：流水 */}
        <div className="col-span-3 flex min-h-0 flex-col rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
          {selectedId ? (
            <>
              {/* 会话头 */}
              <div className="flex items-center gap-2 border-b border-slate-100 dark:border-slate-800 px-4 py-2.5">
                {selectedMeta?.status === 'alive' ? (
                  <Loader2 size={14} className="shrink-0 animate-spin text-blue-500" />
                ) : selectedMeta?.status === 'failed' ? (
                  <span className="h-3.5 w-3.5 shrink-0 rounded-full bg-red-500" />
                ) : (
                  <span className="h-3.5 w-3.5 shrink-0 rounded-full bg-emerald-500" />
                )}
                <span className="min-w-0 flex-1 truncate text-[13px] font-bold text-slate-800 dark:text-slate-100">{selectedMeta?.label ?? selectedId}</span>
                <span className="mono text-micro text-slate-400 dark:text-slate-500">{selectedId}</span>
                {selectedMeta?.status === 'alive' && (
                  <>
                    <span className="tnum text-micro text-slate-400 dark:text-slate-500">
                      {elapsed !== null && active?.sessionId === selectedId ? `${elapsed} · ${rate} 行/分` : `${rate} 行/分`}
                    </span>
                    {active && (
                      <button
                        onClick={kill}
                        className="flex items-center gap-1 rounded-md border border-red-200 dark:border-red-900/60 bg-red-50 dark:bg-red-950/40 px-2 py-1 text-micro font-semibold text-red-600 hover:bg-red-100 dark:text-red-300 dark:hover:bg-red-900/40"
                        title="终止该会话（二次确认）"
                      >
                        <Square size={9} /> 终止
                      </button>
                    )}
                  </>
                )}
                {selectedMeta && selectedMeta.status !== 'alive' && (
                  <span className={`rounded-full px-1.5 py-px text-micro font-semibold ${selectedMeta.status === 'failed' ? 'bg-red-50 dark:bg-red-950/40 text-red-500' : 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-600'}`}>
                    {selectedMeta.status === 'failed' ? '失败' : '成功'}
                  </span>
                )}
              </div>

              {/* 流水主体（三档） */}
              {tier === 'card' && (
                <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 p-6">
                  {lastText ? (
                    <>
                      <p className="max-w-lg text-center text-[15px] font-semibold leading-7 text-slate-800 dark:text-slate-100">{lastText}</p>
                      <div className="flex items-center gap-3 text-micro text-slate-400 dark:text-slate-500">
                        {selectedMeta?.status === 'alive' && (
                          <>
                            <Loader2 size={11} className="animate-spin text-blue-500" />
                            {active?.sessionId === selectedId && elapsed !== null && <span className="tnum">已运行 {elapsed}</span>}
                            <span>·</span>
                            <span className="tnum">{rate} 行/分</span>
                          </>
                        )}
                        <span>·</span>
                        <span className="tnum">共 {selectedLines.length} 行</span>
                      </div>
                    </>
                  ) : (
                    <p className="text-[12px] text-slate-400 dark:text-slate-500">等待 agent 输出…</p>
                  )}
                </div>
              )}

              {tier === 'timeline' && (
                <div
                  ref={scrollRef}
                  onScroll={(e) => {
                    const el = e.currentTarget
                    followRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40
                  }}
                  className="min-h-0 flex-1 space-y-2 overflow-y-auto p-4"
                >
                  {blocks.map((b, i) => {
                    const isCollapsed = b.kind === 'think' && collapsed.has(i)
                    return (
                      <div
                        key={i}
                        className={`rounded-lg px-3 py-2 ${
                          b.kind === 'think'
                            ? 'bg-slate-50 dark:bg-slate-950/70'
                            : b.kind === 'err'
                              ? 'bg-red-50 dark:bg-red-950/40'
                              : b.kind === 'result'
                                ? 'bg-emerald-50 dark:bg-emerald-950/40'
                                : ''
                        }`}
                      >
                        {b.kind === 'think' && (
                          <button
                            onClick={() =>
                              setCollapsed((prev) => {
                                const next = new Set(prev)
                                if (next.has(i)) next.delete(i)
                                else next.add(i)
                                return next
                              })
                            }
                            className="flex w-full items-center gap-1.5 text-left"
                          >
                            {isCollapsed ? <ChevronRight size={11} className="shrink-0 text-slate-400" /> : <ChevronDown size={11} className="shrink-0 text-slate-400" />}
                            <span className="shrink-0 rounded bg-slate-200 dark:bg-slate-700 px-1 py-px text-micro font-semibold text-slate-500 dark:text-slate-300">思考</span>
                            <span className="min-w-0 flex-1 truncate text-[12px] text-slate-500 dark:text-slate-400">{b.lines[0] || '（空）'}</span>
                            <span className="tnum shrink-0 text-micro text-slate-300 dark:text-slate-600">{b.lines.length} 行</span>
                          </button>
                        )}
                        {(b.kind !== 'think' || !isCollapsed) && (
                          <div className={b.kind === 'think' ? 'mt-1.5 space-y-1 pl-4' : 'space-y-1'}>
                            {b.lines.map((l, j) => (
                              <p
                                key={j}
                                className={`text-[12px] leading-5 ${
                                  b.kind === 'err'
                                    ? 'font-mono text-red-600 dark:text-red-400'
                                    : b.kind === 'result'
                                      ? 'font-mono font-semibold text-emerald-600 dark:text-emerald-400'
                                      : b.kind === 'think'
                                        ? 'text-slate-500 dark:text-slate-400'
                                        : 'text-slate-700 dark:text-slate-200'
                                }`}
                              >
                                {l}
                              </p>
                            ))}
                          </div>
                        )}
                      </div>
                    )
                  })}
                  {selectedMeta?.status === 'alive' && (
                    <p className="flex items-center gap-1.5 pl-1 text-micro text-slate-400 dark:text-slate-500">
                      <Loader2 size={10} className="animate-spin text-blue-500" /> 正在输出…
                    </p>
                  )}
                  {blocks.length === 0 && selectedMeta?.status !== 'alive' && (
                    <p className="py-8 text-center text-[11px] text-slate-300 dark:text-slate-600">没有捕获到输出行</p>
                  )}
                </div>
              )}

              {tier === 'terminal' && (
                <div
                  ref={scrollRef}
                  onScroll={(e) => {
                    const el = e.currentTarget
                    followRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40
                  }}
                  className="min-h-0 flex-1 overflow-y-auto bg-slate-950 p-4"
                >
                  <pre className="font-mono text-[11px] leading-5 text-slate-200">
                    {selectedLines.slice(-200).map((l, i) => (
                      <div key={i} className={l.stream === 'stderr' || l.line.startsWith('[err]') ? 'text-red-400' : l.line.includes('[EASYVIBE-RESULT]') ? 'text-emerald-400' : undefined}>
                        {l.line}
                      </div>
                    ))}
                    {selectedLines.length === 0 && <span className="text-slate-500"># 等待输出…</span>}
                  </pre>
                </div>
              )}
            </>
          ) : (
            <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6">
              {overview.active.length === 0 && overview.queued.length === 0 && history.length === 0 ? (
                <>
                  <Activity size={20} className="text-slate-300 dark:text-slate-600" />
                  <p className="text-[12px] font-semibold text-slate-500 dark:text-slate-400">当前没有运行中的 agent</p>
                  <p className="text-[11px] text-slate-400 dark:text-slate-500">归纳 / 巡检 / 深入分析启动后，这里会实时显示它的流水。</p>
                </>
              ) : (
                <p className="text-[12px] text-slate-400 dark:text-slate-500">从左侧选择一个会话查看流水。</p>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
