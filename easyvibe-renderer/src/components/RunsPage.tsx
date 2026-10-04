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
  X,
} from 'lucide-react'
import { onQueueChanged, onSessionEvent, onSessionOutput } from '@/lib/growthBus'
import { toast } from '@/lib/toast'
import { toMs } from '@/lib/diffStat'
import { formatElapsed, isEmptyState, kindFromLabel, type SessionQueueKind, type SessionQueueSnapshot } from '@/lib/sessionQueue'

// 「运行」页（docs/runs-page-design-v1.md M3）：
// 左栏三态会话列表（运行中/排队中/历史-M1前诚实占位）+ 右栏单会话流水三档呈现（进展卡/时间线/终端）。
// 数据面：GET /session-queue 快照 + WS（statusChanged/queue.changed/session.output）实时增量；
// 速度显示 = 60s 滑窗行频（tokens/s 待 M1/M2 后端解析 usage 后接入）。

interface StreamLine {
  line: string
  t: number
}

interface EndedSession {
  sessionId: string
  label: string
  status: string
  lines: StreamLine[]
}

const MAX_LINES = 500
const RATE_WINDOW_MS = 60_000

function kindIcon(kind: SessionQueueKind) {
  return kind === 'patrol' ? HeartPulse : kind === 'submap' ? Search : Sparkles
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
  for (const { line } of lines) {
    const isThink = line.startsWith('[思考]')
    const isErr = line.startsWith('[err]')
    const isResult = line.includes('[EASYVIBE-RESULT]')
    const kind: Block['kind'] = isErr ? 'err' : isResult ? 'result' : isThink ? 'think' : 'text'
    const clean = isThink ? line.slice('[思考]'.length).trim() : isErr ? line.slice('[err]'.length).trim() : line
    const last = blocks[blocks.length - 1]
    // 同型才合并；result 行独立成块（协议行不粘连）
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

export function RunsPage({ backendRepo }: { backendRepo: string | null }) {
  const [snap, setSnap] = useState<SessionQueueSnapshot | null>(null)
  // 各会话流水（内存态，M2 落盘前重启即失——诚实降级见历史区）
  const [streams, setStreams] = useState<Map<string, StreamLine[]>>(new Map())
  const [ended, setEnded] = useState<EndedSession[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [tier, setTier] = useState<Tier>('timeline')
  const [collapsed, setCollapsed] = useState<Set<number>>(new Set())
  const streamsRef = useRef(streams)
  useEffect(() => {
    streamsRef.current = streams
  }, [streams])
  const snapRef = useRef(snap)
  useEffect(() => {
    snapRef.current = snap
  }, [snap])
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const followRef = useRef(true)

  const pull = useCallback(() => {
    if (!backendRepo) return
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/session-queue`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: SessionQueueSnapshot } | null) => setSnap(d?.data ?? null))
      .catch(() => {})
  }, [backendRepo])

  useEffect(() => {
    pull()
    const offQ = onQueueChanged((e) => {
      if (e.repo !== backendRepo) return
      if (e.type === 'failed') toast(`排队任务启动失败：${e.error ?? '未知原因'}`, 'error')
      pull()
    })
    const offS = onSessionEvent((e) => {
      if (e.repo !== backendRepo) return
      // 终态：流水留在内存里继续可看（直到离开页面/刷新）
      if (e.status === 'succeeded' || e.status === 'failed') {
        const cur = snapRef.current
        if (cur?.active?.sessionId === e.sessionId) {
          const lines = streamsRef.current.get(e.sessionId) ?? []
          setEnded((prev) =>
            [{ sessionId: e.sessionId, label: cur.active!.label, status: e.status, lines }, ...prev].slice(0, 5),
          )
        }
      }
      pull()
    })
    const offO = onSessionOutput((e) => {
      const t = Date.now()
      setStreams((prev) => {
        const list = prev.get(e.sessionId)
        const next = list ? [...list, { line: e.line, t }] : [{ line: e.line, t }]
        if (next.length > MAX_LINES) next.splice(0, next.length - MAX_LINES)
        const m = new Map(prev)
        m.set(e.sessionId, next)
        return m
      })
    })
    return () => {
      offQ()
      offS()
      offO()
    }
  }, [backendRepo, pull])

  // 已运行时长 tick：interval 只强制重渲染，渲染期现取 wall clock（SessionBubble 同款实弹修复）
  const [, forceRender] = useReducer((x: number) => x + 1, 0)
  const active = snap?.active ?? null
  const startedMs = active?.startedAt ? toMs(active.startedAt) : null
  const needTicker = startedMs !== null || !!snap?.queued
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

/** 会话最后一条有效输出（进展卡与运行卡共用） */
function lastTextOf(lines: StreamLine[]): string {
  for (let i = lines.length - 1; i >= 0; i--) {
    const l = lines[i].line
    if (l && !l.startsWith('[err]')) return l.startsWith('[思考]') ? l.slice(4).trim() : l
  }
  return ''
}

  const selectedLines = useMemo(() => {
    if (!selectedId) return []
    const dead = ended.find((e) => e.sessionId === selectedId)
    if (dead) return dead.lines
    return streams.get(selectedId) ?? []
  }, [selectedId, ended, streams])

  const selectedEnded = selectedId ? ended.find((e) => e.sessionId === selectedId) : undefined
  const selectedAlive = active && active.sessionId === selectedId
  const blocks = useMemo(() => (tier === 'timeline' ? toBlocks(selectedLines) : []), [tier, selectedLines])
  const rate = linesPerMinute(selectedLines, now)
  const lastText = useMemo(() => lastTextOf(selectedLines), [selectedLines])
  const activeLines = active ? streams.get(active.sessionId) ?? [] : []
  const activeLastText = lastTextOf(activeLines)

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

  const cancelQueue = () => {
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/session-queue`, { method: 'DELETE' })
      .then(async (r) => {
        if (r.status === 404) toast('没有排队任务', 'error')
        else if (!r.ok) toast(`取消排队失败（HTTP ${r.status}）`, 'error')
        else toast('已取消排队')
        pull()
      })
      .catch(() => toast('取消排队失败（请确认后端在线后重试）。', 'error'))
  }

  const elapsed = startedMs !== null ? formatElapsed(now - startedMs) : null
  const queuedAt = snap?.queued?.enqueuedAt ? toMs(snap.queued.enqueuedAt) : null

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
          {/* 运行中 */}
          <section>
            <p className="mb-1.5 flex items-center gap-1.5 text-cap font-semibold text-slate-400 dark:text-slate-500">
              <span className="h-1.5 w-1.5 rounded-full bg-blue-500" /> 运行中
            </p>
            {active ? (
              <button
                onClick={() => setSelectedId(active.sessionId)}
                className={`w-full rounded-xl border p-3 text-left transition-colors ${
                  selectedId === active.sessionId
                    ? 'border-blue-400 dark:border-blue-700 bg-blue-50 dark:bg-blue-950/30'
                    : 'border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 hover:border-slate-300 dark:hover:border-slate-600'
                }`}
              >
                <div className="flex items-center gap-2">
                  {(() => {
                    const Icon = kindIcon(kindFromLabel(active.label))
                    return <Icon size={13} className="shrink-0 text-blue-500" />
                  })()}
                  <span className="min-w-0 flex-1 truncate text-[12px] font-bold text-slate-700 dark:text-slate-200">{active.label}</span>
                  <span className="tnum text-micro text-slate-400 dark:text-slate-500">{elapsed !== null ? `已运行 ${elapsed}` : '…'}</span>
                </div>
                <p className="mt-1 truncate text-micro text-slate-400 dark:text-slate-500">
                  {activeLastText || '等待输出…'}
                </p>
                <p className="tnum mt-0.5 text-micro text-slate-400 dark:text-slate-500">输出 {linesPerMinute(streams.get(active.sessionId) ?? [], now)} 行/分</p>
              </button>
            ) : (
              <p className="rounded-xl border border-dashed border-slate-200 dark:border-slate-700 px-3 py-3 text-center text-[11px] text-slate-400 dark:text-slate-500">
                没有运行中的会话
              </p>
            )}
          </section>

          {/* 排队中 */}
          {snap?.queued && (
            <section>
              <p className="mb-1.5 flex items-center gap-1.5 text-cap font-semibold text-slate-400 dark:text-slate-500">
                <span className="h-1.5 w-1.5 rounded-full bg-amber-500" /> 排队中
              </p>
              <div className="rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-3">
                <div className="flex items-center gap-2">
                  <Clock size={12} className="shrink-0 text-amber-500" />
                  <span className="min-w-0 flex-1 truncate text-[12px] font-semibold text-slate-600 dark:text-slate-300">{snap.queued.label}</span>
                  <button onClick={cancelQueue} className="rounded p-0.5 text-slate-300 dark:text-slate-600 hover:text-red-500" title="取消排队">
                    <X size={11} />
                  </button>
                </div>
                <p className="mt-1 text-micro text-slate-400 dark:text-slate-500">
                  排队 {queuedAt !== null ? formatElapsed(now - queuedAt) : '…'} · 当前会话结束后自动开始
                </p>
              </div>
            </section>
          )}

          {/* 本次期间已结束（内存态） */}
          {ended.length > 0 && (
            <section>
              <p className="mb-1.5 flex items-center gap-1.5 text-cap font-semibold text-slate-400 dark:text-slate-500">
                <span className="h-1.5 w-1.5 rounded-full bg-slate-400" /> 本次已结束
              </p>
              <div className="space-y-1.5">
                {ended.map((e) => (
                  <button
                    key={e.sessionId}
                    onClick={() => setSelectedId(e.sessionId)}
                    className={`w-full rounded-lg border px-3 py-2 text-left transition-colors ${
                      selectedId === e.sessionId
                        ? 'border-blue-400 dark:border-blue-700 bg-blue-50 dark:bg-blue-950/30'
                        : 'border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 hover:border-slate-300 dark:hover:border-slate-600'
                    }`}
                  >
                    <div className="flex items-center gap-2">
                      <span className={`min-w-0 flex-1 truncate text-[12px] ${e.status === 'failed' ? 'font-semibold text-red-500' : 'text-slate-600 dark:text-slate-300'}`}>
                        {e.label}
                      </span>
                      <span className={`rounded-full px-1.5 py-px text-micro font-semibold ${e.status === 'failed' ? 'bg-red-50 dark:bg-red-950/40 text-red-500' : 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-600'}`}>
                        {e.status === 'failed' ? '失败' : '成功'}
                      </span>
                    </div>
                    <p className="tnum mt-0.5 text-micro text-slate-400 dark:text-slate-500">{e.lines.length} 行输出 · {e.sessionId}</p>
                  </button>
                ))}
              </div>
            </section>
          )}

          {/* 历史（M1 诚实占位） */}
          <section>
            <p className="mb-1.5 flex items-center gap-1.5 text-cap font-semibold text-slate-400 dark:text-slate-500">
              <span className="h-1.5 w-1.5 rounded-full bg-slate-300 dark:bg-slate-600" /> 历史
            </p>
            <div className="rounded-xl border border-dashed border-slate-200 dark:border-slate-700 px-3 py-3 text-center">
              <p className="text-[11px] text-slate-400 dark:text-slate-500">会话历史即将支持</p>
              <p className="mt-0.5 text-micro leading-4 text-slate-300 dark:text-slate-600">
                后端落盘后，重启也能回放每次会话的完整流水与耗时。
              </p>
            </div>
          </section>
        </div>

        {/* 右栏：流水 */}
        <div className="col-span-3 flex min-h-0 flex-col rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
          {selectedId ? (
            <>
              {/* 会话头 */}
              <div className="flex items-center gap-2 border-b border-slate-100 dark:border-slate-800 px-4 py-2.5">
                {selectedAlive
                  ? (() => {
                      const Icon = kindIcon(kindFromLabel(active.label))
                      return <Icon size={14} className="shrink-0 text-blue-500" />
                    })()
                  : selectedEnded && (selectedEnded.status === 'failed' ? (
                      <X size={14} className="shrink-0 text-red-500" />
                    ) : (
                      <Activity size={14} className="shrink-0 text-emerald-500" />
                    ))}
                <span className="min-w-0 flex-1 truncate text-[13px] font-bold text-slate-800 dark:text-slate-100">
                  {selectedAlive ? active.label : selectedEnded?.label ?? selectedId}
                </span>
                <span className="mono text-micro text-slate-400 dark:text-slate-500">{selectedId}</span>
                {selectedAlive && (
                  <>
                    <span className="tnum text-micro text-slate-400 dark:text-slate-500">
                      {elapsed !== null ? elapsed : '…'} · {rate} 行/分
                    </span>
                    <button
                      onClick={kill}
                      className="flex items-center gap-1 rounded-md border border-red-200 dark:border-red-900/60 bg-red-50 dark:bg-red-950/40 px-2 py-1 text-micro font-semibold text-red-600 hover:bg-red-100 dark:text-red-300 dark:hover:bg-red-900/40"
                      title="终止该会话（二次确认）"
                    >
                      <Square size={9} /> 终止
                    </button>
                  </>
                )}
                {selectedEnded && (
                  <span className={`rounded-full px-1.5 py-px text-micro font-semibold ${selectedEnded.status === 'failed' ? 'bg-red-50 dark:bg-red-950/40 text-red-500' : 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-600'}`}>
                    {selectedEnded.status === 'failed' ? '失败' : '成功'}
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
                        {selectedAlive && (
                          <>
                            <Loader2 size={11} className="animate-spin text-blue-500" />
                            <span className="tnum">已运行 {elapsed ?? '…'}</span>
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
                            <span className="min-w-0 flex-1 truncate text-[12px] text-slate-500 dark:text-slate-400">
                              {b.lines[0] || '（空）'}
                            </span>
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
                  {selectedAlive && (
                    <p className="flex items-center gap-1.5 pl-1 text-micro text-slate-400 dark:text-slate-500">
                      <Loader2 size={10} className="animate-spin text-blue-500" /> 正在输出…
                    </p>
                  )}
                  {blocks.length === 0 && !selectedAlive && (
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
                    {selectedLines.slice(-200).map(({ line }, i) => (
                      <div key={i} className={line.startsWith('[err]') ? 'text-red-400' : line.includes('[EASYVIBE-RESULT]') ? 'text-emerald-400' : undefined}>
                        {line}
                      </div>
                    ))}
                    {selectedLines.length === 0 && <span className="text-slate-500"># 等待输出…</span>}
                  </pre>
                </div>
              )}
            </>
          ) : (
            <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6">
              {isEmptyState(snap) ? (
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
