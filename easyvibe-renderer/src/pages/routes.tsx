import type React from 'react'
import { ReactFlowProvider } from '@xyflow/react'
import { BookOpen, Plug, ScrollText } from 'lucide-react'

import type { CodeMap } from '@/types/map'
import type { PageId } from '@/components/AppShell'
import type { TaskDraft } from '@/lib/taskContext'
import { loadTaskIdea, clearTaskIdea } from '@/lib/onboarding'
import { PlaceholderPage } from '@/components/PlaceholderPage'
import { ModulesPage } from '@/components/ModulesPage'
import { WorkbenchPage } from '@/components/WorkbenchPage'
import { TaskPage } from '@/components/TaskPage'
import { DriftPage } from '@/components/DriftPage'
import { HealthPage } from '@/components/HealthPage'
import { ChangesPage } from '@/components/ChangesPage'
import { GitPage } from '@/components/GitPage'
import { SettingsPanel } from '@/components/SettingsPanel'
import { DepsPage } from '@/components/DepsPage'
import { RunsPage } from '@/components/RunsPage'
import { UsagePage } from '@/components/UsagePage'
import { Canvas } from '@/components/canvas/Canvas'
import { CanvasBoundary } from '@/components/canvas/CanvasBoundary'

export type ChatContext = { refId: string; refName: string; kind: 'module' | 'layer' }

export type BuildPagesProps = {
  map: CodeMap
  backendRepo: string | null
  onPatrollingChange: (v: boolean) => void
  panelOpen: boolean
  onPanelOpenChange: (open: boolean) => void
  panelWidth: number
  onPanelWidthChange: (w: number) => void
  viewRequest: string[] | null
  onViewRequestConsumed: () => void
  agentReady: boolean
  guideDismissed: boolean
  onDismissGuide: () => void
  onTaskCreated: (id: string) => void
  onOpenTask: (d: TaskDraft) => void
  onChatAbout: (target: ChatContext) => void
  onGoWorkbench: () => void
  onInspectEdge: (cardId: string) => void
  onOpenRuns: (sessionId: string) => void
  onOpenDeps: () => void
  lensRequest: string | null
  onLensRequestConsumed: () => void
  dark: boolean
  page: PageId
  taskFocus: { id: string; nonce: number } | null
  runsFocus: string | null
  onRunsFocusConsumed: () => void
  queueResyncTick: number
  onOpenChangesToTasks: (id: string) => void
  onRequestView: (ids: string[]) => void
  onRequestLens: (id: string) => void
  onDepsFocusConsumed: () => void
  depsFocus: string | null
  pendingChatContext: ChatContext | null
  onConsumeChatContext: () => void
  onNavigate: (p: PageId) => void
}

/**
 * PAGES 装配表（唯一「PageId ↔ 页面」对照点）：17 键（含 todo/review 兼容别名）。
 * 从 App 抽出，页面节点构造逐字保留；App 只负责传数据源与回调。
 */
