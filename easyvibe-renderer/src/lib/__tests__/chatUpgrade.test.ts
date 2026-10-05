import { describe, expect, it } from 'vitest'
import { buildTaskDraftFromChat } from '@/components/chat/chatUpgrade'

describe('buildTaskDraftFromChat（对话→任务升级纯逻辑）', () => {
  it('空对话 / 无用户消息 → null（不产出草稿）', () => {
    expect(buildTaskDraftFromChat([], 'conv-1')).toBeNull()
    expect(buildTaskDraftFromChat([{ role: 'assistant', content: '答', refs: [] }], 'conv-1')).toBeNull()
    expect(buildTaskDraftFromChat([{ role: 'system', content: 'trace', refs: [] }], 'conv-1')).toBeNull()
  })

  it('取最近一条用户问句作标题与描述，携带会话 id', () => {
    const draft = buildTaskDraftFromChat(
      [
        { role: 'user', content: '早期问题', refs: [] },
        { role: 'assistant', content: '早期回答', refs: [] },
        { role: 'user', content: '最近的问题：如何修复 app-shell 的循环依赖？', refs: ['app-shell'] },
        { role: 'assistant', content: '建议拆分 hooks', refs: ['app-shell', 'ai-stream-hooks'] },
      ],
      'conv-9',
    )
    expect(draft).not.toBeNull()
    expect(draft!.title).toBe('对话：最近的问题：如何修复 app-s')
    expect(draft!.description.startsWith('最近的问题：如何修复 app-shell 的循环依赖？')).toBe(true)
    expect(draft!.conversation_id).toBe('conv-9')
    expect(draft!.source).toBe('manual')
  })

  it('refs 去重且截断到 5 个；摘要只含问答、不含 system、取最近 6 轮', () => {
    const msgs = [
      { role: 'system' as const, content: '上下文已压缩：82%→34%', refs: [] },
      { role: 'user' as const, content: 'u1', refs: ['a', 'b'] },
      { role: 'assistant' as const, content: 'a1', refs: ['a', 'c'] },
      { role: 'user' as const, content: 'u2', refs: ['d', 'e'] },
      { role: 'assistant' as const, content: 'a2', refs: ['f', 'g'] },
    ]
    const draft = buildTaskDraftFromChat(msgs, null)
    expect(draft!.modules).toEqual(['a', 'b', 'c', 'd', 'e'])
    expect(draft!.conversation_id).toBeUndefined()
    const conv = (draft!.context.inject as { conversation: string }).conversation
    expect(conv).not.toContain('上下文已压缩')
    expect(conv.split('\n').length).toBeLessThanOrEqual(6)
    expect(conv).toContain('问：u2')
  })
})
