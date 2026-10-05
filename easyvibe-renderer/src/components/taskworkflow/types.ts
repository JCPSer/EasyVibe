// TaskWorkflowPage 拆分产物：任务/审批/产物文档类型与状态标签。
// 拆自 TaskWorkflowPage.tsx（2026-10-05 防膨胀）。

export interface TaskItem {
  id: string
  title: string
  description: string
  status: string
  gate: string | null
  trust: string
  sessionId?: string | null
  modules?: string[]
  acceptance?: string
  error?: string | null
  createdAt?: string
  updatedAt?: string
  result?: {
    diffStat?: string
    contractViolations?: string[]
    warnings?: string[]
    review?: { verdict: string; summary: string; at?: number }
    /** 阶段初审结论（2026-10-03）：key = analysis（需求矩阵）/ solution（方案设计） */
    phaseReviews?: Record<string, { verdict: string; summary: string }>
  } | null
}

export interface Approval {
  id: string
  gate: string
  decision: string // approved / rejected / skipped / flagged
  note: string | null
  decidedAt: string
}

export interface DevDoc {
  name: string
  path: string
  mtime: number
  excerpt: string
}

export const STATUS_LABEL: Record<string, string> = {
  pending: '排队中',
  running: '执行中',
  awaiting_approval: '等待审批',
  done: '已完成',
  failed: '失败',
  rejected: '已驳回',
  interrupted: '已中断',
}
