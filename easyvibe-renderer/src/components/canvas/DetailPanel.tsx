import { X, Boxes, ListChecks, MessagesSquare } from 'lucide-react'
import type { TaskDraft } from '@/shared/logic/taskContext'
import type { CodeMap, SubMap } from '@/types/map'
import { IssuesList } from './IssuesList'
import { ModuleView, LayerView, SubmoduleView } from './DetailViews'
import { PanelChat } from '@/components/chat/PanelChat'
import { useLang } from '@/runtime/i18n'

// 共享契约类型（2026-10-05 下沉 renderer-shared/contract）：canvas 与 chat 两侧共用，
// 定义在此会让 chat 域反向依赖 map-canvas，故统一改为从 shared 引入。
import type { Selection, PanelTab } from '@/shared/contract/selection'
import type { ChatAboutTarget } from '@/shared/contract/chat'

interface Props {
  map: CodeMap
  selection: Selection
  tab: PanelTab
  onTabChange: (tab: PanelTab) => void
  submaps: Record<string, SubMap | 'loading' | 'error'>
  backendRepo: string | null
  onCreateTask: (draft: TaskDraft) => void
  onLocateModule: (moduleId: string) => void
  /** S1-1：打开视图（多模块时画布 solo 聚焦） */
  onOpenView?: (ids: string[]) => void
  /** v0.2：对话页签占位与详情视图的「就此对话」入口（携带当前选中对象） */
  onChatAbout?: (target: ChatAboutTarget) => void
  /** 2026-10-05 Redesign-A：右栏审批出口——跳工作台「任务对话」页裁决 */
  onGoWorkbench?: () => void
  /** 2026-10-05 依赖体检入口（详情页签「耦合概览」区块「看全部 →」） */
  onOpenDeps?: () => void
  onClose: () => void
  /** 改进#4：右栏可调宽（测试员 IA 反馈的非破坏性验证——宽度够不够先看数据） */
  width?: number
}

