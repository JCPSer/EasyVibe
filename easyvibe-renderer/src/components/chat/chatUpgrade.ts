import { t } from '@/runtime/i18n'
import type { TaskDraft } from '@/shared/logic/taskContext'

// S1 对话升级任务的纯逻辑（QuickAsk 与旧 ChatPanel 同一契约的共享实现）：
// 最近用户问句 + 近 6 轮问答摘要 + 引用模块（去重，≤5）→ TaskDraft（表单可再编辑）
// 任务标题/描述是用户可见文案（表单预填 + 任务列表展示），经模块级 t 自译。
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
    .map((m) => `${t(m.role === 'user' ? 'chat.summaryAsk' : 'chat.summaryAnswer')}${m.content.slice(0, 120)}`)
    .join('\n')
  return {
    title: t('chat.taskTitle', { text: lastUser.content.slice(0, 16) }),
    description: t('chat.taskDesc', { text: lastUser.content }),
    modules: refs,
    acceptance: '',
    source: 'manual',
    context: { inject: { conversation: summary, refs } },
    conversation_id: convId ?? undefined,
  }
}
