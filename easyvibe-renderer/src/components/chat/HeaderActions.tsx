// 头部状态行（token 统计）+ 操作按钮组：删会话/导出清空/转任务/压缩上下文/新会话。
// 拆自 ChatPanel.tsx（2026-10-05 防膨胀）。
import { Download, RotateCcw, Shrink, Wrench, X as XIcon } from 'lucide-react'

export function HeaderActions({
  usage, convId, embedded, backendRepo, sending, compacting, messageCount, hasUserMessage,
  onDelete, onExportClear, onUpgrade, onCompress, onCreate,
}: {
  usage: { promptTokens: number; completionTokens: number }
  convId: string | null
  embedded?: boolean
  backendRepo: string | null
  sending: boolean
  compacting: boolean
  messageCount: number
  hasUserMessage: boolean
  onDelete: () => void
  onExportClear: () => void
  onUpgrade: () => void
  onCompress: () => void
  onCreate: () => void
}) {
  return (
    <div className="mb-2 flex items-center justify-between">
      <span className="tnum text-micro text-slate-400 dark:text-slate-500">
        会话已持久化 · 累计 {usage.promptTokens.toLocaleString()} / {usage.completionTokens.toLocaleString()} tokens
      </span>
      <div className="flex flex-wrap items-center justify-end gap-1">
        {convId && !embedded && (
          <button
            onClick={onDelete}
            disabled={!backendRepo}
            className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-0.5 text-micro text-slate-500 dark:text-slate-400 shadow-sm hover:bg-red-50 dark:hover:bg-red-950/40 hover:text-red-600 disabled:opacity-40"
            title="删除当前会话"
          >
            <XIcon size={10} />
            删会话
          </button>
        )}
        <button
          onClick={onExportClear}
          disabled={!backendRepo || messageCount === 0}
          className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-0.5 text-micro text-slate-500 dark:text-slate-400 shadow-sm hover:bg-slate-50 dark:hover:bg-slate-800/70 disabled:opacity-40"
          title="导出为 Markdown 后可清空会话（D2 拍板：留痕照旧，清理显式）"
        >
          <Download size={10} />
          导出/清空
        </button>
        <button
          onClick={onUpgrade}
          disabled={!backendRepo || sending || !hasUserMessage}
          className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-0.5 text-micro text-slate-500 dark:text-slate-400 shadow-sm hover:bg-blue-50 dark:hover:bg-blue-950/40 hover:text-blue-600 disabled:opacity-40"
          title="把本轮对话（已澄清的需求与引用模块）组织成修复任务"
        >
          <Wrench size={10} />
          转为任务
        </button>
        <button
          onClick={onCompress}
          disabled={!backendRepo || compacting}
          className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-0.5 text-micro text-slate-500 dark:text-slate-400 shadow-sm hover:bg-slate-50 dark:hover:bg-slate-800/70 disabled:opacity-40"
          title="压缩上下文：早期历史折叠为结构化摘要（原文保留在库中可回放）"
        >
          <Shrink size={10} />
          {compacting ? '压缩中…' : '压缩上下文'}
        </button>
        {/* v0.2：embedded 模式隐藏"新会话"——会话创建统一走工作台左栏（列表即管理），
            消除"预填/新会话落在哪"的双入口解释成本 */}
        {!embedded && (
          <button
            onClick={onCreate}
            disabled={!backendRepo || sending}
            className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-0.5 text-micro text-slate-500 dark:text-slate-400 shadow-sm hover:bg-slate-50 dark:hover:bg-slate-800/70 disabled:opacity-40"
            title="新会话：开一个全新的对话容器（旧会话保留在列表中）"
          >
            <RotateCcw size={10} />
            新会话
          </button>
        )}
      </div>
    </div>
  )
}
