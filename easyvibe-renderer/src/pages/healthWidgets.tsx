// 健康看板小组件（HealthPage 拆出，防膨胀）：迷你圆环 + KPI 涨跌指示。
import { TrendingDown, TrendingUp } from 'lucide-react'
import { healthColor } from '@/shared/logic/layout'
import { useLang } from '@/runtime/i18n'

/** 迷你圆环（KPI 与排行共用）：score 0-100；NaN 入参会污染全部 SVG 坐标（ui-test P2 控制台报错来源） */
export function Ring({ score, size = 44 }: { score: number; size?: number }) {
  const safe = Number.isFinite(score) ? score : 0
  const r = (size - 6) / 2
  const c = 2 * Math.PI * r
  const pct = Math.max(0, Math.min(100, safe)) / 100
  const color = healthColor(safe)
  return (
    <svg width={size} height={size} className="-rotate-90">
      <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="#f1f5f9" strokeWidth={4} />
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        stroke={color}
        strokeWidth={4}
        strokeLinecap="round"
        strokeDasharray={`${c * pct} ${c}`}
        className="transition-all duration-500"
      />
    </svg>
  )
}

/** KPI 卡片上的环比涨跌（与上一次成功巡检比） */
export function Delta({ now, prev }: { now: number; prev: number | null }) {
  const { t } = useLang()
  if (prev === null) return <p className="mt-0.5 text-micro text-slate-300 dark:text-slate-600">{t('pages.health.deltaNone')}</p>
  const d = now - prev
  if (d === 0) return <p className="mt-0.5 flex items-center gap-0.5 text-micro text-slate-400 dark:text-slate-500">{t('pages.health.deltaSame')}</p>
  const up = d > 0
  return (
    <p className={`mt-0.5 flex items-center gap-0.5 text-micro ${up ? 'text-emerald-500' : 'text-red-500'}`}>
      {up ? <TrendingUp size={10} /> : <TrendingDown size={10} />}
      {t('pages.health.delta', { delta: `${up ? '+' : ''}${d}` })}
    </p>
  )
}
