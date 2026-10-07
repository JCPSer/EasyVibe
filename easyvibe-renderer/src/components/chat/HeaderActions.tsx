// 头部状态行（token 统计）+ 操作按钮组：删会话/导出清空/转任务/压缩上下文/新会话。
// 拆自 ChatPanel.tsx（2026-10-05 防膨胀）。
import { Download, RotateCcw, Shrink, Wrench, X as XIcon } from 'lucide-react'
import { useLang } from '@/runtime/i18n'

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
  const { t } = useLang()
  return (
    <div className="mb-2 flex items-center justify-between">
      <span className="tnum text-micro text-slate-400 dark:text-slate-500">
        {t('chat.persistent', { in: usage.promptTokens.toLocaleString(), out: usage.completionTokens.toLocaleString() })}
      </span>
      <div className="flex flex-wrap items-center justify-end gap-1">
        {convId && !embedded && (
          <button
            onClick={onDelete}
            disabled={!backendRepo}
            className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-0.5 text-micro text-slate-500 dark:text-slate-400 shadow-sm hover:bg-red-50 dark:hover:bg-red-950/40 hover:text-red-600 disabled:opacity-40"
            title={t('chat.deleteConvTip')}
          >
            <XIcon size={10} />
            {t('chat.deleteConv')}
          </button>
        )}
        <button
          onClick={onExportClear}
          disabled={!backendRepo || messageCount === 0}
          className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-0.5 text-micro text-slate-500 dark:text-slate-400 shadow-sm hover:bg-slate-50 dark:hover:bg-slate-800/70 disabled:opacity-40"
          title={t('chat.exportClearTip')}
        >
          <Download size={10} />
          {t('chat.exportClear')}
        </button>
        <button
          onClick={onUpgrade}
          disabled={!backendRepo || sending || !hasUserMessage}
          className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-0.5 text-micro text-slate-500 dark:text-slate-400 shadow-sm hover:bg-blue-50 dark:hover:bg-blue-950/40 hover:text-blue-600 disabled:opacity-40"
          title={t('chat.toTaskTip')}
        >
          <Wrench size={10} />
          {t('chat.toTask')}
        </button>
        <button
          onClick={onCompress}
          disabled={!backendRepo || compacting}
          className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-0.5 text-micro text-slate-500 dark:text-slate-400 shadow-sm hover:bg-slate-50 dark:hover:bg-slate-800/70 disabled:opacity-40"
          title={t('chat.compressTip')}
        >
          <Shrink size={10} />
          {compacting ? t('chat.compressing') : t('chat.compress')}
        </button>
        {/* v0.2：embedded 模式隐藏"新会话"——会话创建统一走工作台左栏（列表即管理），
            消除"预填/新会话落在哪"的双入口解释成本 */}
        {!embedded && (
          <button
            onClick={onCreate}
            disabled={!backendRepo || sending}
            className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-0.5 text-micro text-slate-500 dark:text-slate-400 shadow-sm hover:bg-slate-50 dark:hover:bg-slate-800/70 disabled:opacity-40"
            title={t('chat.newConvTip')}
          >
            <RotateCcw size={10} />
            {t('chat.newConv')}
          </button>
        )}
      </div>
    </div>
  )
}
