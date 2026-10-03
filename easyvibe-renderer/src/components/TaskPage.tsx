import { useState } from 'react'
import { ClipboardList, LayoutGrid, Rows3, Plus } from 'lucide-react'
import { TaskWorkflowPage } from '@/components/TaskWorkflowPage'
import { TaskBoardPage } from '@/components/TaskBoardPage'
import type { CodeMap } from '@/types/map'
import type { TaskDraft } from '@/lib/taskContext'

// 任务页（v4 P1 施工）：「任务」唯一入口，页内视图切换——
// 流水线（默认，单任务全生命周期）/ 看板（多任务并行全景）。治理视图 P2。
// 旧「任务编排」「任务工作流」两个页签删除，组件整体迁入本页。

type View = 'pipeline' | 'board'

export function TaskPage({
  backendRepo,
  map,
  onCreateTask,
}: {
  backendRepo: string | null
  map: CodeMap
  onCreateTask: (d: TaskDraft) => void
}) {
  // v4 修订（用户裁定）：默认看板（多任务全景是首页心智），点卡跳流水线看单任务全程
  const [view, setView] = useState<View>('board')
  const [focusTask, setFocusTask] = useState<{ id: string; nonce: number } | null>(null)

  const openPipeline = (taskId: string) => {
    setFocusTask({ id: taskId, nonce: Date.now() })
    setView('pipeline')
  }

  return (
    <div className="flex h-full flex-col">
      {/* 页头：标题 + 视图切换 + 新建（看板第一顺位） */}
      <div className="flex items-center gap-3 border-b border-slate-100 bg-white px-4 py-2.5">
        <h2 className="flex items-center gap-1.5 text-[14px] font-bold text-slate-800">
          <ClipboardList size={15} className="text-slate-500" /> 任务
        </h2>
        <div className="flex rounded-lg bg-slate-100 p-0.5">
          <button
            onClick={() => setView('board')}
            className={`flex items-center gap-1 rounded-md px-3 py-1 text-[11px] font-bold transition-colors ${
              view === 'board' ? 'bg-white text-blue-600 shadow-sm' : 'text-slate-500 hover:text-slate-700'
            }`}
          >
            <LayoutGrid size={11} /> 看板
          </button>
          <button
            onClick={() => setView('pipeline')}
            className={`flex items-center gap-1 rounded-md px-3 py-1 text-[11px] font-bold transition-colors ${
              view === 'pipeline' ? 'bg-white text-blue-600 shadow-sm' : 'text-slate-500 hover:text-slate-700'
            }`}
          >
            <Rows3 size={11} /> 流水线
          </button>
        </div>
        <p className="hidden text-[10px] text-slate-400 lg:block">
          {view === 'board' ? '全部任务的并行全景，拖拽即处分，点卡看单个任务全程' : '一个任务走 harness 五阶段的全程'}
        </p>
        <button
          onClick={() => onCreateTask({ title: '', description: '', modules: [], acceptance: '', source: 'manual', context: {} })}
          className="ml-auto flex items-center gap-1 rounded-lg bg-blue-600 px-3 py-1.5 text-[11px] font-bold text-white hover:bg-blue-700"
        >
          <Plus size={11} /> 新建任务
        </button>
      </div>
      {/* 视图区（组件常驻，display 切换保状态——终端缓冲/选中不丢） */}
      <div className="min-h-0 flex-1" style={view === 'pipeline' ? undefined : { display: 'none' }}>
        <TaskWorkflowPage backendRepo={backendRepo} map={map} onCreateTask={onCreateTask} focusTask={focusTask} />
      </div>
      <div className="min-h-0 flex-1" style={view === 'board' ? undefined : { display: 'none' }}>
        <TaskBoardPage backendRepo={backendRepo} onOpenWorkflow={() => setView('pipeline')} onSelectTask={openPipeline} onCreateTask={onCreateTask} />
      </div>
    </div>
  )
}
