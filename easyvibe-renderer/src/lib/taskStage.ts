// 五阶段映射（方案 v3 §4.1）：harness 五阶段落在现有 (status, gate) 状态机上，零状态机改动。
// 渲染纪律：status 优先于 gate（failed 会残留旧 gate 值，不可按二元组直查——复审意见）。
// 返回值：0-4 = 当前阶段索引（管道 now），'done' = 全部完成，'error' = 未启动/终态灰态。

export type TaskStage = 0 | 1 | 2 | 3 | 4 | 'done' | 'error'

export function stageOf(status: string, gate: string | null | undefined): TaskStage {
  switch (status) {
    case 'awaiting_approval':
      // failed 可能残留 gate=diff——status 优先已挡在上面；这里只认活动审批态
      if (gate === 'plan') return 0 // ①②同体（方案：计划关等待期①②同格点亮）
      if (gate === 'diff') return 3
      if (gate === 'report') return 4
      return 'error' // 未知 gate 兜底灰态，不制造假阶段
    case 'running':
      return 2 // 实施中：①②标 done（直通），整管只亮③（复审口径漏洞修复）
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

export const STAGES = [
  { key: 'analysis', label: '需求分析', hint: '需求矩阵 · 需评审' },
  { key: 'solution', label: '需求方案', hint: '方案文档 · 需评审' },
  { key: 'implement', label: '实施', hint: '实时执行' },
  { key: 'review', label: '代码审查', hint: 'Diff 审批' },
  { key: 'archive', label: '归档', hint: '审查报告 · STAR 记忆' },
] as const
