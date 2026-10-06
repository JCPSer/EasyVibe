import { useState } from 'react'
import { TrafficLightsSpacer, FakeTrafficLights } from './WindowControls'
import { startWindowDrag, toggleWindowMaximize } from '@/runtime/host'
import { useLang } from '@/lib/i18n'
import {
  GitBranch,
  Map as MapIcon,
  Boxes,
  Waypoints,
  Radar,
  HeartPulse,
  MessagesSquare,
  ClipboardList,
  Activity,
  Gauge,
  History,
  BookOpen,
  ScrollText,
  Plug,
  Settings,
  PanelLeftClose,
  PanelLeftOpen,
} from 'lucide-react'

// M4-1 应用壳：左侧导航（探索/工作区/知识库 + 设置）+ 顶栏槽位 + 页面容器。
// 信息架构以 docs/m4-design-brief-v3.md 为准；P1/P2 页面先占位（PlaceholderPage 诚实空态）。
export type PageId =
  | 'map'
  | 'modules'
  | 'deps'
  | 'drift'
  | 'health'
  | 'workbench'
  | 'tasks'
  | 'runs'
  | 'usage'
  | 'todo'
  | 'review'
  | 'changes'
  | 'git'
  | 'kb-docs'
  | 'kb-decisions'
  | 'kb-apis'
  | 'settings'

// 导航文案经 i18n key 取词（字典见 @/lib/i18n，域 shell.nav）。
const NAV: { groupKey: string; items: { id: PageId; labelKey: string; icon: typeof MapIcon }[] }[] = [
  {
    groupKey: 'shell.nav.group.explore',
    items: [
      { id: 'map', labelKey: 'shell.nav.map', icon: MapIcon },
      { id: 'modules', labelKey: 'shell.nav.modules', icon: Boxes },
      { id: 'deps', labelKey: 'shell.nav.deps', icon: Waypoints },
      { id: 'drift', labelKey: 'shell.nav.drift', icon: Radar },
      { id: 'health', labelKey: 'shell.nav.health', icon: HeartPulse },
    ],
  },
  {
    groupKey: 'shell.nav.group.workspace',
    items: [
      { id: 'workbench', labelKey: 'shell.nav.workbench', icon: MessagesSquare },
      { id: 'tasks', labelKey: 'shell.nav.tasks', icon: ClipboardList },
      { id: 'runs', labelKey: 'shell.nav.runs', icon: Activity },
      { id: 'usage', labelKey: 'shell.nav.usage', icon: Gauge },
      { id: 'changes', labelKey: 'shell.nav.changes', icon: History },
      { id: 'git', labelKey: 'shell.nav.git', icon: GitBranch },
    ],
  },
  {
    groupKey: 'shell.nav.group.kb',
    items: [
      { id: 'kb-docs', labelKey: 'shell.nav.kb-docs', icon: BookOpen },
      { id: 'kb-decisions', labelKey: 'shell.nav.kb-decisions', icon: ScrollText },
      { id: 'kb-apis', labelKey: 'shell.nav.kb-apis', icon: Plug },
    ],
  },
]

interface Props {
  page: PageId
  onPageChange: (p: PageId) => void
  /** 页签徽标：数字=红底待审批数；{alert, info}=双徽标（红=等你审批，蓝=执行中） */
  badges?: Partial<Record<PageId, number | { alert: number; info: number }>>
  /** v4 全局注意力条（有待审批时出现；null 不渲染） */
  attentionBar?: React.ReactNode
  /** 顶栏内容（项目选择器 + 全局动作），由 App 注入 */
  topBar: React.ReactNode
  /** 2026-10-04 顶栏正中绝对居中槽位（运行会话指示器）——相对整条 header 居中，
      不受左侧 logo/项目区与右侧按钮簇不等宽影响（mx-auto/相对 topBar 都会被顶偏） */
  topCenter?: React.ReactNode
  children: React.ReactNode
}

