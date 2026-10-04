import { useState } from 'react'
import { ChatPanel } from '@/components/ChatPanel'
import type { CodeMap } from '@/types/map'
import type { Selection } from '@/components/DetailPanel'
import type { TaskDraft } from '@/lib/taskContext'

// 右栏「对话」页签实体化（2026-10-04——替代「已迁至任务对话」占位死胡同）：
// 就地挂载全功能 ChatPanel（多会话/附件/@模块/打断/Markdown，与工作台共享服务端会话库，
// 此处发起的会话在工作台列表可见，反之亦然）。
// 选中模块/子模块时自动钉 @提及（nonce 一次性消费、同 id 去重）；新建会话默认标题带对象名。
export function PanelChat({
  backendRepo,
  map,
  selection,
  onCreateTask,
  onLocateModule,
}: {
  backendRepo: string | null
  map: CodeMap
  selection: Selection
  onCreateTask: (draft: TaskDraft) => void
  onLocateModule: (moduleId: string) => void
}) {
  const mod =
    selection?.kind === 'module'
      ? (map.modules.find((m) => m.id === selection.id) ?? null)
      : selection?.kind === 'submodule'
        ? (map.modules.find((m) => m.id === selection.parentId) ?? null)
        : null
  // 选中对象变化 → 重新生成 @提及（渲染期派生态：官方推荐的 adjust-state-during-render 模式，
  // 避免 setState-in-effect 级联渲染告警；nonce 保证 ChatPanel 每次消费一次）
  const modKey = mod ? `${mod.id}:${mod.name}` : null
  const [mentionState, setMentionState] = useState<{ key: string; value: { id: string; name: string; nonce: number } } | null>(null)
  if (modKey !== null && mentionState?.key !== modKey) {
    setMentionState((prev) => ({ key: modKey, value: { id: mod!.id, name: mod!.name, nonce: (prev?.value.nonce ?? 0) + 1 } }))
  }

  return (
    <ChatPanel
      backendRepo={backendRepo}
      map={map}
      onLocateModule={onLocateModule}
      onCreateTask={onCreateTask}
      pendingMention={mentionState?.value ?? null}
      defaultConvTitle={mod ? `模块 · ${mod.name}` : null}
    />
  )
}
