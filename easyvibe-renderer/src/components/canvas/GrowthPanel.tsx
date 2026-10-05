import { Pause, Play, RotateCcw, Sparkles, X } from 'lucide-react'
import type { GrowthState } from './types'

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
  const total = growth.events.length
  const pct = total === 0 ? 0 : Math.round((growth.index / total) * 100)
  const cur = growth.index < growth.events.length ? growth.events[growth.index] : null
  const status = growth.done
    ? '归纳完成'
    : total === 0
      ? '等待生长事件…'
    : cur?.type === 'layer'
      ? `分层：${cur.layer.name}`
      : cur?.type === 'module'
        ? `正在分析模块：${cur.module.name}`
        : cur?.type === 'arch_health'
          ? '架构级健康评估'
          : '初始化'

  return (
    <div className="flex w-[460px] items-center gap-3 rounded-xl border border-slate-200 dark:border-slate-700 bg-white/95 dark:bg-slate-900/95 px-4 py-2.5 shadow-sm backdrop-blur">
      <button
        onClick={onPause}
        disabled={growth.done}
        className="rounded-full p-1.5 text-slate-500 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-700 disabled:opacity-30"
        title={growth.playing ? '暂停' : '继续'}
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
      <button onClick={onRestart} className="rounded-full p-1.5 text-slate-400 dark:text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-600" title="重播">
        <RotateCcw size={13} />
      </button>
      <button onClick={onExit} className="rounded-full p-1.5 text-slate-400 dark:text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-600" title="退出演示">
        <X size={14} />
      </button>
    </div>
  )
}
