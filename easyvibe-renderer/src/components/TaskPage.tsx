import { useEffect, useState } from 'react'
import { ClipboardList, LayoutGrid, Rows3, Plus, Table2, MessagesSquare } from 'lucide-react'
import { TaskWorkflowPage } from '@/components/TaskWorkflowPage'
import { TaskBoardPage } from '@/components/TaskBoardPage'
import { TaskGovernancePage } from '@/components/TaskGovernancePage'
import type { CodeMap } from '@/types/map'
import type { TaskDraft } from '@/lib/taskContext'

// 任务页（v4 P1 施工 + P2 治理视图）：「任务」唯一入口，页内视图切换——
// 看板（默认，并行全景）/ 流水线（单任务全程）/ 治理（历史/失败/返工链）。
// 旧「任务编排」「任务工作流」两个页签删除，组件迁入本页。
// v0.2 分工：本页 = 流程视角（任务走到哪一步）；孵化视角（对话聊出任务）在「任务对话」页。

type View = 'board' | 'pipeline' | 'governance'

export function TaskPage({
  backendRepo,
  map,
  onCreateTask,
  externalFocus,
  onGoChat,
}: {
  backendRepo: string | null
  map: CodeMap
  onCreateTask: (d: TaskDraft) => void
  /** 表单"前往任务页签跟踪"传入：切流水线并选中该任务（nonce 区分多次） */
  externalFocus?: { id: string; nonce: number } | null
  /** v0.2 互指条：跳「任务对话」页 */
  onGoChat?: () => void
}) {
  // v4 修订（用户裁定）：默认看板（多任务全景是首页心智），点卡跳流水线看单任务全程
  const [view, setView] = useState<View>('board')
  const [focusTask, setFocusTask] = useState<{ id: string; nonce: number } | null>(null)

  const openPipeline = (taskId: string) => {
    setFocusTask({ id: taskId, nonce: Date.now() })
    setView('pipeline')
  }

  // 外部跳入（任务表单创建成功）：nonce 变化即生效
  useEffect(() => {
    if (externalFocus?.id) openPipeline(externalFocus.id)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [externalFocus])

  return (
    <div className="flex h-full flex-col">
      {/* 页头：标题 + 视图切换 + 新建（看板第一顺位） */}
      <div className="flex items-center gap-3 border-b border-slate-100 dark:border-slate-800 bg-white dark:bg-slate-900 px-4 py-2.5">
        <h2 className="flex items-center gap-1.5 text-[14px] font-bold text-slate-800 dark:text-slate-100">
          <ClipboardList size={15} className="text-slate-500 dark:text-slate-400" /> 任务
        </h2>
        <div className="flex rounded-lg bg-slate-100 dark:bg-slate-800 p-0.5">
          <button
            onClick={() => setView('board')}
            className={`flex items-center gap-1 rounded-md px-3 py-1 text-[11px] font-bold transition-colors ${
              view === 'board' ? 'bg-white dark:bg-slate-900 text-blue-600 shadow-sm' : 'text-slate-500 dark:text-slate-400 hover:text-slate-700'
            }`}
          >
            <LayoutGrid size={11} /> 看板
          </button>
          <button
            onClick={() => setView('pipeline')}
            className={`flex items-center gap-1 rounded-md px-3 py-1 text-[11px] font-bold transition-colors ${
              view === 'pipeline' ? 'bg-white dark:bg-slate-900 text-blue-600 shadow-sm' : 'text-slate-500 dark:text-slate-400 hover:text-slate-700'
            }`}
          >
            <Rows3 size={11} /> 流水线
          </button>
          <button
            onClick={() => setView('governance')}
            className={`flex items-center gap-1 rounded-md px-3 py-1 text-[11px] font-bold transition-colors ${
              view === 'governance' ? 'bg-white dark:bg-slate-900 text-blue-600 shadow-sm' : 'text-slate-500 dark:text-slate-400 hover:text-slate-700'
            }`}
          >
            <Table2 size={11} /> 治理
          </button>
        </div>
        <p className="hidden text-[10px] text-slate-400 dark:text-slate-500 lg:block">
          {view === 'board' ? '全部任务的并行全景，拖拽即处分，点卡看单个任务全程' : view === 'pipeline' ? '一个任务走 harness 五阶段的全程' : '全部任务的历史、失败与返工链'}
        </p>
        {onGoChat && (
          <button
            onClick={onGoChat}
            className="ml-1 flex shrink-0 items-center gap-1 rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-1 text-micro font-semibold text-slate-500 dark:text-slate-400 hover:border-blue-300 hover:text-blue-600"
            title="任务从对话孵化：去「任务对话」页与 agent 聊出任务"
          >
            <MessagesSquare size={10} /> 去任务对话
          </button>
        )}
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
      <div className="min-h-0 flex-1" style={view === 'governance' ? undefined : { display: 'none' }}>
        <TaskGovernancePage backendRepo={backendRepo} onOpenTask={openPipeline} onCreateTask={onCreateTask} />
      </div>
    </div>
  )
}
