import { Focus, FoldVertical, MessagesSquare, RefreshCw, UnfoldVertical } from 'lucide-react'
import { useLang } from '@/runtime/i18n'

// 选中模块时的横向工具栏（F1a）
export function ModuleToolbar({
  moduleName,
  expanded,
  backendActive,
  inducing,
  solo,
  onToggleExpand,
  onToggleSolo,
  onReinduce,
  onChat,
}: {
  moduleName: string
  expanded: boolean
  backendActive: boolean
  inducing: boolean
  solo: boolean
  onToggleExpand: () => void
  onToggleSolo: () => void
  onReinduce: () => void
  /** v0.2：就此模块对话——带上下文跳「任务对话」页 */
  onChat?: () => void
}) {
  const { t } = useLang()
  return (
    <div className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 bg-white/95 dark:bg-slate-900/95 py-1.5 pl-4 pr-2 shadow-sm backdrop-blur">
      <span className="mr-1 max-w-[180px] truncate text-[12px] font-bold text-slate-700 dark:text-slate-200">{moduleName}</span>
      {onChat && (
        <button
          onClick={onChat}
          className="flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full border px-3 py-1 text-[11px] font-semibold transition-colors border-transparent text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700/70"
          title={t('canvas.toolbar.chatTip')}
        >
          <MessagesSquare size={12} />
          {t('canvas.toolbar.chat')}
        </button>
      )}
      <button
        onClick={onToggleExpand}
        className={`flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full border px-3 py-1 text-[11px] font-semibold transition-colors ${
          expanded ? 'border-blue-300 dark:border-blue-800 bg-blue-50 dark:bg-blue-950/40 text-blue-600' : 'border-transparent text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700/70'
        }`}
      >
        {expanded ? <FoldVertical size={12} /> : <UnfoldVertical size={12} />}
        {expanded ? t('canvas.toolbar.collapse') : t('canvas.toolbar.expand')}
      </button>
      <button
        onClick={onToggleSolo}
        className={`flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full border px-3 py-1 text-[11px] font-semibold transition-colors ${
          solo ? 'border-indigo-300 dark:border-indigo-800 bg-indigo-50 dark:bg-indigo-950/40 text-indigo-600' : 'border-transparent text-slate-500 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-700/70'
        }`}
      >
        <Focus size={12} />
        {solo ? t('canvas.toolbar.exitSolo') : t('canvas.toolbar.solo')}
      </button>
      <button
        onClick={onReinduce}
        disabled={!backendActive || inducing}
        title={
          !backendActive
            ? t('canvas.toolbar.reinduceOffline')
            : inducing
              ? t('canvas.toolbar.reinduceBusy')
              : t('canvas.toolbar.reinduceTip')
        }
        className={`flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full border px-3 py-1 text-[11px] font-semibold transition-colors ${
          inducing
            ? 'cursor-wait border-amber-300 dark:border-amber-800 bg-amber-50 dark:bg-amber-950/40 text-amber-700'
            : backendActive
              ? 'border-transparent text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700/70'
              : 'cursor-not-allowed border-transparent text-slate-300 dark:text-slate-600'
        }`}
      >
        <RefreshCw size={12} className={inducing ? 'animate-spin' : ''} />
        {inducing ? t('canvas.toolbar.inducing') : t('canvas.toolbar.reinduce')}
      </button>
    </div>
  )
}
