import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { AlertTriangle, Clock3, Coins, RefreshCw } from 'lucide-react'
import { onFreshnessEvent } from '@/runtime/growthBus'
import { useRepoActivityMap } from '@/runtime/useRepoActivity'
import { relTime } from '@/shared/logic/diffStat'
import { toast } from '@/runtime/toast'
import { useLang } from '@/runtime/i18n'
import { listRepos, patrolRuns } from '@/api/repos'
import { freshness, reinduce as reinduceApi } from '@/api/canvas'

// M4-3 漂移洞察整页（按 ui-mockups/漂移洞察原型.png 施工）：
// 跨仓库的保鲜仪表盘——KPI（需重归纳/平均落后提交/最近巡检）+ 仓库漂移排名
// （漂移进度条 = 已漂移天数占 7 天保鲜窗的比例，颜色按 fresh/drifting/stale 三级）
// + 检测到漂移横幅。数据面：GET /repos × (freshness + patrol-runs)，零新增后端。

interface RepoInfo {
  id: string
  name: string
  root: string
}

interface Freshness {
  status: 'fresh' | 'drifting' | 'stale' | 'unknown'
  mapGeneratedAt: string | null
  latestCommitAt: number | null
  commitsSinceMap: number | null
}

interface PatrolRun {
  id: string
  startedAt: string
  status: string
  archScore: number | null
}

interface RepoDrift {
  repo: RepoInfo
  freshness: Freshness | null
  latestPatrolAt: string | null
  reinducing: boolean
}

// 漂移状态元信息（label 走字典 pages.drift.status.*）
const STATUS_META: Record<string, { color: string; bg: string; labelKey: string }> = {
  fresh: { color: '#10b981', bg: 'bg-emerald-50 dark:bg-emerald-950/40', labelKey: 'pages.drift.status.fresh' },
  drifting: { color: '#f59e0b', bg: 'bg-amber-50 dark:bg-amber-950/40', labelKey: 'pages.drift.status.drifting' },
  stale: { color: '#ef4444', bg: 'bg-red-50 dark:bg-red-950/40', labelKey: 'pages.drift.status.stale' },
  unknown: { color: '#94a3b8', bg: 'bg-slate-50 dark:bg-slate-950/70', labelKey: 'pages.drift.status.unknown' },
}

/** 漂移进度：已漂移天数占 7 天保鲜窗的比例（fresh=0%，超过 7 天=100%） */
function driftPercent(f: Freshness, now: number): number | null {
  if (!f.mapGeneratedAt || f.status === 'unknown') return null
  const t = Date.parse(f.mapGeneratedAt)
  if (Number.isNaN(t)) return null
  const days = Math.max(0, now - t) / 86400000
  return Math.min(100, Math.round((days / 7) * 100))
}

