import { useState } from 'react'
import { useLang } from '@/runtime/i18n'
import { QuickAsk } from '@/components/chat/QuickAsk'
import type { CodeMap } from '@/types/map'
import type { Selection } from '@/shared/contract/selection'
import type { TaskDraft } from '@/shared/logic/taskContext'

// 右栏「对话」页签薄壳（2026-10-05 Redesign-A）：QuickAsk 检查器文档流的父级接线——
// 选中模块/子模块时投递 @提及（nonce 一次性消费、同 id 去重）+ 新建会话默认标题带对象名。
// 数据面（会话恢复/发送/审批/存视图）全部在 QuickAsk 内与工作台 ChatPanel 镜像同一契约。
export function PanelChat({
  backendRepo,
  map,
  selection,
  onCreateTask,
  onLocateModule,
  onGoWorkbench,
}: {
  backendRepo: string | null
  map: CodeMap
  selection: Selection
  onCreateTask: (draft: TaskDraft) => void
  onLocateModule: (moduleId: string) => void
  /** 审批出口：跳工作台「任务对话」页裁决 */
  onGoWorkbench?: () => void
}) {
  const { t } = useLang()
  const mod =
    selection?.kind === 'module'
      ? (map.modules.find((m) => m.id === selection.id) ?? null)
      : selection?.kind === 'submodule'
        ? (map.modules.find((m) => m.id === selection.parentId) ?? null)
        : null
  // 选中对象变化 → 重新生成 @提及（渲染期派生态：官方推荐的 adjust-state-during-render 模式，
  // 避免 setState-in-effect 级联渲染告警；nonce 保证 QuickAsk 每次消费一次）
  const modKey = mod ? `${mod.id}:${mod.name}` : null
  const [mentionState, setMentionState] = useState<{ key: string; value: { id: string; name: string; nonce: number } } | null>(null)
  if (modKey !== null && mentionState?.key !== modKey) {
    setMentionState((prev) => ({ key: modKey, value: { id: mod!.id, name: mod!.name, nonce: (prev?.value.nonce ?? 0) + 1 } }))
  }

  return (
    <QuickAsk
      backendRepo={backendRepo}
      map={map}
      selection={selection}
      onCreateTask={onCreateTask}
      onLocateModule={onLocateModule}
      onGoWorkbench={onGoWorkbench}
      pendingMention={mentionState?.value ?? null}
      defaultConvTitle={mod ? t('chat.panelTitle', { name: mod.name }) : null}
    />
  )
}
