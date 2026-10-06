// SessionBubble 悬停详情卡（纯展示拆件）：跨仓库列出全部活动会话（仓库 · label · 起止 · 各自取消）。
// 拆分动机（i18n 第一批）：翻译后主件需压在 repoLayout overlays ≤300 线内。
import { X } from 'lucide-react'
import { absTime, toMs } from '@/shared/logic/diffStat'
import { formatElapsed } from '@/runtime/sessionQueue'
import { useLang } from '@/lib/i18n'

export interface SessionActive {
  sessionId: string
  repo: string
  label: string
  startedAt?: string | null
}

export function SessionBubbleCard({ activeList, primaryStartedMs, now, onKill }: {
  activeList: SessionActive[]
  primaryStartedMs: number | null
  now: number
  onKill: (a: { sessionId: string; repo: string; label: string }) => void
}) {
  const { t } = useLang()
  return (
    <div className="pointer-events-none invisible absolute left-1/2 top-full z-50 mt-1.5 w-72 -translate-x-1/2 opacity-0 transition-opacity duration-150 group-hover:pointer-events-auto group-hover:visible group-hover:opacity-100">
      <div className="glass elev-3 anim-scale-in rounded-xl border border-slate-200 p-3 dark:border-slate-700">
        <p className="text-cap font-bold text-slate-700 dark:text-slate-200">
          {t('shell.bubble.cardTitle', { count: activeList.length })}
        </p>
        <div className="mt-1.5 space-y-2">
          {activeList.map((a) => {
            const st = a.startedAt ? toMs(a.startedAt) : null
            return (
              <div key={a.sessionId} className="rounded-lg bg-slate-50 dark:bg-slate-950/70 px-2.5 py-2">
                <div className="flex items-center gap-1.5">
                  <span className="min-w-0 flex-1 truncate text-cap font-semibold text-slate-700 dark:text-slate-200">{a.label}</span>
                  <span className="shrink-0 rounded bg-blue-50 dark:bg-blue-950/40 px-1 py-px text-micro font-semibold text-blue-600 dark:text-blue-300">
                    {a.repo}
                  </span>
                </div>
                <div className="mt-0.5 flex items-center justify-between text-micro text-slate-500 dark:text-slate-400">
                  <span className="mono">{a.sessionId}</span>
                  <span className="tnum">
                    {st !== null ? t('shell.bubble.elapsed', { elapsed: formatElapsed(now - st) }) : '…'}
                  </span>
                </div>
                <button
                  onClick={() => onKill(a)}
                  className="mt-1.5 flex w-full items-center justify-center gap-1 rounded-md border border-red-200 bg-red-50 px-2 py-1 text-micro font-semibold text-red-600 transition-colors hover:bg-red-100 dark:border-red-900/60 dark:bg-red-950/40 dark:text-red-300 dark:hover:bg-red-900/40"
                  title={t('shell.bubble.cancelSessionTip')}
                >
                  <X size={10} />
                  {t('shell.bubble.cancelSession')}
                </button>
              </div>
            )
          })}
        </div>
        {primaryStartedMs !== null && (
          <p className="mt-2 flex justify-between text-micro text-slate-400 dark:text-slate-500">
            <span>{t('shell.bubble.startedAt')}</span>
            <span className="tnum">{absTime(new Date(primaryStartedMs).toISOString()).slice(5)}</span>
          </p>
        )}
      </div>
    </div>
  )
}
