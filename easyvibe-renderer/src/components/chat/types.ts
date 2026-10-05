// 对话域共享类型（ChatPanel / QuickAsk 数据契约镜像的单一事实源）。
// 拆自 ChatPanel.tsx / QuickAsk.tsx（2026-10-05 防膨胀）。
import type { TaskDraft } from '@/lib/taskContext'
import type { CodeMap } from '@/types/map'

export interface ChatMessage {
  role: 'user' | 'assistant' | 'system'
  content: string
  refs: string[]
  /** 库消息 id（R1 分页游标） */
  id?: number
  /** 图片附件（dataURL；仅当前会话内存，刷新后不还原） */
  images?: { name: string; dataUrl: string }[]
  /** D9 @模块：本条用户消息显式钉住的模块 */
  mentions?: { id: string; name: string }[]
}

export interface Clarify {
  question: string
  options: { label: string; desc?: string }[]
  why?: string
}

/** M4-2 多会话：AionUI 运行时摘要精简版（后端 conversation_summary 产出） */
export interface ConversationSummary {
  id: string
  title: string | null
  repo: string
  updatedAt: string
  messageCount: number
  usage: { promptTokens: number; completionTokens: number }
  runtime: { state: 'idle' | 'running' | 'waiting_confirmation'; pendingConfirmations: number; runningTasks: number }
}

export interface PendingApproval {
  taskId: string
  title: string
  gate: string | null
}

export const GATE_LABEL: Record<string, string> = { plan: '① 计划审批', diff: '② Diff 审批', report: '③ 审查报告' }

export interface ChatRestore {
  summary: string | null
  conversation?: { id: string; title: string | null }
  messages: { id: number; role: string; content: string }[]
  hasMore: boolean
  pendingApprovals?: PendingApproval[]
  usage: { promptTokens: number; completionTokens: number }
}

/** 两形态对话组件的公共 props 基础（形态各自追加） */
export interface ChatCommonProps {
  backendRepo: string | null
  map: CodeMap | null
  onCreateTask: (draft: TaskDraft) => void
  onLocateModule: (moduleId: string) => void
  pendingMention?: { id: string; name: string; nonce: number } | null
  defaultConvTitle?: string | null
}
