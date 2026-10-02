import { useCallback, useEffect, useMemo, useState } from 'react'
import { HeartPulse, Loader2, Play, TrendingDown, TrendingUp } from 'lucide-react'
import type { CodeMap } from '@/types/map'
import { healthColor } from '@/lib/layout'
import { absTime } from '@/lib/diffStat'

// M4-3 健康看板整页（按 ui-mockups/健康看板原型.png 施工）：
// KPI×4（架构健康/模块平均/逆向依赖/覆盖率）+ 近 10 次巡检趋势图（架构级 vs 模块平均）
// + 最差模块排行 + 巡检记录表。数据面：GET /repos/{id}/health-dashboard（M4-3 新增聚合接口）
// + 主地图（逆向依赖/覆盖率无历史，取当前地图口径并在卡片上注明）。

interface DashboardRun {
  id: string
  startedAt: string
  finishedAt: string | null
  status: string
  model: string | null
  archScore: number | null
  moduleAvg: number
  moduleCount: number
}

interface LatestModule {
  moduleId: string
  name: string | null
  score: number
  decayFlags: string
}

interface Dashboard {
  runs: DashboardRun[]
  latestModules: LatestModule[]
}

/** 迷你圆环（KPI 与排行共用）：score 0-100 */
function Ring({ score, size = 44 }: { score: number; size?: number }) {
  const r = (size - 6) / 2
  const c = 2 * Math.PI * r
  const pct = Math.max(0, Math.min(100, score)) / 100
  const color = healthColor(score)
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

function Delta({ now, prev }: { now: number; prev: number | null }) {
  if (prev === null) return <p className="mt-0.5 text-micro text-slate-300">无上次记录</p>
  const d = now - prev
  if (d === 0) return <p className="mt-0.5 flex items-center gap-0.5 text-micro text-slate-400">较上次 +0</p>
  const up = d > 0
  return (
    <p className={`mt-0.5 flex items-center gap-0.5 text-micro ${up ? 'text-emerald-500' : 'text-red-500'}`}>
      {up ? <TrendingUp size={10} /> : <TrendingDown size={10} />}
      较上次 {up ? '+' : ''}
      {d}
    </p>
  )
}

export function HealthPage({ backendRepo, map }: { backendRepo: string | null; map: CodeMap | null }) {
  const [data, setData] = useState<Dashboard | null>(null)
  const [loading, setLoading] = useState(false)
  const [starting, setStarting] = useState(false)

  const load = useCallback(() => {
    if (!backendRepo) return
    setLoading(true)
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/health-dashboard`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: Dashboard } | null) => setData(d?.data ?? null))
      .catch(() => setData(null))
      .finally(() => setLoading(false))
  }, [backendRepo])

  useEffect(() => {
    load()
  }, [load])

  const succeeded = useMemo(
    () => (data?.runs ?? []).filter((r) => r.status === 'succeeded' && r.archScore !== null),
    [data],
  )
  const latest = succeeded[0] ?? null
  const prev = succeeded[1] ?? null

  // KPI 口径：巡检记录优先，无记录回落当前地图
  const archScore = latest?.archScore ?? map?.health.score ?? null
  const moduleAvg = latest && latest.moduleCount > 0 ? latest.moduleAvg : null
  const mapModuleAvg = useMemo(() => {
    if (!map || map.modules.length === 0) return null
    return Math.round(map.modules.reduce((s, m) => s + m.health.score, 0) / map.modules.length)
  }, [map])
  const reverseDeps = useMemo(() => map?.edges.filter((e) => e.direction_violation).length ?? null, [map])
  const coverage = map?.meta.stats?.coverage_ratio ?? null

  // 趋势图：近 10 次成功巡检，旧→新
  const trend = useMemo(() => [...succeeded].slice(0, 10).reverse(), [succeeded])
  const [hover, setHover] = useState<number | null>(null)

  const startPatrol = async () => {
    if (!backendRepo || starting) return
    setStarting(true)
    try {
      await fetch(`/api/repos/${encodeURIComponent(backendRepo)}/patrol`, { method: 'POST' })
    } finally {
      setStarting(false)
      setTimeout(load, 1500)
    }
  }

  if (!backendRepo) {
    return (
      <div className="flex h-full items-center justify-center text-[12px] text-slate-400">先在左侧选择一个项目。</div>
    )
  }

  const hasAnyRun = (data?.runs.length ?? 0) > 0

  return (
    <div className="h-full overflow-y-auto p-5">
      <div className="mb-4 flex items-start justify-between">
        <div>
          <h2 className="flex items-center gap-1.5 text-[15px] font-bold text-slate-800">
            <HeartPulse size={15} className="text-red-400" /> 健康看板
          </h2>
          <p className="mt-0.5 text-[11px] text-slate-400">
            这个仓库的体检报告：架构与模块健康分的趋势、最差模块排行、每次巡检的记录与结论。
          </p>
        </div>
        <button
          onClick={startPatrol}
          disabled={starting}
          className="flex items-center gap-1 rounded-lg border border-slate-200 bg-white px-2.5 py-1.5 text-[11px] font-semibold text-slate-500 hover:bg-slate-50 disabled:opacity-40"
        >
          {starting ? <Loader2 size={11} className="animate-spin" /> : <Play size={11} />} 发起巡检
        </button>
      </div>

      {/* KPI 行 */}
      <div className="mb-4 grid grid-cols-4 gap-3">
        <div className="lift relative rounded-xl border border-slate-200 bg-white px-4 py-3">
          <p className="text-cap text-slate-400">架构健康</p>
          <p className="tnum mt-1 text-[22px] font-bold leading-6" style={{ color: archScore !== null ? healthColor(archScore) : '#94a3b8' }}>
            {archScore ?? '—'} <span className="text-cap font-normal text-slate-300">/ 100</span>
          </p>
          {archScore !== null && <Delta now={archScore} prev={prev?.archScore ?? null} />}
          <div className="absolute right-3 top-1/2 -translate-y-1/2">{archScore !== null && <Ring score={archScore} />}</div>
        </div>
        <div className="lift relative rounded-xl border border-slate-200 bg-white px-4 py-3">
          <p className="text-cap text-slate-400">模块平均</p>
          <p className="tnum mt-1 text-[22px] font-bold leading-6" style={{ color: healthColor(moduleAvg ?? mapModuleAvg ?? 0) }}>
            {moduleAvg ?? mapModuleAvg ?? '—'} <span className="text-cap font-normal text-slate-300">/ 100</span>
          </p>
          {moduleAvg !== null && <Delta now={moduleAvg} prev={prev && prev.moduleCount > 0 ? prev.moduleAvg : null} />}
          {moduleAvg === null && <p className="mt-0.5 text-micro text-slate-300">{mapModuleAvg !== null ? '当前地图口径' : '暂无数据'}</p>}
          <div className="absolute right-3 top-1/2 -translate-y-1/2">
            {(moduleAvg ?? mapModuleAvg) !== null && <Ring score={(moduleAvg ?? mapModuleAvg)!} />}
          </div>
        </div>
        <div className="lift relative rounded-xl border border-slate-200 bg-white px-4 py-3">
          <p className="text-cap text-slate-400">逆向依赖</p>
          <p className="tnum mt-1 text-[22px] font-bold leading-6 text-red-500">{reverseDeps ?? '—'}</p>
          <p className="mt-0.5 text-micro text-slate-300">当前地图口径</p>
        </div>
        <div className="lift relative rounded-xl border border-slate-200 bg-white px-4 py-3">
          <p className="text-cap text-slate-400">覆盖率</p>
          <p className="tnum mt-1 text-[22px] font-bold leading-6 text-emerald-500">
            {coverage !== null ? `${Math.round(coverage * 100)}` : '—'}
            <span className="text-[12px]"> %</span>
          </p>
          <p className="mt-0.5 text-micro text-slate-300">当前地图口径</p>
        </div>
      </div>

      {!hasAnyRun && !loading && (
        <div className="mb-4 rounded-xl border border-dashed border-slate-200 bg-white/60 px-4 py-8 text-center">
          <p className="text-[12px] font-semibold text-slate-500">还没有任何巡检记录</p>
          <p className="mt-1 text-[11px] text-slate-400">发起一次巡检，让 EasyVibe 给这个仓库做一次全面体检。</p>
        </div>
      )}

      <div className="grid grid-cols-3 gap-3">
        {/* 趋势图 */}
        <div className="col-span-2 rounded-xl border border-slate-200 bg-white p-4">
          <div className="mb-2 flex items-center justify-between">
            <span className="text-[13px] font-bold text-slate-700">架构健康趋势（近 {Math.min(10, trend.length)} 次巡检）</span>
            <div className="flex items-center gap-3 text-micro text-slate-400">
              <span className="flex items-center gap-1"><i className="h-[3px] w-4 rounded bg-blue-500" /> 架构级</span>
              <span className="flex items-center gap-1"><i className="h-[3px] w-4 rounded bg-slate-300" /> 模块平均</span>
            </div>
          </div>
          {trend.length >= 2 ? (
            <TrendChart runs={trend} hover={hover} setHover={setHover} />
          ) : (
            <div className="flex h-[180px] items-center justify-center text-[11px] text-slate-300">
              至少两次成功巡检后绘制趋势
            </div>
          )}
        </div>

        {/* 最差模块排行 */}
        <div className="rounded-xl border border-slate-200 bg-white p-4">
          <p className="mb-2 text-[13px] font-bold text-slate-700">模块健康排行</p>
          <div className="space-y-1">
            {(data?.latestModules ?? []).slice(0, 10).map((m, i) => (
              <div key={m.moduleId} className="flex items-center gap-2 rounded-lg px-2 py-1.5 hover:bg-slate-50">
                <span className="tnum w-4 text-micro text-slate-300">{i + 1}</span>
                <span className="min-w-0 flex-1 truncate text-[12px] text-slate-600">{m.name ?? m.moduleId}</span>
                <div className="relative">
                  <Ring score={m.score} size={26} />
                  <span
                    className="tnum absolute inset-0 flex items-center justify-center text-[8px] font-bold"
                    style={{ color: healthColor(m.score) }}
                  >
                    {m.score}
                  </span>
                </div>
              </div>
            ))}
            {(data?.latestModules.length ?? 0) === 0 && (
              <p className="py-6 text-center text-[11px] text-slate-300">暂无模块巡检明细</p>
            )}
          </div>
        </div>
      </div>

      {/* 巡检记录 */}
      <div className="mt-3 overflow-hidden rounded-xl border border-slate-200 bg-white">
        <div className="border-b border-slate-100 px-4 py-2.5">
          <span className="text-[13px] font-bold text-slate-700">巡检记录</span>
        </div>
        <table className="w-full text-left">
          <thead>
            <tr className="border-b border-slate-100 bg-slate-50/60 text-[11px] font-semibold text-slate-400">
              <th className="px-4 py-2">时间</th>
              <th className="px-3 py-2">模型</th>
              <th className="px-3 py-2 text-right">架构分</th>
              <th className="px-3 py-2 text-right">模块平均</th>
              <th className="px-3 py-2 text-right">模块数</th>
              <th className="px-3 py-2">状态</th>
            </tr>
          </thead>
          <tbody>
            {(data?.runs ?? []).map((r) => (
              <tr key={r.id} className="border-b border-slate-50 last:border-0 hover:bg-slate-50/60">
                <td className="tnum px-4 py-2.5 text-[12px] text-slate-600">{absTime(r.startedAt)}</td>
                <td className="px-3 py-2.5 text-[12px] text-slate-500">{r.model ?? '—'}</td>
                <td className="tnum px-3 py-2.5 text-right text-[12px] font-semibold" style={{ color: r.archScore !== null ? healthColor(r.archScore) : '#94a3b8' }}>
                  {r.archScore ?? '—'}
                </td>
                <td className="tnum px-3 py-2.5 text-right text-[12px] text-slate-600">{r.moduleCount > 0 ? r.moduleAvg : '—'}</td>
                <td className="tnum px-3 py-2.5 text-right text-[12px] text-slate-400">{r.moduleCount > 0 ? r.moduleCount : '—'}</td>
                <td className="px-3 py-2.5">
                  <span
                    className={`rounded-full px-1.5 py-px text-micro font-semibold ${
                      r.status === 'succeeded'
                        ? 'bg-emerald-50 text-emerald-600'
                        : r.status === 'failed'
                          ? 'bg-red-50 text-red-500'
                          : 'bg-amber-50 text-amber-600'
                    }`}
                  >
                    {r.status === 'succeeded' ? '成功' : r.status === 'failed' ? '失败' : '运行中'}
                  </span>
                </td>
              </tr>
            ))}
            {!hasAnyRun && (
              <tr>
                <td colSpan={6} className="px-4 py-8 text-center text-[12px] text-slate-300">{loading ? '加载中…' : '暂无巡检记录'}</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  )
}

/** 趋势折线图：纯 SVG，x=巡检序、y=0-100 分，悬停显示单点明细 */
function TrendChart({
  runs,
  hover,
  setHover,
}: {
  runs: DashboardRun[]
  hover: number | null
  setHover: (i: number | null) => void
}) {
  const W = 640
  const H = 180
  const PAD = { l: 30, r: 12, t: 12, b: 22 }
  const iw = W - PAD.l - PAD.r
  const ih = H - PAD.t - PAD.b
  const x = (i: number) => PAD.l + (runs.length === 1 ? iw / 2 : (i / (runs.length - 1)) * iw)
  const y = (s: number) => PAD.t + (1 - Math.max(0, Math.min(100, s)) / 100) * ih
  const line = (key: 'archScore' | 'moduleAvg') =>
    runs.map((r, i) => `${i === 0 ? 'M' : 'L'}${x(i).toFixed(1)},${y(key === 'archScore' ? (r.archScore ?? 0) : r.moduleAvg).toFixed(1)}`).join(' ')
  const h = hover !== null ? runs[hover] : null

  return (
    <div className="relative">
      <svg viewBox={`0 0 ${W} ${H}`} className="w-full">
        {[0, 25, 50, 75, 100].map((g) => (
          <g key={g}>
            <line x1={PAD.l} x2={W - PAD.r} y1={y(g)} y2={y(g)} stroke={g === 0 ? '#e2e8f0' : '#f1f5f9'} strokeWidth={1} />
            <text x={PAD.l - 5} y={y(g) + 3} textAnchor="end" fontSize={8} fill="#cbd5e1" className="tnum">
              {g}
            </text>
          </g>
        ))}
        <path d={line('moduleAvg')} fill="none" stroke="#cbd5e1" strokeWidth={1.5} strokeDasharray="3 3" />
        <path d={line('archScore')} fill="none" stroke="#3b82f6" strokeWidth={2} />
        {runs.map((r, i) => (
          <g key={r.id}>
            <rect x={x(i) - 14} y={0} width={28} height={H} fill="transparent" onMouseEnter={() => setHover(i)} onMouseLeave={() => setHover(null)} />
            <circle cx={x(i)} cy={y(r.archScore ?? 0)} r={hover === i ? 4 : 2.5} fill="#fff" stroke="#3b82f6" strokeWidth={2} />
            <circle cx={x(i)} cy={y(r.moduleAvg)} r={hover === i ? 3.5 : 2} fill="#fff" stroke="#cbd5e1" strokeWidth={2} />
            {i % Math.ceil(runs.length / 8) === 0 && (
              <text x={x(i)} y={H - 6} textAnchor="middle" fontSize={8} fill="#cbd5e1">
                {r.startedAt.slice(5, 10)}
              </text>
            )}
          </g>
        ))}
      </svg>
      {h && (
        <div
          className="glass pointer-events-none absolute z-10 rounded-lg border border-slate-200 bg-white/95 px-2.5 py-1.5 text-micro shadow-lg"
          style={{ left: `${(x(hover!) / W) * 100}%`, top: 0, transform: `translateX(${hover! > runs.length / 2 ? '-110%' : '10%'})` }}
        >
          <p className="tnum font-bold text-blue-600">{h.archScore ?? '—'} <span className="font-normal text-slate-400">架构级</span></p>
          <p className="tnum text-slate-500">{h.moduleCount > 0 ? h.moduleAvg : '—'} <span className="font-normal text-slate-300">模块平均</span></p>
          <p className="tnum text-slate-300">{h.startedAt.slice(0, 10)}</p>
        </div>
      )}
    </div>
  )
}