export function buildPages(p: BuildPagesProps): Record<PageId, React.ReactNode> {
  return {
    map: (
      <CanvasBoundary>
        <ReactFlowProvider>
          <Canvas
            map={p.map}
            backendRepo={p.backendRepo}
            onPatrollingChange={p.onPatrollingChange}
            panelOpen={p.panelOpen}
            onPanelOpenChange={p.onPanelOpenChange}
            panelWidth={p.panelWidth}
            onPanelWidthChange={p.onPanelWidthChange}
            viewRequest={p.viewRequest}
            onViewRequestConsumed={p.onViewRequestConsumed}
            agentReady={p.agentReady}
            guide={
              p.guideDismissed ? undefined : (
                <div className="mx-2 mt-2 flex items-start gap-2 rounded-lg border border-blue-100 bg-blue-50/70 dark:bg-blue-950/40 px-3 py-2">
                  <p className="flex-1 text-[11px] leading-5 text-slate-600 dark:text-slate-300">
                    <span className="font-semibold text-blue-700">界面已整理：</span>
                    详情栏页签为 详情/问题；对话在左侧「任务对话」，任务在「任务」，视图与优化建议在顶栏。
                  </p>
                  <button
                    onClick={p.onDismissGuide}
                    className="shrink-0 rounded-full bg-white dark:bg-slate-900 px-2 py-0.5 text-micro font-semibold text-blue-600 shadow-sm hover:bg-blue-50 dark:hover:bg-blue-950/40"
                  >
                    知道了
                  </button>
                </div>
              )
            }
            onTaskCreated={p.onTaskCreated}
            onChatAbout={p.onChatAbout}
            onGoWorkbench={p.onGoWorkbench}
            onInspectEdge={p.onInspectEdge}
            onOpenRuns={p.onOpenRuns}
            onOpenDeps={p.onOpenDeps}
            lensRequest={p.lensRequest}
            onLensRequestConsumed={p.onLensRequestConsumed}
            dark={p.dark}
          />
        </ReactFlowProvider>
      </CanvasBoundary>
    ),
    tasks: <TaskPage backendRepo={p.backendRepo} map={p.map} onCreateTask={p.onOpenTask} externalFocus={p.taskFocus} onGoChat={p.onGoWorkbench} onOpenRuns={p.onOpenRuns} />,
    runs: (
      <RunsPage
        backendRepo={p.backendRepo}
        initialSessionId={p.runsFocus}
        onInitialConsumed={p.onRunsFocusConsumed}
        resyncKey={p.queueResyncTick}
      />
    ),
    usage: (
      <UsagePage
        backendRepo={p.backendRepo}
        map={p.map}
        onOpenModule={(id) => {
          p.onRequestView([id])
          p.onNavigate('map')
        }}
        onOpenSession={p.onOpenRuns}
      />
    ),
    settings: <SettingsPanel backendRepo={p.backendRepo} onClose={() => p.onNavigate('map')} embedded />,
    // v0.2 P1：「任务对话」——对话孵化任务（会话=任务上位容器：计划进度条/内联审批/diff 影响面三栏）
    workbench: (
      <WorkbenchPage
        backendRepo={p.backendRepo}
        map={p.map}
        onCreateTask={p.onOpenTask}
        onLocateModule={(id) => {
          // v0.2 定位链路升级：选中聚焦 + 切页（此前只切页不选中，地图端找不回模块）
          p.onRequestView([id])
          p.onNavigate('map')
        }}
        pendingChatContext={p.pendingChatContext}
        onConsumeChatContext={p.onConsumeChatContext}
        initialIdea={p.page === 'workbench' ? loadTaskIdea() : null}
        onConsumeIdea={clearTaskIdea}
        onNavigate={p.onNavigate}
      />
    ),
    // v4 P1：任务编排/任务工作流两个旧页签删除，统一从「任务」页进入（视图切换）；
    // 旧 id 保留映射，兼容存量回调（合约预警"去评审"等）——落点都是 TaskPage
    todo: <TaskPage backendRepo={p.backendRepo} map={p.map} onCreateTask={p.onOpenTask} externalFocus={p.taskFocus} onGoChat={p.onGoWorkbench} onOpenRuns={p.onOpenRuns} />,
    review: <TaskPage backendRepo={p.backendRepo} map={p.map} onCreateTask={p.onOpenTask} externalFocus={p.taskFocus} onGoChat={p.onGoWorkbench} onOpenRuns={p.onOpenRuns} />,
    changes: (
      <ChangesPage
        backendRepo={p.backendRepo}
        map={p.map}
        /* 重审 P2：变更页 → 任务页流水线（选中该任务）——导航闭环，不再靠用户记任务 ID */
        onOpenTask={p.onOpenChangesToTasks}
      />
    ),
    drift: <DriftPage />,
    health: <HealthPage backendRepo={p.backendRepo} map={p.map} onCreateTask={p.onOpenTask} onOpenDeps={p.onOpenDeps} />,
    modules: (
      <ModulesPage
        map={p.map}
        onOpenMap={() => p.onNavigate('map')}
        onCreateTask={p.onOpenTask}
        onLocateModule={(id) => {
          p.onRequestView([id])
          p.onNavigate('map')
        }}
      />
    ),
    deps: (
      <DepsPage
        backendRepo={p.backendRepo}
        map={p.map}
        onCreateTask={p.onOpenTask}
        onChatAbout={p.onChatAbout}
        onOpenMap={() => p.onNavigate('map')}
        onInspectModule={p.onRequestLens}
        focusCardId={p.depsFocus}
        onFocusConsumed={p.onDepsFocusConsumed}
      />
    ),
    git: (
      <GitPage
        backendRepo={p.backendRepo}
        map={p.map}
        onOpenChanges={() => p.onNavigate('changes')}
        onOpenReview={() => p.onNavigate('review')}
      />
    ),
    'kb-docs': <PlaceholderPage title="文档中心" milestone="M4-4" description="知识库三页为 P3 骨架：从已定样式模式派生。" icon={BookOpen} />,
    'kb-decisions': <PlaceholderPage title="决策记录" milestone="M4-4" description="这个仓库做过的重要技术决策及其来龙去脉。" icon={ScrollText} />,
    'kb-apis': <PlaceholderPage title="接口目录" milestone="M4-4" description="全部关键入口（路由/函数/任务）的索引：谁对外提供什么能力。" icon={Plug} />,
  }
}

// 供索引/守卫引用的键集合（显式列出，避免运行时依赖）
export const PAGE_IDS: PageId[] = ['map', 'modules', 'deps', 'drift', 'health', 'workbench', 'tasks', 'runs', 'usage', 'todo', 'review', 'changes', 'git', 'kb-docs', 'kb-decisions', 'kb-apis', 'settings']
