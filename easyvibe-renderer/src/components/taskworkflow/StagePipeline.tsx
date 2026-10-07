import { CheckCircle2 } from 'lucide-react'
import { useLang } from '@/runtime/i18n'
import { stageOf, STAGES } from '@/components/taskworkflow/taskStage'

// 五阶段管道公共组件（v4 brief §8：两处五阶段渲染漂移的对策——判定只在 taskStage 一处，
// 这里只负责画）。compact = 工作台迷你档（无 hint 行）；detail = 任务页标准档。

type DotState = 'done' | 'now' | 'todo'

export function StagePipeline({
  status,
  gate,
  variant = 'detail',
  selected,
  onSelectStage,
}: {
  status: string
  gate: string | null | undefined
  variant?: 'detail' | 'compact'
  /** 2026-10-05 管道回看：当前查看的阶段（默认=当前阶段，无圈选不传） */
  selected?: number
  /** 点击早期阶段回看（仅 detail 档生效；未来阶段不可点） */
  onSelectStage?: (i: number) => void
}) {
  const { t } = useLang()
  const stage = stageOf(status, gate)
  const dotState = (i: number): DotState => {
    if (stage === 'error' || stage === null) return 'todo'
    if (stage === 'done') return 'done'
    if (stage === 0) return i <= 1 ? 'now' : 'todo' // ①②同格点亮（plan/analysis 皆属①）
    if (i < stage) return 'done'
    if (i === stage) return 'now'
    return 'todo'
  }
  const cls = (s: DotState) =>
    s === 'done' ? 'bg-emerald-500 text-white' : s === 'now' ? 'bg-blue-600 text-white' : 'bg-slate-100 dark:bg-slate-800 text-slate-400 dark:text-slate-500'

  // 可回看性：当前阶段及之前的阶段可点（'done' 全可点；error 灰态全不可点）
  const viewable = (i: number): boolean => {
    if (!onSelectStage) return false
    if (stage === 'done') return true
    if (typeof stage === 'number') return i <= stage
    return false
  }

  if (variant === 'compact') {
    return (
      <div className="flex items-center gap-1">
        {STAGES.map((s, i) => (
          <div key={s.key} className="flex flex-1 items-center gap-1">
            <span className={`flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-micro font-bold ${cls(dotState(i))}`}>
              {dotState(i) === 'done' ? <CheckCircle2 size={10} /> : i + 1}
            </span>
            <span className={`truncate text-micro ${dotState(i) === 'todo' ? 'text-slate-400 dark:text-slate-500' : 'font-semibold text-slate-600 dark:text-slate-300'}`}>{t(s.labelKey)}</span>
            {i < STAGES.length - 1 && (
              <span className={`h-px flex-1 ${dotState(i) === 'done' ? 'bg-emerald-400' : 'bg-slate-200'}`} />
            )}
          </div>
        ))}
      </div>
    )
  }

  return (
    <div className="flex items-center gap-1">
      {STAGES.map((s, i) => {
        const clickable = viewable(i)
        const ring = selected === i && stage !== i ? ' ring-2 ring-violet-400 dark:ring-violet-600 ring-offset-1 dark:ring-offset-slate-900 rounded-full' : ''
        return (
          <div key={s.key} className="flex flex-1 items-center gap-1">
            <button
              type="button"
              disabled={!clickable}
              onClick={() => onSelectStage?.(i)}
              title={clickable ? t('task.pipeLookbackTip', { stage: t(s.labelKey) }) : stage === 'done' ? t(s.labelKey) : t('task.pipeNotReached')}
              className={`flex items-center gap-1 rounded-full${ring} ${clickable ? 'cursor-pointer transition-transform hover:scale-105' : 'cursor-default'}`}
            >
              <span className={`flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-micro font-bold ${cls(dotState(i))}`}>
                {dotState(i) === 'done' ? <CheckCircle2 size={11} /> : i + 1}
              </span>
              <span className="min-w-0">
                <span className="block truncate text-micro font-semibold text-slate-600 dark:text-slate-300">{t(s.labelKey)}</span>
                <span className="block truncate text-[9px] text-slate-400 dark:text-slate-500">{stage === 'error' ? '—' : t(s.hintKey)}</span>
              </span>
            </button>
            {i < STAGES.length - 1 && (
              <span className={`h-px flex-1 ${dotState(i) === 'done' ? 'bg-emerald-400' : 'bg-slate-200'}`} />
            )}
          </div>
        )
      })}
    </div>
  )
}
