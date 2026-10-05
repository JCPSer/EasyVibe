import { Activity, CircleHelp, FileDown, LayoutGrid, Lightbulb, Settings, WifiOff } from 'lucide-react'
import { useEffect, useState } from 'react'
import { isWsConnected, onWsConnection } from '@/runtime/growthBus'
import { ThemeToggle } from '@/components/ThemeToggle'
import { TopCenter } from './TopCenter'

/**
 * 实时连接状态（2026-10-05 实弹：WS 断线曾导致任务终端静默空白而全界面无感知）。
 * 断开时显示弱琥珀提示；正常时零占用。HTTP 兜底（终端/运行页）已覆盖主要消费端。
 */
function WsStatusChip() {
  const [connected, setConnected] = useState(isWsConnected())
  useEffect(() => onWsConnection(setConnected), [])
  if (connected) return null
  return (
    <span
      className="flex items-center gap-1 rounded-lg border border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-2 py-1 text-[12px] font-semibold text-amber-700"
      title="与后端的实时连接已断开，正在自动重连；页面数据走 HTTP 轮询兜底（实时性降级）"
    >
      <WifiOff size={12} />
      重连中
    </span>
  )
}

/**
 * 顶栏：中区仓库切换器 + 右区全局动作（视图/建议/巡检/导出/主题/帮助/设置）。
 * 从 App 抽出，JSX 与条件渲染逐字保留。
 */
export function TopBar({
  backendOnline,
  backendRepo,
  repos,
  onSwitchRepo,
  onAddRepo,
  onRemoveRepo,
  onToggleViews,
  onToggleSuggest,
  patrolling,
  onStartPatrol,
  agentReady,
  onExport,
  dark,
  onToggleDark,
  welcomeOpen,
  onToggleWelcome,
  page,
  onOpenSettings,
}: {
  backendOnline: boolean | null
  backendRepo: string | null
  repos: { id: string; name: string }[]
  onSwitchRepo: (id: string) => void
  onAddRepo: () => void
  onRemoveRepo: (id: string, wipe: boolean) => Promise<boolean>
  onToggleViews: () => void
  onToggleSuggest: () => void
  patrolling: boolean
  onStartPatrol: () => void
  /** agentState.found：仅显式 false 视为不可用 */
  agentReady: boolean
  onExport: () => void
  dark: boolean
  onToggleDark: (v: boolean) => void
  welcomeOpen: boolean
  onToggleWelcome: () => void
  page: string
  onOpenSettings: () => void
}) {
  return (
      <div className="flex min-w-0 flex-1 items-center gap-2">
        <TopCenter
          backendOnline={backendOnline}
          backendRepo={backendRepo}
          repos={repos}
          onSwitchRepo={onSwitchRepo}
          onAddRepo={onAddRepo}
          onRemoveRepo={onRemoveRepo}
        />
        <div className="ml-auto flex shrink-0 items-center gap-1.5">
        <WsStatusChip />
        <button
          onClick={onToggleViews}
          className="flex items-center gap-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-1 text-[12px] font-semibold text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70"
          title="我的视图（对话沉淀的图资产）"
        >
          <LayoutGrid size={12} />
          视图
        </button>
        {backendRepo && (
          <button
            onClick={onToggleSuggest}
            className="flex items-center gap-1 rounded-lg border border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-2 py-1 text-[12px] font-semibold text-amber-700 hover:bg-amber-100 dark:hover:bg-amber-900/40"
            title="AI 主动发现优化建议，逐条可发起修复"
          >
            <Lightbulb size={12} />
            优化建议
          </button>
        )}
        {backendRepo && (
          <button
            onClick={onStartPatrol}
            disabled={patrolling || !agentReady}
            className={`flex items-center gap-1 rounded-lg border px-2 py-1 text-[12px] font-semibold transition-colors disabled:opacity-40 ${
              patrolling ? 'border-amber-300 dark:border-amber-800 bg-amber-50 dark:bg-amber-950/40 text-amber-700' : 'border-emerald-200 dark:border-emerald-900/60 bg-emerald-50 dark:bg-emerald-950/40 text-emerald-700 hover:bg-emerald-100'
            }`}
            title={!agentReady ? '未检测到执行 agent——先安装或在设置中配置' : '巡检：Supervisor 直调 LLM（带健康基线），产出新地图并落健康历史'}
          >
            <Activity size={12} className={patrolling ? 'animate-pulse' : ''} />
            {patrolling ? '巡检中…' : '巡检'}
          </button>
        )}
        <button
          onClick={onExport}
          className="flex items-center gap-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-1 text-[12px] font-semibold text-slate-600 dark:text-slate-300 hover:bg-slate-50 dark:hover:bg-slate-800/70"
          title="导出架构健康报告（Markdown，零 token 成本）"
        >
          <FileDown size={12} />
          导出
        </button>
        {/* 主题开关：亮=太阳 / 暗=月亮滑动拨块（2026-10-04） */}
        <ThemeToggle dark={dark} onChange={onToggleDark} />
        {/* 新手引导：帮助入口——重看欢迎页（再点关闭 = 开关语义，2026-10-04 实弹） + 重置上手指引 */}
        <button
          onClick={onToggleWelcome}
          className={`rounded-lg border px-2 py-1 text-[12px] font-semibold transition-colors ${
            welcomeOpen
              ? 'border-blue-300 bg-blue-50 text-blue-700 dark:border-blue-800 dark:bg-blue-950/50 dark:text-blue-300'
              : 'border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70'
          }`}
          title="新手引导（欢迎页 + 上手指引）——再点关闭"
        >
          <CircleHelp size={12} />
        </button>
        <button
          onClick={onOpenSettings}
          className={`rounded-lg border px-2 py-1 text-[12px] font-semibold ${
            page === 'settings' ? 'border-blue-300 dark:border-blue-800 bg-blue-50 dark:bg-blue-950/40 text-blue-700' : 'border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70'
          }`}
          title="设置（LLM 服务 / 槽位绑定 / 高级）"
        >
          <Settings size={12} />
        </button>
        </div>
      </div>
  )
}
