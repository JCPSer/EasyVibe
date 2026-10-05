import { Bot, FolderOpen } from 'lucide-react'

export type Attention = { count: number; sample: string } | null

/**
 * 应用壳注意力条（优先级：零仓库 > agent 缺失引导 > 审批提醒）。
 * 从 App 抽出，三态 JSX 逐字保留。
 */
export function AttentionBar({
  backendOnline,
  repoCount,
  agentState,
  attention,
  page,
  onAddRepo,
  onAdoptAgent,
  onCopyInstallCmd,
  onOpenSettings,
  onGoTasks,
}: {
  backendOnline: boolean | null
  repoCount: number
  agentState: { found: boolean | null; detected: { command: string }[] }
  attention: Attention
  page: string
  onAddRepo: () => void
  onAdoptAgent: (command: string) => void
  onCopyInstallCmd: () => void
  onOpenSettings: () => void
  onGoTasks: () => void
}) {
  if (backendOnline === true && repoCount === 0) {
    return (
      <button
        onClick={onAddRepo}
        className="flex shrink-0 items-center gap-2 border-b border-blue-200 dark:border-blue-900/60 bg-blue-50 dark:bg-blue-950/40 px-4 py-1.5 text-left transition-colors hover:bg-blue-100"
      >
        <FolderOpen size={12} className="shrink-0 text-blue-500" />
        <span className="text-[11px] font-bold text-blue-800">尚未打开任何仓库——当前画布是演示数据</span>
        <span className="min-w-0 flex-1 truncate text-[11px] text-blue-600">
          归纳 / 巡检 / 任务 / 对话都需要一个本地代码仓库
        </span>
        <span className="shrink-0 rounded-md bg-blue-600 px-2 py-0.5 text-micro font-bold text-white">选择仓库 →</span>
      </button>
    )
  }
  if (backendOnline === true && agentState.found === false) {
    /* M2 首跑引导（R5）：检测到可采用的 → 一键采用；全未安装 → 安装指引+一键复制 */
    return agentState.detected.length > 0 ? (
      <div className="flex shrink-0 items-center gap-2 border-b border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-4 py-1.5">
        <Bot size={12} className="shrink-0 text-amber-500" />
        <span className="text-[11px] font-bold text-amber-800">
          检测到 {agentState.detected[0].command} 已安装
        </span>
        <span className="min-w-0 flex-1 truncate text-[11px] text-amber-600">采用后即可开始归纳 / 任务</span>
        <button
          onClick={() => onAdoptAgent(agentState.detected[0].command)}
          className="shrink-0 rounded-md bg-amber-600 px-2 py-0.5 text-micro font-bold text-white hover:bg-amber-700"
        >
          采用 {agentState.detected[0].command} →
        </button>
        <button onClick={onOpenSettings} className="shrink-0 rounded-md border border-amber-300 dark:border-amber-800 px-2 py-0.5 text-micro font-semibold text-amber-700 hover:bg-amber-100 dark:hover:bg-amber-900/40">
          去设置
        </button>
      </div>
    ) : (
      <div className="flex shrink-0 items-center gap-2 border-b border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-4 py-1.5">
        <Bot size={12} className="shrink-0 text-amber-500" />
        <span className="text-[11px] font-bold text-amber-800">未检测到执行 agent</span>
        <span className="min-w-0 flex-1 truncate text-[11px] text-amber-600">
          归纳 / 巡检 / 任务需要本地 CLI agent（支持 claude / codex / opencode）
        </span>
        <button onClick={onCopyInstallCmd} className="shrink-0 rounded-md bg-amber-600 px-2 py-0.5 text-micro font-bold text-white hover:bg-amber-700">
          复制 claude 安装命令
        </button>
        <button onClick={onOpenSettings} className="shrink-0 rounded-md border border-amber-300 dark:border-amber-800 px-2 py-0.5 text-micro font-semibold text-amber-700 hover:bg-amber-100 dark:hover:bg-amber-900/40">
          了解更多
        </button>
      </div>
    )
  }
  if (attention && page !== 'tasks') {
    return (
      <button
        onClick={onGoTasks}
        className="flex shrink-0 items-center gap-2 border-b border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-4 py-1.5 text-left transition-colors hover:bg-amber-100 dark:hover:bg-amber-900/40"
      >
        <span className="flex h-2 w-2 animate-pulse rounded-full bg-amber-500" />
        <span className="text-[11px] font-bold text-amber-800">
          {attention.count} 项任务等你审批
        </span>
        <span className="min-w-0 flex-1 truncate text-[11px] text-amber-600">—— {attention.sample}</span>
        <span className="shrink-0 rounded-md bg-amber-600 px-2 py-0.5 text-micro font-bold text-white">去处理 →</span>
      </button>
    )
  }
  return undefined
}
