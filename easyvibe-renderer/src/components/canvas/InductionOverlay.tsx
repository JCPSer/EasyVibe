import { useCallback, useEffect, useRef, useState } from 'react'
import { Clock, X } from 'lucide-react'
import { onSessionOutput } from '@/runtime/growthBus'
import { prefersReducedMotion } from '@/runtime/motion'
import {
  formatTickerLine,
  interpretInductionProgress,
  type InductionPhaseView,
  type InductionProgress,
} from '@/shared/logic/inductionProgress'

// 归纳期间地图区域"原位过场"：罩层 + agent 实时输出行（ticker）+ 阶段进度卡（底部中央，
// 接管 GrowthPanel"等待生长事件"的空态位置）/ 排队态卡。高频状态全部留在本组件内部
// （useInductionLive），不上提 Canvas；暗色一律 dark: CSS 变体，不读 Canvas 的 dark prop。

export interface InductionSessionRef {
  sessionId: string
  label: string
}

const TICKER_MAX_CHARS = 90
const TICKER_KEEP_LINES = 20 // 内存留最近 20 行，滚动取句
const SILENT_MS = 30_000 // agent 静默阈值：ticker 降显 + "（agent 静默中）"

/**
 * 过场实时数据：progress.json 2s 轮询（排队时不轮）+ session.output WS 直播 + 首挂补拉。
 * enabled=false 时全部静默。
 */
function useInductionLive(repo: string | null, session: InductionSessionRef | null, enabled: boolean) {
  const [view, setView] = useState<InductionPhaseView>(() => interpretInductionProgress(null))
  const [ticker, setTicker] = useState<{ text: string; silent: boolean }>({ text: '', silent: false })

  // progress.json → 阶段卡（内容变化才 setState；陈旧 done/缺失的解读在 interpretInductionProgress）
  useEffect(() => {
    if (!enabled || !repo) return
    let stale = false
    let last = ''
    const tick = () => {
      fetch(`/api/repos/${encodeURIComponent(repo)}/progress`)
        .then((r) => (r.ok ? r.json() : null))
        .then((d: { data: InductionProgress | null } | null) => {
          if (stale) return
          const next = JSON.stringify(d?.data ?? null)
          if (next === last) return
          last = next
          setView(interpretInductionProgress(d?.data ?? null))
        })
        .catch(() => {})
    }
    tick()
    const t = window.setInterval(tick, 2000)
    return () => {
      stale = true
      window.clearInterval(t)
    }
  }, [enabled, repo])

  // agent 输出行：WS 直播（按 seq 幂等去重）+ 首挂补拉（接住订阅前已产出的行）
  const linesRef = useRef<string[]>([])
  const seenRef = useRef<Set<number>>(new Set())
  const silentTimerRef = useRef<number | null>(null)
  const sessionId = session?.sessionId ?? null

  const armSilentTimer = useCallback(() => {
    if (silentTimerRef.current) window.clearTimeout(silentTimerRef.current)
    silentTimerRef.current = window.setTimeout(() => setTicker((t) => ({ ...t, silent: true })), SILENT_MS)
  }, [])

  const pushLine = useCallback(
    (seq: number, stream: string, line: string) => {
      if (seq >= 0) {
        if (seenRef.current.has(seq)) return
        seenRef.current.add(seq)
      }
      const text = formatTickerLine(line, stream)
      if (!text) return
      const lines = linesRef.current
      lines.push(text)
      if (lines.length > TICKER_KEEP_LINES) lines.splice(0, lines.length - TICKER_KEEP_LINES)
      setTicker({ text: lines[lines.length - 1], silent: false })
      armSilentTimer()
    },
    [armSilentTimer],
  )

  useEffect(() => {
    linesRef.current = []
    seenRef.current = new Set()
    setTicker({ text: '', silent: false })
    if (silentTimerRef.current) window.clearTimeout(silentTimerRef.current)
    if (!enabled || !repo || !sessionId) return
    armSilentTimer()
    fetch(
      `/api/repos/${encodeURIComponent(repo)}/sessions/${encodeURIComponent(sessionId)}/output?afterSeq=0&limit=5000`,
    )
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { seq: number; stream: string; line: string }[] } | null) => {
        for (const row of d?.data ?? []) pushLine(row.seq, row.stream, row.line)
      })
      .catch(() => {})
    const off = onSessionOutput((e) => {
      if (e.sessionId !== sessionId) return
      pushLine(e.seq, e.stream, e.line)
    })
    return () => {
      off()
      if (silentTimerRef.current) window.clearTimeout(silentTimerRef.current)
    }
  }, [enabled, repo, sessionId, pushLine, armSilentTimer])

  return { view, ticker }
}

function truncate(s: string, n: number) {
  return s.length > n ? `${s.slice(0, n)}…` : s
}

// 慢速细环转圈（SessionBubble 同款语言：2.4s、细弧，不用 Loader2 快转）
function SlowRing() {
  return (
    <svg className="absolute inset-0 animate-spin" style={{ animationDuration: '2.4s' }} viewBox="0 0 22 22" aria-hidden>
      <circle cx="11" cy="11" r="9" fill="none" strokeWidth="1.5" className="stroke-indigo-100 dark:stroke-slate-700" />
      <circle
        cx="11"
        cy="11"
        r="9"
        fill="none"
        stroke="#818cf8"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeDasharray="12 45"
      />
    </svg>
  )
}

