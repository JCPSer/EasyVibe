import { Pause, Play, RotateCcw, Sparkles, X } from 'lucide-react'
import type { GrowthState } from './types'
import { useLang } from '@/runtime/i18n'

// 生长回放控制条（消费 v2.2 growth.log 事件流）
export function GrowthPanel({
  growth,
  onPause,
  onRestart,
  onExit,
}: {
  growth: GrowthState
  onPause: () => void
  onRestart: () => void
  onExit: () => void
}) {
  const { t } = useLang()
  const total = growth.events.length
  const pct = total === 0 ? 0 : Math.round((growth.index / total) * 100)
  const cur = growth.index < growth.events.length ? growth.events[growth.index] : null
  const status = growth.done
    ? t('canvas.growth.done')
    : total === 0
      ? t('canvas.growth.waiting')
    : cur?.type === 'layer'
      ? t('canvas.growth.layer', { name: cur.layer.name })
      : cur?.type === 'module'
        ? t('canvas.growth.module', { name: cur.module.name })
        : cur?.type === 'arch_health'
          ? t('canvas.growth.archHealth')
          : t('canvas.growth.init')

  return (
    <div className="flex w-[460px] items-center gap-3 rounded-xl border border-slate-200 dark:border-slate-700 bg-white/95 dark:bg-slate-900/95 px-4 py-2.5 shadow-sm backdrop-blur">
      <button
        onClick={onPause}
        disabled={growth.done}
        className="rounded-full p-1.5 text-slate-500 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-700 disabled:opacity-30"
        title={growth.playing ? t('canvas.growth.pauseTip') : t('canvas.growth.resumeTip')}
      >
        {growth.playing ? <Pause size={14} /> : <Play size={14} />}
      </button>
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between">
          <span className="truncate text-[12px] font-semibold text-slate-700 dark:text-slate-200">
            <Sparkles size={11} className="mr-1 inline text-blue-500" />
            {status}
          </span>
          <span className="text-micro tabular-nums text-slate-400 dark:text-slate-500">{pct}%</span>
        </div>
        <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800">
          <div
            className={`h-full rounded-full transition-all duration-500 ${growth.done ? 'bg-emerald-500' : 'bg-blue-500'}`}
            style={{ width: `${pct}%` }}
          />
        </div>
      </div>
      <button onClick={onRestart} className="rounded-full p-1.5 text-slate-400 dark:text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-600" title={t('canvas.growth.replayTip')}>
        <RotateCcw size={13} />
      </button>
      <button onClick={onExit} className="rounded-full p-1.5 text-slate-400 dark:text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-600" title={t('canvas.growth.exitTip')}>
        <X size={14} />
      </button>
    </div>
  )
}
