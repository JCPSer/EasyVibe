// 五阶段映射（方案 v3 §4.1 + 2026-10-03 分阶段流扩展）：harness 五阶段落在状态机上。
// 渲染纪律：status 优先于 gate（failed 会残留旧 gate 值，不可按二元组直查——复审意见）。
// 分阶段执行流：plan → p:analysis(运行) → analysis(评审) → p:solution(运行) →
//   solution(评审) → p:implement(运行) → diff → report → done。
// 返回值：0-4 = 当前阶段索引（管道 now），'done' = 全部完成，'error' = 未启动/终态灰态。

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
    case 'failed':
    case 'rejected':
    case 'interrupted':
      return 'error'
    default:
      return 'error'
  }
}

/** 关卡人话标签（列表/管道用） */
export function gateLabel(status: string, gate: string | null | undefined): string | null {
  if (status !== 'awaiting_approval' && status !== 'running') return null
  switch (gate) {
    case 'plan': return '任务书审批'
    case 'p:analysis': return '分析中'
    case 'analysis': return '矩阵评审'
    case 'p:solution': return '方案中'
    case 'solution': return '方案评审'
    case 'diff': return 'Diff 审批'
    case 'report': return '审查报告'
    default: return status === 'running' ? '实施中' : null
  }
}

export const STAGES = [
  { key: 'analysis', label: '需求分析', hint: '需求矩阵 · 需评审' },
  { key: 'solution', label: '方案设计', hint: '方案文档 · 需评审' },
  { key: 'implement', label: '实施', hint: '实时执行' },
  { key: 'review', label: '代码审查', hint: 'Diff 审批' },
  { key: 'archive', label: '归档', hint: '审查报告 · STAR 记忆' },
] as const
