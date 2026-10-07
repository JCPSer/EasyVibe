// 五阶段映射（方案 v3 §4.1 + 2026-10-03 分阶段流扩展）：harness 五阶段落在状态机上。
// 渲染纪律：status 优先于 gate（failed 会残留旧 gate 值，不可按二元组直查——复审意见）。
// 分阶段执行流：plan → p:analysis(运行) → analysis(评审) → p:solution(运行) →
//   solution(评审) → p:implement(运行) → diff → report → done。
// 返回值：0-4 = 当前阶段索引（管道 now），'done' = 全部完成，'error' = 未启动/终态灰态。
// i18n 第三批：人话标签渲染期经模块级 t 自译（枚举值为后端契约，不随语言变）。
import { t } from '@/runtime/i18n'

export type TaskStage = 0 | 1 | 2 | 3 | 4 | 'done' | 'error'

export function stageOf(status: string, gate: string | null | undefined): TaskStage {
  switch (status) {
    case 'awaiting_approval':
      // failed 可能残留 gate=diff——status 优先已挡在上面；这里只认活动审批态
      if (gate === 'plan' || gate === 'analysis') return 0 // ①需求分析（任务书批准 / 矩阵评审）
      if (gate === 'solution') return 1 // ②方案设计（方案评审）
      if (gate === 'diff') return 3
      if (gate === 'report') return 4
      return 'error' // 未知 gate 兜底灰态，不制造假阶段
    case 'running':
      // 分阶段运行标记：p:analysis=阶段1 产矩阵、p:solution=阶段2 产方案、其余=实施
      if (gate === 'p:analysis') return 0
      if (gate === 'p:solution') return 1
      return 2 // 实施中：①②标 done 直通（auto/supervised 单次 spawn 同理）
    case 'done':
      return 'done'
    case 'pending':
      // 2026-10-05 实弹：写互斥退回 pending 但 gate 保留 p: 阶段——
      // 归对应阶段（呈现"该阶段排队等待"，不再掉灰态让文档卡消失）
      if (gate === 'p:analysis') return 0
      if (gate === 'p:solution') return 1
      if (gate === 'p:implement') return 2
      return 'error'
    case 'failed':
    case 'rejected':
    case 'interrupted':
      return 'error'
    default:
      return 'error'
  }
}

/** 关卡人话标签（列表/管道用；渲染期 t 自译，随语言切换刷新） */
export function gateLabel(status: string, gate: string | null | undefined): string | null {
  if (status !== 'awaiting_approval' && status !== 'running') return null
  switch (gate) {
    case 'plan': return t('task.gate.plan')
    case 'p:analysis': return t('task.gate.pAnalysis')
    case 'analysis': return t('task.gate.analysis')
    case 'p:solution': return t('task.gate.pSolution')
    case 'solution': return t('task.gate.solution')
    case 'diff': return t('task.gate.diff')
    case 'report': return t('task.gate.report')
    default: return status === 'running' ? t('task.gate.running') : null
  }
}

/** 五阶段管道数据源：label/hint 只存字典 key，渲染期 t(`task.stage.*`) / t(`task.stageHint.*`) 解析。 */
export const STAGES: readonly { key: 'analysis' | 'solution' | 'implement' | 'review' | 'archive'; labelKey: string; hintKey: string }[] = [
  { key: 'analysis', labelKey: 'task.stage.analysis', hintKey: 'task.stageHint.analysis' },
  { key: 'solution', labelKey: 'task.stage.solution', hintKey: 'task.stageHint.solution' },
  { key: 'implement', labelKey: 'task.stage.implement', hintKey: 'task.stageHint.implement' },
  { key: 'review', labelKey: 'task.stage.review', hintKey: 'task.stageHint.review' },
  { key: 'archive', labelKey: 'task.stage.archive', hintKey: 'task.stageHint.archive' },
]
