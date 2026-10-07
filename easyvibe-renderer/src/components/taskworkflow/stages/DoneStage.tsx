// done：归档摘要（变更统计 + 归档路径复制 + 重新巡检验证入口）。
// 拆自 TaskWorkflowPage.tsx（2026-10-05 防膨胀）。
import { CheckCircle2, Copy } from 'lucide-react'
import { toast } from '@/runtime/toast'
import { useLang } from '@/runtime/i18n'
import { absTime } from '@/shared/logic/diffStat'
import { patrol } from '@/api/canvas'
import type { TaskItem } from '../types'

export function DoneStage({ sel, backendRepo, impact }: {
  sel: TaskItem
  backendRepo: string
  impact: { adds: number; dels: number; files: number }[]
}) {
  const { t } = useLang()
  return (
    <div className="m-4 overflow-y-auto rounded-xl border border-emerald-200 dark:border-emerald-900/60 bg-emerald-50/40 dark:bg-emerald-950/30 p-4">
      <p className="flex items-center gap-1 text-micro font-bold uppercase tracking-wider text-emerald-600">
        <CheckCircle2 size={10} /> {t('task.archived')}
      </p>
      {impact.map((im, i) => (
        <p key={i} className="tnum mt-2 text-micro text-slate-500 dark:text-slate-400">
          {t('task.doneLine', { adds: im.adds, dels: im.dels, files: im.files, time: absTime(sel.updatedAt ?? '') })}
        </p>
      ))}
      {/* TaskPanel 碎片②：归档路径（P1 留在 done 卡，P2 挪治理视图）——只显示仓库内相对路径，不暴露本机绝对路径 */}
      {(() => {
        const ap = (sel as unknown as { result?: { archivedPath?: string | null } }).result?.archivedPath
        if (!ap) return null
        const rel = ap.includes('/.easyvibe/') ? `.easyvibe/${ap.split('/.easyvibe/')[1]}` : ap.split('/').pop()
        return (
          <p className="mono mt-1 flex items-center gap-1 text-micro text-slate-400 dark:text-slate-500">
            <span className="truncate" title={rel}>{t('task.archivedPrefix', { path: rel ?? '' })}</span>
            {/* 审计 P2：归档路径可复制（此前只能眼看） */}
            <button
              onClick={() => {
                void navigator.clipboard?.writeText(rel ?? '').then(
                  () => toast('归档路径已复制', 'info'),
                  () => toast('复制失败（剪贴板不可用）', 'error'),
                )
              }}
              className="shrink-0 rounded p-0.5 text-slate-300 dark:text-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-500"
              title={t('task.copyArchivedTip')}
            >
              <Copy size={9} />
            </button>
          </p>
        )
      })()}
      <p className="mt-1 text-micro text-slate-400 dark:text-slate-500">{t('task.starNote')}</p>
      {/* TaskPanel 碎片①：重新巡检验证改动效果（治理闭环入口不能丢） */}
      <button
        onClick={() => {
          if (!backendRepo) return
          patrol(backendRepo).catch(() => {})
          toast(t('task.repatrolToast'), 'info')
        }}
        className="mt-2 rounded-lg border border-emerald-300 dark:border-emerald-800 bg-white dark:bg-slate-900 px-3 py-1.5 text-micro font-semibold text-emerald-700 hover:bg-emerald-50 dark:hover:bg-emerald-950/40"
      >
        重新巡检验证改动效果
      </button>
    </div>
  )
}