export function AppShell({ page, onPageChange, badges, attentionBar, topBar, topCenter, children }: Props) {
  const { t } = useLang()
  // 默认展开（设计师：默认即 90% 场景，折叠只是权力不是义务）；折叠选择持久化（真人测试建议#5）
  const [collapsed, setCollapsed] = useState(() => typeof window !== 'undefined' && localStorage.getItem('ev.nav.collapsed') === '1')
  const toggleCollapsed = () => {
    setCollapsed((v) => {
      localStorage.setItem('ev.nav.collapsed', v ? '0' : '1')
      return !v
    })
  }
  return (
    <div className="flex h-screen flex-col bg-slate-50 dark:bg-slate-950/70">
      {/* 自绘标题栏（multica/Electron hiddenInset 范式）：原生红绿灯悬浮左上（Overlay），
          前端留 76px 净空。拖拽双保险：data-tauri-drag-region（Tauri 原生命中测试，免 IPC）
          + 空白 mousedown → startDragging（权限已开）；双击空白 → 最大化/还原 */}
      <header
        data-tauri-drag-region
        className="glass relative z-20 flex h-10 shrink-0 items-center gap-2 border-b border-slate-200 dark:border-slate-700 px-3"
        onMouseDown={(e) => {
          if (e.button !== 0) return
          if ((e.target as HTMLElement).closest('button, a, input, select, textarea, [data-no-drag]')) return
          startWindowDrag()
        }}
        onDoubleClick={(e) => {
          if ((e.target as HTMLElement).closest('button, a, input, select, textarea, [data-no-drag]')) return
          toggleWindowMaximize()
        }}
      >
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <FakeTrafficLights />
          <TrafficLightsSpacer />
          <span className="flex shrink-0 items-center gap-1.5 pr-2">
            <span className="flex h-6 w-6 items-center justify-center rounded-md bg-blue-600 text-[11px] font-black text-white">EV</span>
            <span className="text-[13px] font-bold tracking-tight text-slate-800 dark:text-slate-100">EasyVibe</span>
          </span>
          {topBar}
        </div>
        {/* 顶栏正中槽位：相对整条 header 绝对居中（不受左右簇宽度影响） */}
        {topCenter && (
          <div className="pointer-events-none absolute left-1/2 top-1/2 z-30 -translate-x-1/2 -translate-y-1/2">
            <div className="pointer-events-auto">{topCenter}</div>
          </div>
        )}
      </header>
      <div className="flex min-h-0 flex-1">
        {/* 左侧导航 */}
        <nav
          className={`flex shrink-0 flex-col border-r border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 transition-all ${
            collapsed ? 'w-12 items-center' : 'w-52'
          }`}
        >
          <div className="min-h-0 flex-1 space-y-3 overflow-y-auto py-3">
            {NAV.map((g) => (
              <div key={g.groupKey}>
                {!collapsed && <p className="px-3 pb-1 text-micro font-semibold uppercase tracking-wider text-slate-300 dark:text-slate-600">{t(g.groupKey)}</p>}
                {g.items.map((it) => {
                  const active = page === it.id
                  const badge = badges?.[it.id]
                  // v4：双徽标——红=待审批（等你），蓝=执行中（活着）；折叠态只显红点
                  const alert = typeof badge === 'object' ? badge.alert : badge
                  const info = typeof badge === 'object' ? badge.info : undefined
                  return (
                    <button
                      key={it.id}
                      onClick={() => onPageChange(it.id)}
                      title={collapsed ? t(it.labelKey) : undefined}
                      className={`flex w-full items-center gap-2 px-3 py-1.5 text-[13px] transition-colors ${
                        active ? 'border-r-2 border-blue-600 bg-blue-50/70 dark:bg-blue-950/40 font-semibold text-blue-700' : 'text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70 hover:text-slate-700'
                      } ${collapsed ? 'justify-center border-r-0 px-0' : ''}`}
                    >
                      <it.icon size={15} className="shrink-0" />
                      {!collapsed && <span className="min-w-0 flex-1 truncate text-left">{t(it.labelKey)}</span>}
                      {alert ? (
                        <span key="a" className="anim-scale-in rounded-full bg-red-500 px-1.5 text-micro font-bold leading-4 text-white">{alert}</span>
                      ) : null}
                      {info && !collapsed ? (
                        <span key="i" className="anim-scale-in flex items-center gap-1 rounded-full bg-blue-100 px-1.5 text-micro font-bold leading-4 text-blue-600">
                          <span className="h-1 w-1 animate-pulse rounded-full bg-blue-500" />
                          {info}
                        </span>
                      ) : null}
                    </button>
                  )
                })}
              </div>
            ))}
          </div>
          {/* 底部：设置 + 折叠把手 */}
          <div className={`border-t border-slate-100 dark:border-slate-800 py-2 ${collapsed ? 'flex flex-col items-center' : ''}`}>
            <button
              onClick={() => onPageChange('settings')}
              title={t('shell.nav.settings')}
              className={`flex w-full items-center gap-2 px-3 py-1.5 text-[13px] ${
                page === 'settings' ? 'font-semibold text-blue-700' : 'text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70 hover:text-slate-700'
              } ${collapsed ? 'justify-center border-r-0 px-0' : ''}`}
            >
              <Settings size={15} className="shrink-0" />
              {!collapsed && <span>{t('shell.nav.settings')}</span>}
            </button>
            <button
              onClick={toggleCollapsed}
              title={t(collapsed ? 'shell.nav.expandTip' : 'shell.nav.collapseTip')}
              className={`flex w-full items-center gap-2 px-3 py-1.5 text-[11px] text-slate-300 dark:text-slate-600 hover:text-slate-500 ${collapsed ? 'justify-center px-0' : ''}`}
            >
              {collapsed ? <PanelLeftOpen size={14} /> : (
                <>
                  <PanelLeftClose size={14} />
                  <span>{t('shell.nav.collapse')}</span>
                </>
              )}
            </button>
          </div>
        </nav>
        {/* 页面容器 */}
        <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
          {/* v4 全局注意力条：有待审批时出现在任何页面顶部，只说用户要做什么（数据 App 注入） */}
          {attentionBar}
          <div className="min-h-0 flex-1 overflow-hidden">{children}</div>
        </main>
      </div>
    </div>
  )
}