/**
 * 过场罩层。active=false 后 300ms 淡出再卸载（本页会话首事件到达 → 生长直播接管的过渡）；
 * 从未激活过则不渲染。罩层 pointer-events-none（Controls/MiniMap 保持可用），
 * 卡面与 ticker 单独 pointer-events-auto。
 */
export function InductionOverlay({
  repo,
  session,
  queued,
  active,
  onOpenRuns,
  onCancelQueue,
}: {
  repo: string | null
  session: InductionSessionRef | null
  /** 排队态：琥珀语义卡（时钟 + 取消），不轮 progress.json */
  queued: boolean
  /** 过场显示中；false 触发 300ms 淡出 */
  active: boolean
  /** ticker 点击 → 跳运行页看完整流水（不要打开右栏对话页签） */
  onOpenRuns?: (sessionId: string) => void
  onCancelQueue?: () => void
}) {
  const [leaving, setLeaving] = useState(false)
  const everShownRef = useRef(false)
  useEffect(() => {
    if (active) {
      everShownRef.current = true
      setLeaving(false)
      return
    }
    if (!everShownRef.current) return
    const t = window.setTimeout(() => setLeaving(true), prefersReducedMotion() ? 0 : 300)
    return () => window.clearTimeout(t)
  }, [active])
  // 归纳结束抬罩后复位，下一次归纳重新淡入
  useEffect(() => {
    if (!leaving) return
    const t = window.setTimeout(() => {
      everShownRef.current = false
      setLeaving(false)
    }, 50)
    return () => window.clearTimeout(t)
  }, [leaving])

  const live = useInductionLive(repo, session, active && !queued)

  if (!everShownRef.current || leaving) return null

  const tickerText = truncate(live.ticker.text, TICKER_MAX_CHARS)
  const tickerClickable = !!session && !!onOpenRuns
  return (
    <div className="anim-fade-in-fast pointer-events-none absolute inset-0 z-[3]">
      {/* 留守旧地图的暗场罩层：不拦截指针，节点交互由 Canvas 侧拦 */}
      <div className="absolute inset-0 bg-slate-100/70 dark:bg-slate-950/60" />

      {/* 底部中央：ticker（紧贴进度卡上方）+ 阶段进度卡 / 排队卡 */}
      <div className="absolute inset-x-0 bottom-10 flex flex-col items-center gap-2 px-4">
        {!queued && tickerText && (
          <button
            onClick={tickerClickable ? () => onOpenRuns!(session!.sessionId) : undefined}
            disabled={!tickerClickable}
            title={tickerClickable ? '查看完整流水' : undefined}
            className={`pointer-events-auto max-w-[min(460px,90%)] truncate rounded-full border border-slate-200 dark:border-slate-700 bg-white/95 dark:bg-slate-900/95 px-3 py-1 text-left font-mono text-micro text-slate-500 shadow-sm backdrop-blur anim-fade-in-fast dark:text-slate-400 ${
              live.ticker.silent ? 'opacity-50' : ''
            } ${tickerClickable ? 'hover:border-blue-300 dark:hover:border-blue-700' : ''}`}
          >
            {tickerText}
            {live.ticker.silent && <span className="font-sans text-slate-400 dark:text-slate-500">（agent 静默中）</span>}
          </button>
        )}

        <div className="pointer-events-auto w-[min(460px,90%)] rounded-xl border border-slate-200 bg-white/95 px-4 py-3 shadow-sm backdrop-blur dark:border-slate-700 dark:bg-slate-900/95">
          {queued ? (
            <div className="flex items-center gap-2.5">
              <Clock size={15} className="shrink-0 text-amber-500" />
              <div className="min-w-0 flex-1">
                <p className="text-[12px] font-semibold text-amber-700 dark:text-amber-400">归纳排队中</p>
                <p className="mt-0.5 truncate text-micro text-slate-400 dark:text-slate-500">
                  当前会话结束后自动开始
                </p>
              </div>
              {onCancelQueue && (
                <button
                  onClick={onCancelQueue}
                  className="shrink-0 rounded-full p-1.5 text-slate-400 hover:bg-slate-100 hover:text-slate-600 dark:text-slate-500 dark:hover:bg-slate-700/70 dark:hover:text-slate-300"
                  title="取消排队"
                >
                  <X size={13} />
                </button>
              )}
            </div>
          ) : (
            <div className="flex items-center gap-3">
              <span className="relative h-[22px] w-[22px] shrink-0">
                <SlowRing />
              </span>
              <div className="min-w-0 flex-1">
                <div className="flex items-baseline justify-between gap-2">
                  <span className="truncate text-[12px] font-semibold text-slate-700 dark:text-slate-200">
                    {live.view.title}
                  </span>
                  {live.view.percent !== null && (
                    <span className="shrink-0 tabular-nums text-micro text-slate-400 dark:text-slate-500">
                      {live.view.percent}%
                    </span>
                  )}
                </div>
                {live.view.percent !== null && (
                  <div
                    className="mt-1 h-1 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800"
                    role="progressbar"
                    aria-valuenow={live.view.percent}
                    aria-valuemin={0}
                    aria-valuemax={100}
                  >
                    <div
                      className="h-full rounded-full bg-blue-500 transition-[width] duration-500"
                      style={{ width: `${live.view.percent}%` }}
                    />
                  </div>
                )}
                <p className="mt-1 truncate text-micro text-slate-400 dark:text-slate-500">{live.view.subline}</p>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
