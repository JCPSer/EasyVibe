import { useMemo, useRef, useState } from 'react'
import {
  Activity,
  ChevronDown,
  ChevronRight,
  LayoutList,
  ListTree,
  Loader2,
  Square,
  Terminal,
} from 'lucide-react'
import { useLang } from '@/runtime/i18n'

// 运行页右栏流水视图（英文化第二批从 RunsPage 抽出：RunsPage 贴 componentGuard LEGACY 667 红线，
// t() 迁移净增行数，抽出后两边都回到阈值内；新增文件已登记守卫）。
// 三档呈现：进展卡 / 时间线 / 终端；分块与时间格式化口径与抽离前逐处一致。

export interface StreamLine {
  seq: number
  stream: string
  line: string
  t: number
}

export type Tier = 'card' | 'timeline' | 'terminal'
export const TIERS: { id: Tier; labelKey: string; icon: typeof Terminal }[] = [
  { id: 'card', labelKey: 'pages.runs.tierCard', icon: LayoutList },
  { id: 'timeline', labelKey: 'pages.runs.tierTimeline', icon: ListTree },
  { id: 'terminal', labelKey: 'pages.runs.tierTerminal', icon: Terminal },
]

/** 60s 滑窗行频 */
export function linesPerMinute(lines: StreamLine[], now: number): number {
  const cutoff = now - 60_000
  let n = 0
  for (let i = lines.length - 1; i >= 0; i--) {
    if (lines[i].t < cutoff) break
    n++
  }
  return n
}

/** 时间线分段：思考块(可折叠)/err/result/文本 */
type Block = { kind: 'think' | 'text' | 'err' | 'result'; lines: string[] }

export function toBlocks(lines: StreamLine[]): Block[] {
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

/** 会话最后一条有效输出（进展卡与运行卡共用） */
export function lastTextOf(lines: StreamLine[]): string {
  for (let i = lines.length - 1; i >= 0; i--) {
    const l = lines[i].line
    if (l && !l.startsWith('[err]')) return l.startsWith('[思考]') ? l.slice(4).trim() : l
  }
  return ''
}

export function RunsStreamView({
  tier,
  selectedId,
  selectedLines,
  selectedMeta,
  elapsed,
  rate,
  empty,
  onKill,
}: {
  tier: Tier
  selectedId: string | null
  selectedLines: StreamLine[]
  selectedMeta: { label: string; status: 'alive' | 'succeeded' | 'failed' } | null
  elapsed: string | null
  rate: number
  /** 无任何会话（活动/排队/历史全空）→ 空态引导；否则提示从左侧选择 */
  empty: boolean
  /** 会话存活时的「终止」动作（undefined = 不渲染终止按钮） */
  onKill?: () => void
}) {
  const { t } = useLang()
  const [collapsed, setCollapsed] = useState<Set<number>>(new Set())
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const followRef = useRef(true)
  const blocks = useMemo(() => (tier === 'timeline' ? toBlocks(selectedLines) : []), [tier, selectedLines])
  const lastText = useMemo(() => lastTextOf(selectedLines), [selectedLines])

  if (!selectedId) {
    return (
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6">
        {empty ? (
          <>
            <Activity size={20} className="text-slate-300 dark:text-slate-600" />
            <p className="text-[12px] font-semibold text-slate-500 dark:text-slate-400">{t('pages.runs.allEmptyTitle')}</p>
            <p className="text-[11px] text-slate-400 dark:text-slate-500">{t('pages.runs.allEmptyHint')}</p>
          </>
        ) : (
          <p className="text-[12px] text-slate-400 dark:text-slate-500">{t('pages.runs.noSelection')}</p>
        )}
      </div>
    )
  }

  return (
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
          <span className="tnum text-micro text-slate-400 dark:text-slate-500">
            {elapsed !== null ? t('pages.runs.elapsedRate', { elapsed, count: rate }) : t('pages.runs.rate', { count: rate })}
          </span>
        )}
        {selectedMeta && selectedMeta.status !== 'alive' && (
          <span className={`rounded-full px-1.5 py-px text-micro font-semibold ${selectedMeta.status === 'failed' ? 'bg-red-50 dark:bg-red-950/40 text-red-500' : 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-600'}`}>
            {selectedMeta.status === 'failed' ? t('common.status.failed') : t('common.status.succeeded')}
          </span>
        )}
        {selectedMeta?.status === 'alive' && onKill && (
          <button
            onClick={onKill}
            className="flex items-center gap-1 rounded-md border border-red-200 dark:border-red-900/60 bg-red-50 dark:bg-red-950/40 px-2 py-1 text-micro font-semibold text-red-600 hover:bg-red-100 dark:text-red-300 dark:hover:bg-red-900/40"
            title={t('pages.runs.killTip')}
          >
            <Square size={9} /> {t('pages.runs.kill')}
          </button>
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
                    {elapsed !== null && <span className="tnum">{t('pages.runs.cardElapsed', { elapsed })}</span>}
                    <span>·</span>
                    <span className="tnum">{t('pages.runs.rate', { count: rate })}</span>
                  </>
                )}
                <span>·</span>
                <span className="tnum">{t('pages.runs.cardLines', { count: selectedLines.length })}</span>
              </div>
            </>
          ) : (
            <p className="text-[12px] text-slate-400 dark:text-slate-500">{t('pages.runs.cardWaiting')}</p>
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
                    <span className="shrink-0 rounded bg-slate-200 dark:bg-slate-700 px-1 py-px text-micro font-semibold text-slate-500 dark:text-slate-300">{t('pages.runs.think')}</span>
                    <span className="min-w-0 flex-1 truncate text-[12px] text-slate-500 dark:text-slate-400">{b.lines[0] || t('pages.runs.thinkEmpty')}</span>
                    <span className="tnum shrink-0 text-micro text-slate-300 dark:text-slate-600">{t('pages.runs.blockLines', { count: b.lines.length })}</span>
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
              <Loader2 size={10} className="animate-spin text-blue-500" /> {t('pages.runs.streaming')}
            </p>
          )}
          {blocks.length === 0 && selectedMeta?.status !== 'alive' && (
            <p className="py-8 text-center text-[11px] text-slate-300 dark:text-slate-600">{t('pages.runs.noOutput')}</p>
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
            {selectedLines.length === 0 && <span className="text-slate-500"># {t('pages.runs.cardWaiting')}</span>}
          </pre>
        </div>
      )}
    </>
  )
}