export function DriftPage() {
  const { t } = useLang()
  const [rows, setRows] = useState<RepoDrift[] | null>(null)
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const tableRef = useRef<HTMLDivElement | null>(null) // 重审 P2：横幅"查看详情"滚动到排名表
  const [now] = useState(() => Date.now()) // 渲染期纯度：漂移百分比以进入页面时刻为锚
  // 跨页共享"归纳进行中"（19:29 实弹：本页发起的归纳只存于行内态，按钮随后恢复可点，
  // 用户在地图头部再点一次 → 重复入队）：活动会话/排队以 sessions/overview 为准
  const activityMap = useRepoActivityMap()

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const reposRes = await listRepos()
      if (!reposRes.ok) throw new Error(`HTTP ${reposRes.status}`)
      const reposData: { data?: RepoInfo[] } | null = await reposRes.json()
      const repos = reposData?.data ?? []
      const settled = await Promise.all(
        repos.map(async (repo) => {
          const [fRes, pRes] = await Promise.all([
            freshness(repo.id),
            patrolRuns(repo.id),
          ])
          const f: { data?: Freshness } | null = fRes.ok ? await fRes.json() : null
          const p: { data?: PatrolRun[] } | null = pRes.ok ? await pRes.json() : null
          const latestPatrolAt = p?.data?.length ? p.data[0].startedAt : null
          return { repo, freshness: f?.data ?? null, latestPatrolAt, reinducing: false }
        }),
      )
      setRows(settled)
      setLoadError(null)
    } catch {
      // 错误归因（审查 2#9）：后端离线 ≠ 没有仓库——两者文案必须区分
      setRows(null)
      setLoadError('backend-offline')
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    load()
  }, [load])
  // 审查 2#8：freshness 事件风暴会放大成 2N+1 请求风暴——5s 尾随防抖合并连续事件
  useEffect(() => {
    let timer: number | undefined
    const off = onFreshnessEvent(() => {
      window.clearTimeout(timer)
      timer = window.setTimeout(load, 5000)
    })
    return () => {
      window.clearTimeout(timer)
      off()
    }
  }, [load])

  const sorted = useMemo(() => {
    if (!rows) return []
    const rank = { stale: 0, drifting: 1, fresh: 2, unknown: 3 }
    return [...rows].sort((a, b) => {
      const ra = rank[a.freshness?.status ?? 'unknown']
      const rb = rank[b.freshness?.status ?? 'unknown']
      if (ra !== rb) return ra - rb
      return (b.freshness?.commitsSinceMap ?? 0) - (a.freshness?.commitsSinceMap ?? 0)
    })
  }, [rows])

  const kpis = useMemo(() => {
    const known = sorted.filter((r) => r.freshness?.status === 'drifting' || r.freshness?.status === 'stale')
    const withCommits = sorted.filter((r) => typeof r.freshness?.commitsSinceMap === 'number')
    const avgCommits = withCommits.length
      ? Math.round(withCommits.reduce((s, r) => s + (r.freshness?.commitsSinceMap ?? 0), 0) / withCommits.length)
      : null
    const latestPatrol = sorted
      .map((r) => r.latestPatrolAt)
      .filter((t): t is string => !!t)
      .sort()
      .pop()
    return { needReinduce: known.length, avgCommits, latestPatrol }
  }, [sorted])

  const reinduce = async (repoId: string) => {
    setRows((rs) => rs?.map((r) => (r.repo.id === repoId ? { ...r, reinducing: true } : r)) ?? null)
    try {
      const r = await reinduceApi(repoId)
      const d = await r.json().catch(() => null)
      if (!r.ok) toast(d?.error ?? t('pages.drift.reinduceFailed'), 'error')
    } catch {
      toast(t('pages.drift.reinduceOffline'), 'error')
    } finally {
      await load()
    }
  }

  return (
    <div className="h-full overflow-y-auto p-5">
      <div className="mb-4 flex items-start justify-between">
        <div>
          <h2 className="text-[15px] font-bold text-slate-800 dark:text-slate-100">{t('pages.drift.title')}</h2>
          <p className="mt-0.5 text-[11px] text-slate-400 dark:text-slate-500">
            {t('pages.drift.subtitle')}
          </p>
        </div>
        <button
          onClick={load}
          disabled={loading}
          className="flex items-center gap-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1.5 text-[11px] font-semibold text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70 disabled:opacity-40"
        >
          <RefreshCw size={11} className={loading ? 'animate-spin' : ''} /> {t('pages.drift.refresh')}
        </button>
      </div>

      {/* KPI 行 */}
      <div className="mb-4 grid grid-cols-3 gap-3">
        {[
          { icon: <Coins size={16} className="text-red-500" />, chip: 'bg-red-50 dark:bg-red-950/40', label: t('pages.drift.kpiNeedReinduce'), value: kpis.needReinduce, unit: t('pages.drift.unitRepos') },
          { icon: <Clock3 size={16} className="text-amber-500" />, chip: 'bg-amber-50 dark:bg-amber-950/40', label: t('pages.drift.kpiAvgBehind'), value: kpis.avgCommits, unit: t('pages.drift.unitCommits') },
          { icon: <RefreshCw size={16} className="text-emerald-500" />, chip: 'bg-emerald-50 dark:bg-emerald-950/40', label: t('pages.drift.kpiLatestPatrol'), value: kpis.latestPatrol ? relTime(kpis.latestPatrol, now, t) : null, unit: '' },
        ].map((k) => (
          <div key={k.label} className="lift flex items-center gap-3 rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-4 py-3">
            <div className={`flex h-8 w-8 items-center justify-center rounded-lg ${k.chip}`}>{k.icon}</div>
            <div>
              <p className="text-cap text-slate-400 dark:text-slate-500">{k.label}</p>
              <p className="tnum text-[17px] font-bold leading-5 text-slate-800 dark:text-slate-100">
                {k.value ?? '—'} {k.unit && <span className="text-cap font-normal text-slate-400 dark:text-slate-500">{k.unit}</span>}
              </p>
            </div>
          </div>
        ))}
      </div>

      {/* 仓库漂移排名 */}
      <div ref={tableRef} className="scroll-mt-4 overflow-hidden rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
        <div className="border-b border-slate-100 dark:border-slate-800 px-4 py-2.5">
          <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">{t('pages.drift.tableTitle')}</span>
        </div>
        <table className="w-full text-left">
          <thead>
            <tr className="border-b border-slate-100 dark:border-slate-800 bg-slate-50/60 dark:bg-slate-900/60 text-[11px] font-semibold text-slate-400 dark:text-slate-500">
              <th className="w-14 px-4 py-2">{t('pages.drift.colRank')}</th>
              <th className="px-3 py-2">{t('pages.drift.colRepo')}</th>
              <th className="px-3 py-2">{t('pages.drift.colProgress')}</th>
              <th className="w-28 px-3 py-2 text-right">{t('pages.drift.colAction')}</th>
            </tr>
          </thead>
          <tbody>
            {sorted.map((r, i) => {
              const meta = STATUS_META[r.freshness?.status ?? 'unknown']
              const pct = r.freshness ? driftPercent(r.freshness, now) : null
              return (
                <tr key={r.repo.id} className="border-b border-slate-50 last:border-0 hover:bg-slate-50/60 dark:bg-slate-900/60">
                  <td className="px-4 py-3">
                    <span
                      className={`tnum inline-flex h-5 w-5 items-center justify-center rounded-md text-cap font-bold ${
                        i < 3 ? 'bg-red-50 dark:bg-red-950/40 text-red-500' : 'bg-slate-100 dark:bg-slate-800 text-slate-400 dark:text-slate-500'
                      }`}
                    >
                      {i + 1}
                    </span>
                  </td>
                  <td className="px-3 py-3">
                    <p className="flex items-center gap-1.5 text-[13px] font-semibold text-slate-700 dark:text-slate-200">
                      {r.repo.name}
                      <span className="h-1.5 w-1.5 rounded-full" style={{ backgroundColor: meta.color }} />
                    </p>
                    <p className="tnum mt-0.5 text-cap text-slate-400 dark:text-slate-500">
                      {typeof r.freshness?.commitsSinceMap === 'number'
                        ? t('pages.drift.behind', { count: r.freshness.commitsSinceMap })
                        : t('pages.drift.behindUnknown')}
                      {r.freshness?.mapGeneratedAt ? ` · ${t('pages.drift.lastInduce', { time: relTime(r.freshness.mapGeneratedAt, now, t) })}` : ''}
                    </p>
                  </td>
                  <td className="px-3 py-3">
                    <div className="flex items-center gap-2">
                      <div className="h-1.5 w-40 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800">
                        <div
                          className="h-full rounded-full transition-all duration-500"
                          style={{ width: `${pct ?? 0}%`, backgroundColor: meta.color }}
                        />
                      </div>
                      <span className="tnum w-9 text-[11px] font-semibold" style={{ color: meta.color }}>
                        {pct === null ? '—' : `${pct}%`}
                      </span>
                    </div>
                  </td>
                  <td className="px-3 py-3 text-right">
                    {(() => {
                      const act = activityMap.get(r.repo.id)
                      const busy = r.reinducing || act?.inducing || act?.reinduceQueued
                      return (
                        <button
                          onClick={() => reinduce(r.repo.id)}
                          disabled={busy}
                          className="rounded-lg border border-blue-200 dark:border-blue-900/60 bg-blue-50 dark:bg-blue-950/40 px-2.5 py-1 text-cap font-semibold text-blue-600 hover:bg-blue-100 disabled:opacity-40"
                        >
                          {act?.reinduceQueued ? t('pages.drift.reinduceQueued') : r.reinducing || act?.inducing ? t('pages.drift.reinducing') : t('pages.drift.reinduce')}
                        </button>
                      )
                    })()}
                  </td>
                </tr>
              )
            })}
            {sorted.length === 0 && (
              <tr>
                <td colSpan={4} className="px-4 py-10 text-center text-[12px] text-slate-400 dark:text-slate-500">
                  {loadError
                    ? t('pages.drift.loadError')
                    : rows === null
                      ? t('common.loading')
                      : t('pages.drift.noRepos')}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {/* 漂移横幅 */}
      {kpis.needReinduce > 0 && (
        <div className="mt-4 flex items-center gap-3 rounded-xl border border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-4 py-3">
          <AlertTriangle size={16} className="shrink-0 text-amber-500" />
          <div className="min-w-0 flex-1">
            <p className="text-[12px] font-bold text-amber-700">{t('pages.drift.bannerTitle')}</p>
            <p className="mt-0.5 text-[11px] text-amber-600">
              {t('pages.drift.bannerBody', { count: kpis.needReinduce })}
            </p>
          </div>
          {/* 重审 P2：横幅"查看详情"此前是 span 假链接（点了没反应）——真按钮，滚动到上方排名表 */}
          <button
            onClick={() => tableRef.current?.scrollIntoView({ behavior: 'smooth', block: 'start' })}
            className="shrink-0 rounded-lg border border-amber-200 dark:border-amber-900/60 bg-white dark:bg-slate-900 px-2.5 py-1 text-[11px] font-semibold text-amber-600 hover:bg-amber-100 dark:hover:bg-amber-900/40"
          >
            {t('pages.drift.bannerDetail')}
          </button>
        </div>
      )}
    </div>
  )
}
