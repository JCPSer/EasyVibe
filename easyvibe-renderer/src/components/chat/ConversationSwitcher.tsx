// 会话切换器（AionUI 模式：三态行 + 待审批角标 + 重命名/新建）。
// 拆自 ChatPanel.tsx（2026-10-05 防膨胀）。
import { AlertTriangle, ChevronDown, Loader2, Pencil, Plus } from 'lucide-react'
import { useLang } from '@/runtime/i18n'
import type { ConversationSummary } from './types'

export function ConversationSwitcher({
  currentConv, displayTitle, convs, convId, convMenuOpen, setConvMenuOpen,
  switchConv, renaming, renameVal, setRenameVal, renameConv, setRenaming, createConv,
}: {
  currentConv: ConversationSummary | undefined
  displayTitle: string
  convs: ConversationSummary[]
  convId: string | null
  convMenuOpen: boolean
  setConvMenuOpen: (v: boolean | ((p: boolean) => boolean)) => void
  switchConv: (id: string | null) => void
  renaming: boolean
  renameVal: string
  setRenameVal: (v: string) => void
  renameConv: () => void
  setRenaming: (v: boolean) => void
  createConv: () => void
}) {
  const { t } = useLang()
  return (
    <div className="relative mb-2">
      <button
        onClick={() => setConvMenuOpen((v) => !v)}
        className="flex w-full items-center gap-2 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1.5 text-left hover:border-blue-300"
      >
        {currentConv?.runtime.state === 'running' ? (
          <Loader2 size={12} className="shrink-0 animate-spin text-amber-500" />
        ) : currentConv && currentConv.runtime.pendingConfirmations > 0 ? (
          <AlertTriangle size={12} className="shrink-0 text-amber-500" />
        ) : (
          <span className="h-2 w-2 shrink-0 rounded-full bg-slate-300" />
        )}
        <span className="min-w-0 flex-1 truncate text-[12px] font-semibold text-slate-700 dark:text-slate-200">{displayTitle}</span>
        {currentConv && currentConv.runtime.pendingConfirmations > 0 && (
          <span className="rounded-full bg-red-500 px-1.5 text-micro font-bold leading-4 text-white">
            {currentConv.runtime.pendingConfirmations}
          </span>
        )}
        <ChevronDown size={12} className="shrink-0 text-slate-400 dark:text-slate-500" />
      </button>
      {convMenuOpen && (
        <>
          <div className="fixed inset-0 z-30" onClick={() => setConvMenuOpen(false)} />
          <div className="glass absolute left-0 right-0 top-full z-40 mt-1 max-h-72 overflow-y-auto rounded-xl border border-slate-200 dark:border-slate-700 p-1.5 shadow-xl anim-scale-in">
            {convs.map((c) => (
              <div key={c.id} className="flex items-center gap-1.5 rounded-lg px-2 py-1.5 hover:bg-slate-50 dark:hover:bg-slate-800/70">
                <button
                  className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
                  onClick={() => switchConv(c.id === convId ? null : c.id)}
                >
                  {c.runtime.state === 'running' ? (
                    <Loader2 size={11} className="shrink-0 animate-spin text-amber-500" />
                  ) : c.runtime.pendingConfirmations > 0 ? (
                    <AlertTriangle size={11} className="shrink-0 text-amber-500" />
                  ) : (
                    <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-slate-300" />
                  )}
                  <span className={`min-w-0 flex-1 truncate text-[12px] ${c.id === convId ? 'font-bold text-blue-700' : 'text-slate-600 dark:text-slate-300'}`}>
                    {c.title ?? t('chat.untitled')}
                  </span>
                  {c.runtime.pendingConfirmations > 0 && (
                    <span className="rounded-full bg-red-500 px-1.5 text-micro font-bold leading-4 text-white">{c.runtime.pendingConfirmations}</span>
                  )}
                  <span className="tnum shrink-0 text-micro text-slate-300 dark:text-slate-600">{t('chat.msgCount', { n: c.messageCount })}</span>
                </button>
                {c.id === convId && (
                  <button onClick={() => { setRenaming(true); setRenameVal(c.title ?? '') }} className="shrink-0 rounded p-0.5 text-slate-300 dark:text-slate-600 hover:text-blue-500" title={t('chat.rename')}>
                    <Pencil size={10} />
                  </button>
                )}
              </div>
            ))}
            {convs.length === 0 && <p className="px-2 py-1.5 text-[11px] text-slate-400 dark:text-slate-500">{t('chat.noConvs')}</p>}
            <button
              onClick={createConv}
              className="mt-1 flex w-full items-center justify-center gap-1 rounded-lg bg-blue-600 px-2 py-1.5 text-[11px] font-semibold text-white hover:bg-blue-700"
            >
              <Plus size={11} /> {t('chat.createConv')}
            </button>
          </div>
        </>
      )}
      {renaming && (
        <div className="absolute left-0 right-0 top-full z-50 mt-1 flex items-center gap-1 rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-2 shadow-xl">
          <input
            autoFocus
            value={renameVal}
            onChange={(e) => setRenameVal(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') renameConv()
              if (e.key === 'Escape') setRenaming(false)
            }}
            placeholder={t('chat.convNamePh')}
            className="flex-1 rounded-lg border border-slate-200 dark:border-slate-700 px-2 py-1 text-[12px] outline-none focus:border-blue-300"
          />
          <button onClick={renameConv} className="rounded-lg bg-blue-600 px-2.5 py-1 text-[11px] font-bold text-white">{t('chat.saveBtn')}</button>
        </div>
      )}
    </div>
  )
}
