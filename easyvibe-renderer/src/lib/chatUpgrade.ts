import type { TaskDraft } from '@/lib/taskContext'

// S1 对话升级任务的纯逻辑（QuickAsk 与旧 ChatPanel 同一契约的共享实现）：
// 最近用户问句 + 近 6 轮问答摘要 + 引用模块（去重，≤5）→ TaskDraft（表单可再编辑）
export interface UpgradeChatMessage {
  role: 'user' | 'assistant' | 'system'
  content: string
  refs: string[]
}

export function buildTaskDraftFromChat(messages: UpgradeChatMessage[], convId: string | null): TaskDraft | null {
  const turns = messages.filter((m) => m.role !== 'system')
  const lastUser = [...turns].reverse().find((m) => m.role === 'user')
  if (!lastUser) return null
  const refs = [...new Set(turns.flatMap((m) => m.refs))].slice(0, 5)
  const summary = turns
    .slice(-6)
    .map((m) => `${m.role === 'user' ? '问' : '答'}：${m.content.slice(0, 120)}`)
    .join('\n')
  return {
    title: `对话：${lastUser.content.slice(0, 16)}`,
    description: `${lastUser.content}\n\n—— 来自对话的已澄清需求，见上下文中的对话摘要。`,
    modules: refs,
    acceptance: '',
    source: 'manual',
    context: { inject: { conversation: summary, refs } },
    conversation_id: convId ?? undefined,
  }
}
