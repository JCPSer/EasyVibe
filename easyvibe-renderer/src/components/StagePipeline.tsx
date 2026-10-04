import { CheckCircle2 } from 'lucide-react'
import { stageOf, STAGES } from '@/lib/taskStage'

// 五阶段管道公共组件（v4 brief §8：两处五阶段渲染漂移的对策——判定只在 taskStage 一处，
// 这里只负责画）。compact = 工作台迷你档（无 hint 行）；detail = 任务页标准档。

type DotState = 'done' | 'now' | 'todo'

export function StagePipeline({
  status,
  gate,
  variant = 'detail',
}: {
  status: string
  gate: string | null | undefined
  variant?: 'detail' | 'compact'
}) {
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

  if (variant === 'compact') {
    return (
      <div className="flex items-center gap-1">
        {STAGES.map((s, i) => (
          <div key={s.key} className="flex flex-1 items-center gap-1">
            <span className={`flex h-4 w-4 shrink-0 items-center justify-center rounded-full text-micro font-bold ${cls(dotState(i))}`}>
              {dotState(i) === 'done' ? <CheckCircle2 size={10} /> : i + 1}
            </span>
            <span className={`truncate text-micro ${dotState(i) === 'todo' ? 'text-slate-400 dark:text-slate-500' : 'font-semibold text-slate-600 dark:text-slate-300'}`}>{s.label}</span>
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
      {STAGES.map((s, i) => (
        <div key={s.key} className="flex flex-1 items-center gap-1">
          <span className={`flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-micro font-bold ${cls(dotState(i))}`}>
            {dotState(i) === 'done' ? <CheckCircle2 size={11} /> : i + 1}
          </span>
          <span className="min-w-0">
            <span className="block truncate text-micro font-semibold text-slate-600 dark:text-slate-300">{s.label}</span>
            <span className="block truncate text-[9px] text-slate-400 dark:text-slate-500">{stage === 'error' ? '—' : s.hint}</span>
          </span>
          {i < STAGES.length - 1 && (
            <span className={`h-px flex-1 ${dotState(i) === 'done' ? 'bg-emerald-400' : 'bg-slate-200'}`} />
          )}
        </div>
      ))}
    </div>
  )
}