// 右栏详情面板（M4-1：三页签 详情/问题/对话）。英文化第二批瘦身：三个选中对象视图 +
// 治理账单 + 健康趋势抽至 ./DetailViews（DetailPanel 贴 componentGuard LEGACY 628 红线，
// t() 迁移净增行数，拆分后两边都回到阈值内；新增文件已登记守卫）。
export function DetailPanel({ map, selection, tab, onTabChange, submaps, backendRepo, onCreateTask, onLocateModule, onChatAbout, onGoWorkbench, onOpenDeps, onClose, width }: Props) {
  const { t } = useLang()
  const module = selection?.kind === 'module' ? map.modules.find((m) => m.id === selection.id) : undefined
  const layer = selection?.kind === 'layer' ? map.layers.find((l) => l.id === selection.id) : undefined
  const parent = selection?.kind === 'submodule' ? map.modules.find((m) => m.id === selection.parentId) : undefined
  const submap = selection?.kind === 'submodule' ? submaps[selection.parentId] : undefined
  const smLoaded = submap && submap !== 'loading' && submap !== 'error' ? submap : undefined
  const sub = selection?.kind === 'submodule' && smLoaded
    ? smLoaded.sub_modules.find((s) => s.id === selection.subId)
    : undefined

  const detailLabel = module ? t('canvas.panel.detailModule') : layer ? t('canvas.panel.detailLayer') : sub ? t('canvas.panel.detailSub') : t('canvas.panel.detailGeneric')
  // M4-1.5 选区收敛：模块或子模块选中时，问题清单随之收敛到该模块
  const scopeModuleName = module?.name ?? parent?.name

  return (
    <aside className="anim-panel-in flex h-full shrink-0 flex-col border-l border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900" style={{ width: width ?? 340 }}>
      <div className="flex items-center justify-between border-b border-slate-100 dark:border-slate-800 px-3 py-2">
        <div className="flex gap-1">
          {/* M4-1 瘦身：右栏只留 详情/问题/对话 三页签（v3 定稿顺序）；建议/视图移至顶栏抽屉，任务移至工作区页 */}
          {(
            [
              ['detail', detailLabel, <Boxes key="d" size={13} />],
              ['issues', t('canvas.panel.tabIssues'), <ListChecks key="i" size={13} />],
              ['chat', t('canvas.panel.tabChat'), <MessagesSquare key="c" size={13} />],
            ] as const
          ).map(([key, label, icon]) => (
            <button
              key={key as string}
              onClick={() => onTabChange(key as typeof tab)}
              className={`flex items-center gap-1 rounded-md px-2 py-1.5 text-[12px] font-semibold transition-colors ${
                tab === key ? 'bg-blue-50 dark:bg-blue-950/40 text-blue-600' : 'text-slate-400 dark:text-slate-500 hover:text-slate-600'
              }`}
            >
              {icon}
              {label as string}
            </button>
          ))}
        </div>
        <button onClick={onClose} className="rounded p-1 text-slate-400 dark:text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-600">
          <X size={16} />
        </button>
      </div>
      {/* key={tab}：切页签重触发 150ms 淡入（评审 R3——此前内容瞬换无过渡） */}
      {tab === 'chat' ? (
        /* 2026-10-05 Redesign-A：对话页签独立容器——QuickAsk（检查器文档流）自管
           ContextBar/文档流/输入坞的纵向滚动，不再套 p-4 滚动层（双滚动条/贴边观感硬伤之源） */
        <div key={tab} className="anim-fade-in-fast flex h-full min-h-0 flex-1 flex-col overflow-hidden">
          <PanelChat
            backendRepo={backendRepo}
            map={map}
            selection={selection}
            onCreateTask={onCreateTask}
            onLocateModule={onLocateModule}
            onGoWorkbench={onGoWorkbench}
          />
        </div>
      ) : (
      <div key={tab} className="anim-fade-in-fast flex-1 space-y-5 overflow-y-auto p-4">
        {tab === 'issues' && (
          <IssuesList
            map={map}
            onLocate={(id) => onLocateModule(id)}
            onCreateTask={onCreateTask}
            backendRepo={backendRepo}
            scopeId={selection?.kind === 'module' ? selection.id : selection?.kind === 'submodule' ? selection.parentId : undefined}
            scopeName={scopeModuleName}
          />
        )}
        {tab === 'detail' && module && <ModuleView map={map} mod={module} onCreateTask={onCreateTask} backendRepo={backendRepo} onChatAbout={onChatAbout} onOpenDeps={onOpenDeps} />}
        {tab === 'detail' && !module && layer && <LayerView map={map} layer={layer} onCreateTask={onCreateTask} onChatAbout={onChatAbout} />}
        {tab === 'detail' && !module && !layer && sub && parent && smLoaded && (
          <SubmoduleView parent={parent} sub={sub} submap={smLoaded} onCreateTask={onCreateTask} />
        )}
        {tab === 'detail' && !module && !layer && !sub && selection?.kind === 'submodule' && (
          <p className="pt-8 text-center text-[12px] leading-5 text-slate-400 dark:text-slate-500">
            {t('canvas.panel.subLoading')}
          </p>
        )}
        {/* M4-1 详情空选态 = 地图级摘要（设计师要求：禁止空白，三态规范最高频面板落地） */}
        {tab === 'detail' && !module && !layer && !sub && selection?.kind !== 'submodule' && (
          <div className="space-y-3 pt-2">
            <p className="text-[11px] font-semibold text-slate-400 dark:text-slate-500">{t('canvas.panel.overviewTitle')}</p>
            <div className="grid grid-cols-2 gap-2">
              {[
                [t('canvas.panel.overviewModules'), `${map.modules.length}`],
                [t('canvas.panel.overviewEdges'), `${map.edges.length}`],
                [t('canvas.panel.overviewScore'), `${map.health.score}`],
                [t('canvas.panel.overviewCoupling'), map.health.coupling],
              ].map(([k, v]) => (
                <div key={k} className="rounded-lg border border-slate-100 dark:border-slate-800 bg-slate-50 dark:bg-slate-950/70 px-3 py-2">
                  <p className="text-micro text-slate-400 dark:text-slate-500">{k}</p>
                  <p className="text-[15px] font-bold text-slate-700 dark:text-slate-200">{v}</p>
                </div>
              ))}
            </div>
            {map.health.review_note && (
              <p className="rounded-lg border border-slate-100 dark:border-slate-800 bg-slate-50 dark:bg-slate-950/70 px-3 py-2 text-[11px] leading-5 text-slate-500 dark:text-slate-400">
                {map.health.review_note}
              </p>
            )}
            <p className="text-cap leading-5 text-slate-400 dark:text-slate-500">
              {t('canvas.panel.overviewHint')}
            </p>
          </div>
        )}
      </div>
      )}
    </aside>
  )
}
