import type { CodeMap } from '@/types/map'
import type { TaskDraft } from '@/lib/taskContext'
import { TaskFormPanel } from '@/components/TaskFormPanel'

/** 全局任务表单浮层（地图/建议/工作区页共用） */
export function TaskDraftOverlay({
  backendRepo,
  taskDraft,
  draftSeq,
  map,
  agentReady,
  onClose,
  onCreated,
  onLocateModule,
}: {
  backendRepo: string | null
  taskDraft: TaskDraft
  draftSeq: number
  map: CodeMap
  agentReady: boolean
  onClose: () => void
  onCreated: (id: string) => void
  onLocateModule: () => void
}) {
  return (
    <TaskFormPanel
      key={`app-draft-${draftSeq}`}
      backendRepo={backendRepo}
      draft={taskDraft}
      map={map}
      onClose={onClose}
      onCreated={onCreated}
      onLocateModule={onLocateModule}
      agentReady={agentReady}
    />
  )
}
