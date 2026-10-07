import { useCallback, useEffect, useMemo, useState } from 'react'
import { HeartPulse, Loader2, Play, TrendingDown, TrendingUp, Wrench, Trash2, X } from 'lucide-react'
import type { CodeMap } from '@/types/map'
import { healthColor } from '@/shared/logic/layout'
import { absTime, toMs } from '@/shared/logic/diffStat'
import { Select } from '@/components/ui/SelectMenu'
import { buildModuleTask, type TaskDraft } from '@/shared/logic/taskContext'
import { onPatrolFinished } from '@/runtime/growthBus'
import { enqueue } from '@/runtime/sessionQueue'
import { toast } from '@/runtime/toast'
import { useLang } from '@/runtime/i18n'
import { prunePatrolRuns, healthDashboard } from '@/api/repos'
import { patrol } from '@/api/canvas'

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

/** 迷你圆环（KPI 与排行共用）：score 0-100；NaN 入参会污染全部 SVG 坐标（ui-test P2 控制台报错来源） */
function Ring({ score, size = 44 }: { score: number; size?: number }) {
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

function Delta({ now, prev }: { now: number; prev: number | null }) {
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

export function HealthPage({
  backendRepo,
  map,
  onCreateTask,
  onOpenDeps,
}: {
  backendRepo: string | null
  map: CodeMap | null
  /** 把关台范式：排行行内直接派单修复（TaskDraft → 全局任务表单） */
  onCreateTask: (d: TaskDraft) => void
  /** 2026-10-05 评审 S3：逆向依赖 KPI 转链依赖体检页（带筛选），消除双地讲一个数 */
  onOpenDeps?: () => void
}) {
  const { t } = useLang()
  const [data, setData] = useState<Dashboard | null>(null)
  const [loading, setLoading] = useState(false)
  const [starting, setStarting] = useState(false)
  // 重审 P1：巡检历史清理（只保留最近 N 次）
  const [confirmPrune, setConfirmPrune] = useState(false)
  const [pruneKeep, setPruneKeep] = useState(10)
  const [pruning, setPruning] = useState(false)

  const prune = async () => {
    if (!backendRepo || pruning) return
    setPruning(true)
    try {
      const r = await prunePatrolRuns(backendRepo, pruneKeep)
      const d = await r.json().catch(() => null)
      if (!r.ok) throw new Error(d?.error ?? t('pages.health.pruneFailed'))
      toast(t('pages.health.pruned', { count: d?.data?.deleted ?? 0, keep: pruneKeep }))
      setConfirmPrune(false)
      load()
    } catch (e) {
      toast(e instanceof Error ? e.message : t('pages.health.pruneFailed'), 'error')
    } finally {
      setPruning(false)
    }
  }

  const load = useCallback(() => {
    if (!backendRepo) return
    setLoading(true)
    healthDashboard(backendRepo)
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

  // KPI 口径（2026-10-06 实弹修正）：按"测量时间新旧"裁决，不再无条件巡检优先——
  // 归纳后新地图的自评分是对**当前代码**的最新证据，应盖过针对**旧代码**的巡检分
  // （实弹：巡检 89 后重新归纳，地图自评 74，看板仍报 89——双地讲一个数，用户不知信谁）。
  // 来源必须标注：两个分数的严格程度不同（巡检 rubric 体检 vs 归纳自评粗分）。
  const mapScore = map?.health.score ?? null
  const mapAt = map?.meta?.generated_at ? toMs(map.meta.generated_at) : null
  const patrolAt = latest?.finishedAt ? toMs(latest.finishedAt) : null
  const mapNewer = mapScore !== null && (patrolAt === null || (mapAt !== null && mapAt > patrolAt))
  const archScore = mapNewer ? mapScore : latest?.archScore ?? mapScore
  const scoreSource = latest === null || mapNewer ? 'map' : 'patrol'
  const moduleAvg = !mapNewer && latest && latest.moduleCount > 0 ? latest.moduleAvg : null
  const mapModuleAvg = useMemo(() => {
    if (!map || map.modules.length === 0) return null
    const avg = Math.round(map.modules.reduce((s, m) => s + (Number.isFinite(m.health.score) ? m.health.score : 0), 0) / map.modules.length)
    return Number.isFinite(avg) ? avg : null
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
      const r = await patrol(backendRepo)
      // 单会话纪律：归纳/分析会话在跑时后端返回 409——入队，当前会话结束后自动接续
      if (!r.ok) {
        const body = await r.json().catch(() => null)
        const msg = String(body?.error ?? '')
        if (r.status === 409 || body?.code === 'CONFLICT') {
          const res = await enqueue(backendRepo, 'patrol')
          if (res?.outcome === 'replaced')
            toast(t('pages.health.queuedReplaced', { label: res.replacedLabel ?? t('common.replacedFallback') }))
          else if (res?.outcome === 'queued') toast(t('pages.health.queued'))
          else if (res?.outcome === 'started') toast(t('pages.health.started'))
          return
        }
        toast(msg || t('pages.health.startFailed', { status: r.status }), 'error')
        return
      }
      toast(t('pages.health.startOk'))
    } catch {
      toast(t('pages.health.startOffline'), 'error')
    } finally {
      setStarting(false)
      // R3 C1：巡检是分钟级 LLM 任务，1.5s 刷新必然空转——改由 patrol.finished 事件驱动刷新
    }
  }

  // R3 C1：体检报告出来了 = 产品事件：自动刷新看板数据并解除按钮态
  useEffect(
    () =>
      onPatrolFinished((evt) => {
        if (evt.repo !== backendRepo) return
        load()
      }),
    [backendRepo, load],
  )

  if (!backendRepo) {
    return (
      <div className="flex h-full items-center justify-center text-[12px] text-slate-400 dark:text-slate-500">{t('common.pickProject')}</div>
    )
  }

  const hasAnyRun = (data?.runs.length ?? 0) > 0

  return (
    <div className="h-full overflow-y-auto p-5">
      <div className="mb-4 flex items-start justify-between">
        <div>
          <h2 className="flex items-center gap-1.5 text-[15px] font-bold text-slate-800 dark:text-slate-100">
            <HeartPulse size={15} className="text-red-400" /> {t('pages.health.title')}
          </h2>
          <p className="mt-0.5 text-[11px] text-slate-400 dark:text-slate-500">
            {t('pages.health.subtitle')}
          </p>
        </div>
        <button
          onClick={startPatrol}
          disabled={starting}
          className="flex items-center gap-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1.5 text-[11px] font-semibold text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70 disabled:opacity-40"
        >
          {starting ? <Loader2 size={11} className="animate-spin" /> : <Play size={11} />} {t('pages.health.startPatrol')}
        </button>
      </div>

      {/* KPI 行 */}
      <div className="mb-4 grid grid-cols-4 gap-3">
        <div className="lift relative rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-4 py-3">
          <p className="text-cap text-slate-400 dark:text-slate-500">{t('pages.health.kpiArch')}</p>
          <p className="tnum mt-1 text-[22px] font-bold leading-6" style={{ color: archScore !== null ? healthColor(archScore) : '#94a3b8' }}>
            {archScore ?? '—'} <span className="text-cap font-normal text-slate-300 dark:text-slate-600">/ 100</span>
          </p>
          {/* 来源标注：消除"地图说 74 / 看板说 89"的双地歧义（2026-10-06 实弹） */}
          {scoreSource === 'patrol' && latest?.finishedAt ? (
            <p className="mt-0.5 text-micro text-slate-400 dark:text-slate-500">{t('pages.health.sourcePatrol', { time: absTime(latest.finishedAt) })}</p>
          ) : scoreSource === 'map' && latest !== null ? (
            <p className="mt-0.5 text-micro text-amber-600 dark:text-amber-400">{t('pages.health.sourceMapNew')}</p>
          ) : (
            <p className="mt-0.5 text-micro text-slate-300 dark:text-slate-600">{t('pages.health.sourceMap')}</p>
          )}
          {/* 跨口径的 Delta（地图自评 vs 巡检分）是拿两把尺子相减，隐藏防误导 */}
          {archScore !== null && !(mapNewer && latest !== null) && <Delta now={archScore} prev={prev?.archScore ?? null} />}
          <div className="absolute right-3 top-1/2 -translate-y-1/2">{archScore !== null && <Ring score={archScore} />}</div>
        </div>
        <div className="lift relative rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-4 py-3">
          <p className="text-cap text-slate-400 dark:text-slate-500">{t('pages.health.kpiModuleAvg')}</p>
          <p className="tnum mt-1 text-[22px] font-bold leading-6" style={{ color: healthColor(moduleAvg ?? mapModuleAvg ?? 0) }}>
            {moduleAvg ?? mapModuleAvg ?? '—'} <span className="text-cap font-normal text-slate-300 dark:text-slate-600">/ 100</span>
          </p>
          {moduleAvg !== null && <Delta now={moduleAvg} prev={prev && prev.moduleCount > 0 ? prev.moduleAvg : null} />}
          {moduleAvg === null && <p className="mt-0.5 text-micro text-slate-300 dark:text-slate-600">{mapModuleAvg !== null ? t('pages.health.mapCurrent') : t('pages.health.noData')}</p>}
          <div className="absolute right-3 top-1/2 -translate-y-1/2">
            {(moduleAvg ?? mapModuleAvg) !== null && <Ring score={(moduleAvg ?? mapModuleAvg)!} />}
          </div>
        </div>
        <button
          onClick={onOpenDeps}
          disabled={!onOpenDeps}
          className="lift relative rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-4 py-3 text-left disabled:cursor-default hover:border-red-300 dark:hover:border-red-800"
          title={t('pages.health.reverseTip')}
        >
          <p className="text-cap text-slate-400 dark:text-slate-500">{t('pages.health.kpiReverse')}</p>
          <p className="tnum mt-1 text-[22px] font-bold leading-6 text-red-500">{reverseDeps ?? '—'}</p>
          <p className="mt-0.5 text-micro text-slate-300 dark:text-slate-600">{t('pages.health.reverseSub')}</p>
        </button>
        <div className="lift relative rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-4 py-3">
          <p className="text-cap text-slate-400 dark:text-slate-500">{t('pages.health.kpiCoverage')}</p>
          <p className="tnum mt-1 text-[22px] font-bold leading-6 text-emerald-500">
            {coverage !== null ? `${Math.round(coverage * 100)}` : '—'}
            <span className="text-[12px]"> %</span>
          </p>
          <p className="mt-0.5 text-micro text-slate-300 dark:text-slate-600">{t('pages.health.mapCurrent')}</p>
        </div>
      </div>

      {!hasAnyRun && !loading && (
        <div className="mb-4 rounded-xl border border-dashed border-slate-200 dark:border-slate-700 bg-white/60 px-4 py-8 text-center">
          <p className="text-[12px] font-semibold text-slate-500 dark:text-slate-400">{t('pages.health.emptyTitle')}</p>
          <p className="mt-1 text-[11px] text-slate-400 dark:text-slate-500">{t('pages.health.emptyHint')}</p>
        </div>
      )}

      <div className="grid grid-cols-3 gap-3">
        {/* 趋势图 */}
        <div className="col-span-2 rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
          <div className="mb-2 flex items-center justify-between">
            <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">{t('pages.health.trendTitle', { count: Math.min(10, trend.length) })}</span>
            <div className="flex items-center gap-3 text-micro text-slate-400 dark:text-slate-500">
              <span className="flex items-center gap-1"><i className="h-[3px] w-4 rounded bg-blue-500" /> {t('pages.health.legendArch')}</span>
              <span className="flex items-center gap-1"><i className="h-[3px] w-4 rounded bg-slate-300" /> {t('pages.health.legendModuleAvg')}</span>
            </div>
          </div>
          {trend.length >= 2 ? (
            <TrendChart runs={trend} hover={hover} setHover={setHover} />
          ) : (
            <div className="flex h-[180px] items-center justify-center text-[11px] text-slate-300 dark:text-slate-600">
              {t('pages.health.trendEmpty')}
            </div>
          )}
        </div>

        {/* 最差模块排行 */}
        <div className="rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
          <p className="mb-2 text-[13px] font-bold text-slate-700 dark:text-slate-200">{t('pages.health.rankTitle')}</p>
          <div className="space-y-1">
            {(data?.latestModules ?? []).slice(0, 10).map((m, i) => (
              <div key={m.moduleId} className="group flex items-center gap-2 rounded-lg px-2 py-1.5 hover:bg-slate-50 dark:hover:bg-slate-800/70">
                <span className="tnum w-4 text-micro text-slate-300 dark:text-slate-600">{i + 1}</span>
                <span className="min-w-0 flex-1 truncate text-[12px] text-slate-600 dark:text-slate-300">{m.name ?? m.moduleId}</span>
                {/* 把关台范式：健康问题就地派单（buildModuleTask 带 concern 描述与验收标准） */}
                {map && (
                  <button
                    onClick={() => onCreateTask(buildModuleTask(map, m.moduleId))}
                    className="shrink-0 rounded-md p-1 text-slate-200 transition-colors hover:bg-blue-50 dark:hover:bg-blue-950/40 hover:text-blue-600 group-hover:text-slate-300"
                    title={t('pages.health.fixTip', { name: m.name ?? m.moduleId })}
                  >
                    <Wrench size={11} />
                  </button>
                )}
                <div className="relative">
                  <Ring score={m.score} size={26} />
                  <span
                    className="tnum absolute inset-0 flex items-center justify-center text-micro font-bold"
                    style={{ color: healthColor(m.score) }}
                  >
                    {m.score}
                  </span>
                </div>
              </div>
            ))}
            {(data?.latestModules.length ?? 0) === 0 && (
              <p className="py-6 text-center text-[11px] text-slate-300 dark:text-slate-600">{t('pages.health.rankEmpty')}</p>
            )}
          </div>
        </div>
      </div>

      {/* 巡检记录 */}
      <div className="mt-3 overflow-hidden rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
        <div className="flex items-center gap-2 border-b border-slate-100 dark:border-slate-800 px-4 py-2.5">
          <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">{t('pages.health.recordsTitle')}</span>
          {/* 重审 P1：历史只增不减——保留最近 N 次的显式清理（running 永不删，后端保证） */}
          {hasAnyRun && (
            <span className="ml-auto flex items-center gap-1">
              {confirmPrune ? (
                <>
                  <span className="text-[10px] text-slate-400 dark:text-slate-500">{t('pages.health.pruneKeep')}</span>
                  <Select
                    className="w-20"
                    value={String(pruneKeep)}
                    onChange={(v) => setPruneKeep(Number(v))}
                    options={[5, 10, 20, 50].map((n) => ({ value: String(n), label: t('pages.health.pruneUnit', { count: n }) }))}
                  />
                  <button
                    onClick={() => void prune()}
                    disabled={pruning}
                    className="rounded-md bg-red-500 px-2 py-0.5 text-[10px] font-bold text-white hover:bg-red-600 disabled:opacity-40"
                  >
                    {pruning ? t('pages.health.pruneDoing') : t('pages.health.pruneConfirm')}
                  </button>
                  <button onClick={() => setConfirmPrune(false)} className="rounded p-0.5 text-slate-400 dark:text-slate-500 hover:text-slate-600" title={t('pages.health.pruneCancelTip')}>
                    <X size={11} />
                  </button>
                </>
              ) : (
                <button
                  onClick={() => setConfirmPrune(true)}
                  className="flex items-center gap-0.5 rounded-md border border-slate-200 dark:border-slate-700 px-2 py-0.5 text-[10px] font-semibold text-slate-400 dark:text-slate-500 hover:border-red-300 hover:text-red-500"
                  title={t('pages.health.pruneTip')}
                >
                  <Trash2 size={10} /> {t('pages.health.pruneButton')}
                </button>
              )}
            </span>
          )}
        </div>
        <table className="w-full text-left">
          <thead>
            <tr className="border-b border-slate-100 dark:border-slate-800 bg-slate-50/60 dark:bg-slate-900/60 text-[11px] font-semibold text-slate-400 dark:text-slate-500">
              <th className="px-4 py-2">{t('pages.health.colTime')}</th>
              <th className="px-3 py-2">{t('pages.health.colModel')}</th>
              <th className="px-3 py-2 text-right">{t('pages.health.colArch')}</th>
              <th className="px-3 py-2 text-right">{t('pages.health.colModuleAvg')}</th>
              <th className="px-3 py-2 text-right">{t('pages.health.colModuleCount')}</th>
              <th className="px-3 py-2">{t('pages.health.colStatus')}</th>
            </tr>
          </thead>
          <tbody>
            {(data?.runs ?? []).map((r) => (
              <tr key={r.id} className="border-b border-slate-50 last:border-0 hover:bg-slate-50/60 dark:bg-slate-900/60">
                <td className="tnum px-4 py-2.5 text-[12px] text-slate-600 dark:text-slate-300">{absTime(r.startedAt)}</td>
                <td className="px-3 py-2.5 text-[12px] text-slate-500 dark:text-slate-400">{r.model ?? '—'}</td>
                <td className="tnum px-3 py-2.5 text-right text-[12px] font-semibold" style={{ color: r.archScore !== null ? healthColor(r.archScore) : '#94a3b8' }}>
                  {r.archScore ?? '—'}
                </td>
                <td className="tnum px-3 py-2.5 text-right text-[12px] text-slate-600 dark:text-slate-300">{r.moduleCount > 0 ? r.moduleAvg : '—'}</td>
                <td className="tnum px-3 py-2.5 text-right text-[12px] text-slate-400 dark:text-slate-500">{r.moduleCount > 0 ? r.moduleCount : '—'}</td>
                <td className="px-3 py-2.5">
                  <span
                    className={`rounded-full px-1.5 py-px text-micro font-semibold ${
                      r.status === 'succeeded'
                        ? 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-600'
                        : r.status === 'failed'
                          ? 'bg-red-50 dark:bg-red-950/40 text-red-500'
                          : 'bg-amber-50 dark:bg-amber-950/40 text-amber-600'
                    }`}
                  >
                    {r.status === 'succeeded' ? t('common.status.succeeded') : r.status === 'failed' ? t('common.status.failed') : t('pages.health.statusRunning')}
                  </span>
                </td>
              </tr>
            ))}
            {!hasAnyRun && (
              <tr>
                <td colSpan={6} className="px-4 py-8 text-center text-[12px] text-slate-300 dark:text-slate-600">{loading ? t('common.loading') : t('pages.health.recordsEmpty')}</td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  )
}

/** 趋势折线图：纯 SVG，x=巡检序、y=0-100 分，悬停显示单点明细 */
function fmtDay(iso: string): string {
  const t = toMs(iso)
  if (t === null) return '—'
  const d = new Date(t)
  return `${d.getMonth() + 1}/${d.getDate()}`
}

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
  const y = (s: number) => PAD.t + (1 - Math.max(0, Math.min(100, Number.isFinite(s) ? s : 0)) / 100) * ih
  const line = (key: 'archScore' | 'moduleAvg') =>
    // 缺数据的点跳过（M/L 命令不带坐标），折线断开优于 NaN 坐标报错
    runs
      .map((r, i) => {
        const v = key === 'archScore' ? r.archScore : r.moduleCount > 0 ? r.moduleAvg : null
        if (v == null || !Number.isFinite(v)) return null
        return `${i === 0 ? 'M' : 'L'}${x(i).toFixed(1)},${y(v).toFixed(1)}`
      })
      .filter(Boolean)
      .join(' ')
  const h = hover !== null ? runs[hover] : null
  const { t } = useLang()

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
            {/* ui-test P2：archScore 可缺省/moduleAvg 可无数据——NaN 坐标是控制台 cx/cy NaN 报错的来源，缺数据不画点 */}
            {r.archScore != null && <circle cx={x(i)} cy={y(r.archScore)} r={hover === i ? 4 : 2.5} fill="#fff" stroke="#3b82f6" strokeWidth={2} />}
            {r.moduleCount > 0 && <circle cx={x(i)} cy={y(r.moduleAvg)} r={hover === i ? 3.5 : 2} fill="#fff" stroke="#cbd5e1" strokeWidth={2} />}
            {i % Math.ceil(runs.length / 8) === 0 && (
              <text x={x(i)} y={H - 6} textAnchor="middle" fontSize={8} fill="#cbd5e1">
                {/* ui-test P2：startedAt 可能是 epoch 毫秒串——slice(5,10) 会切出乱码；统一走 toMs 格式化 */}
                {fmtDay(r.startedAt)}
              </text>
            )}
          </g>
        ))}
      </svg>
      {h && (
        <div
          className="glass pointer-events-none absolute z-10 rounded-lg border border-slate-200 dark:border-slate-700 bg-white/95 dark:bg-slate-900/95 px-2.5 py-1.5 text-micro shadow-lg"
          style={{ left: `${(x(hover!) / W) * 100}%`, top: 0, transform: `translateX(${hover! > runs.length / 2 ? '-110%' : '10%'})` }}
        >
          <p className="tnum font-bold text-blue-600">{h.archScore ?? '—'} <span className="font-normal text-slate-400 dark:text-slate-500">{t('pages.health.tooltipArch')}</span></p>
          <p className="tnum text-slate-500 dark:text-slate-400">{h.moduleCount > 0 ? h.moduleAvg : '—'} <span className="font-normal text-slate-300 dark:text-slate-600">{t('pages.health.tooltipModuleAvg')}</span></p>
          <p className="tnum text-slate-300 dark:text-slate-600">{absTime(h.startedAt).slice(0, 10)}</p>
        </div>
      )}
    </div>
  )
}
