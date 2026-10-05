// 管道回看：历史阶段产物的只读视图（0/1 → PhaseDocReview readonly + 重开入口；
// 2/3/4 → 只读变更统计摘要）。拆自 TaskWorkflowPage.tsx（2026-10-05 防膨胀）。
import { History } from 'lucide-react'
import { STAGES } from '@/lib/taskStage'
import { PhaseDocReview } from './PhaseDocReview'
import type { TaskItem } from './types'

/** 管道回看（2026-10-05 方案 §3.2）：历史阶段产物的只读视图。
 *  0/1（需求分析/方案设计）→ PhaseDocReview readonly + 「回到此关」重开入口；
 *  2/3/4（实施/代码审查/归档）→ 变更统计等只读摘要（完整 Diff/报告随当前进度卡片）。 */
export function StageLookback({
  backendRepo,
  task,
  look,
  impact,
  onRewind,
  rewinding,
}: {
  backendRepo: string
  task: TaskItem
  /** 回看的目标阶段索引（< 当前阶段） */
  look: number
  impact: { adds: number; dels: number; files: number }[]
  onRewind: (gate: 'analysis' | 'solution') => void
  rewinding: 'analysis' | 'solution' | null
}) {
  // 重开资格与后端 rewind() 前置一致：running/pending 须先等终态；auto 不支持
  const rewindable =
    task.trust !== 'auto' && ['awaiting_approval', 'failed', 'interrupted', 'rejected', 'done'].includes(task.status)
  if (look === 0 || look === 1) {
    const isAnalysis = look === 0
    return (
      <PhaseDocReview
        key={`${task.id}-look-${look}`}
        backendRepo={backendRepo}
        taskId={task.id}
        dirHint={isAnalysis ? '1_requirements_matrix' : '2_requirements_solutions'}
        title={isAnalysis ? '需求矩阵（回看）' : '方案设计（回看）'}
        deciding={null}
        review={isAnalysis ? (task.result?.phaseReviews?.analysis ?? null) : (task.result?.phaseReviews?.solution ?? null)}
        {...(!isAnalysis ? { compareDirHint: '1_requirements_matrix', compareTitle: '需求矩阵（已评审）' } : {})}
        readonly
        rewinding={!!rewinding}
        onRewind={rewindable ? () => onRewind(isAnalysis ? 'analysis' : 'solution') : undefined}
      />
    )
  }
  return (
    <div className="m-4 overflow-y-auto rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
      <p className="flex items-center gap-1 text-micro font-bold uppercase tracking-wider text-violet-500 dark:text-violet-400">
        <History size={10} /> {STAGES[look]?.label ?? look} · 回看
      </p>
      <div className="mt-2 space-y-1.5 text-[12px] leading-5 text-slate-700 dark:text-slate-200">
        {impact.map((im, i) => (
          <p key={i} className="tnum text-micro text-slate-500 dark:text-slate-400">
            本次变更：<span className="text-emerald-600">+{im.adds}</span> <span className="text-red-500">−{im.dels}</span> · {im.files} 文件
          </p>
        ))}
        <p className="text-micro text-slate-400 dark:text-slate-500">
          此阶段的完整产物（{look === 2 ? '实时执行与产物文档' : look === 3 ? 'Diff 与审查意见' : '审查报告与归档路径'}）随当前进度的对应卡片展示；
          如需从更早阶段重开流水线，点击管道上的「需求分析」或「方案设计」回看其文档。
        </p>
      </div>
    </div>
  )
}
