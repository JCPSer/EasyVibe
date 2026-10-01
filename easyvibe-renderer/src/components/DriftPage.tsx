import { useCallback, useEffect, useMemo, useState } from 'react'
import { AlertTriangle, Clock3, Coins, RefreshCw } from 'lucide-react'
import { onFreshnessEvent } from '@/lib/growthBus'
import { relTime } from '@/lib/diffStat'

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

const STATUS_META: Record<string, { color: string; bg: string; label: string }> = {
  fresh: { color: '#10b981', bg: 'bg-emerald-50', label: '新鲜' },
  drifting: { color: '#f59e0b', bg: 'bg-amber-50', label: '漂移中' },
  stale: { color: '#ef4444', bg: 'bg-red-50', label: '已过期' },
  unknown: { color: '#94a3b8', bg: 'bg-slate-50', label: '未知' },
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
  const [rows, setRows] = useState<RepoDrift[] | null>(null)
  const [loading, setLoading] = useState(false)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const reposRes = await fetch('/api/repos')
      const reposData: { data?: RepoInfo[] } | null = reposRes.ok ? await reposRes.json() : null
      const repos = reposData?.data ?? []
      const settled = await Promise.all(
        repos.map(async (repo) => {
          const [fRes, pRes] = await Promise.all([
            fetch(`/api/repos/${encodeURIComponent(repo.id)}/freshness`),
            fetch(`/api/repos/${encodeURIComponent(repo.id)}/patrol-runs`),
          ])
          const f: { data?: Freshness } | null = fRes.ok ? await fRes.json() : null
          const p: { data?: PatrolRun[] } | null = pRes.ok ? await pRes.json() : null
          const latestPatrolAt = p?.data?.length ? p.data[0].startedAt : null
          return { repo, freshness: f?.data ?? null, latestPatrolAt, reinducing: false }
        }),
      )
      setRows(settled)
    } catch {
      setRows([])
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    load()
  }, [load])
  useEffect(() => onFreshnessEvent(() => load()), [load])

  const now = Date.now()
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
      await fetch(`/api/repos/${encodeURIComponent(repoId)}/reinduce`, { method: 'POST' })
    } finally {
      await load()
    }
  }

  return (
    <div className="h-full overflow-y-auto p-5">
      <div className="mb-4 flex items-start justify-between">
        <div>
          <h2 className="text-[15px] font-bold text-slate-800">漂移洞察</h2>
          <p className="mt-0.5 text-[11px] text-slate-400">
            持续跟踪代码仓库与归纳状态之间的漂移，帮助团队及时发现并处理落后风险。
          </p>
        </div>
        <button
          onClick={load}
          disabled={loading}
          className="flex items-center gap-1 rounded-lg border border-slate-200 bg-white px-2.5 py-1.5 text-[11px] font-semibold text-slate-500 hover:bg-slate-50 disabled:opacity-40"
        >
          <RefreshCw size={11} className={loading ? 'animate-spin' : ''} /> 刷新
        </button>
      </div>

      {/* KPI 行 */}
      <div className="mb-4 grid grid-cols-3 gap-3">
        {[
          { icon: <Coins size={16} className="text-red-500" />, chip: 'bg-red-50', label: '需重归纳', value: kpis.needReinduce, unit: '仓库' },
          { icon: <Clock3 size={16} className="text-amber-500" />, chip: 'bg-amber-50', label: '平均落后', value: kpis.avgCommits, unit: '提交' },
          { icon: <RefreshCw size={16} className="text-emerald-500" />, chip: 'bg-emerald-50', label: '最近巡检', value: kpis.latestPatrol ? relTime(kpis.latestPatrol, now) : null, unit: '' },
        ].map((k) => (
          <div key={k.label} className="lift flex items-center gap-3 rounded-xl border border-slate-200 bg-white px-4 py-3">
            <div className={`flex h-8 w-8 items-center justify-center rounded-lg ${k.chip}`}>{k.icon}</div>
            <div>
              <p className="text-[10.5px] text-slate-400">{k.label}</p>
              <p className="tnum text-[17px] font-bold leading-5 text-slate-800">
                {k.value ?? '—'} {k.unit && <span className="text-[10.5px] font-normal text-slate-400">{k.unit}</span>}
              </p>
            </div>
          </div>
        ))}
      </div>

      {/* 仓库漂移排名 */}
      <div className="overflow-hidden rounded-xl border border-slate-200 bg-white">
        <div className="border-b border-slate-100 px-4 py-2.5">
          <span className="text-[12.5px] font-bold text-slate-700">仓库漂移排名</span>
        </div>
        <table className="w-full text-left">
          <thead>
            <tr className="border-b border-slate-100 bg-slate-50/60 text-[11px] font-semibold text-slate-400">
              <th className="w-14 px-4 py-2">排名</th>
              <th className="px-3 py-2">仓库</th>
              <th className="px-3 py-2">漂移进度</th>
              <th className="w-28 px-3 py-2 text-right">操作</th>
            </tr>
          </thead>
          <tbody>
            {sorted.map((r, i) => {
              const meta = STATUS_META[r.freshness?.status ?? 'unknown']
              const pct = r.freshness ? driftPercent(r.freshness, now) : null
              return (
                <tr key={r.repo.id} className="border-b border-slate-50 last:border-0 hover:bg-slate-50/60">
                  <td className="px-4 py-3">
                    <span
                      className={`tnum inline-flex h-5 w-5 items-center justify-center rounded-md text-[10.5px] font-bold ${
                        i < 3 ? 'bg-red-50 text-red-500' : 'bg-slate-100 text-slate-400'
                      }`}
                    >
                      {i + 1}
                    </span>
                  </td>
                  <td className="px-3 py-3">
                    <p className="flex items-center gap-1.5 text-[12.5px] font-semibold text-slate-700">
                      {r.repo.name}
                      <span className="h-1.5 w-1.5 rounded-full" style={{ backgroundColor: meta.color }} />
                    </p>
                    <p className="tnum mt-0.5 text-[10.5px] text-slate-400">
                      {typeof r.freshness?.commitsSinceMap === 'number'
                        ? `落后 ${r.freshness.commitsSinceMap} 提交`
                        : '落后提交数未知'}
                      {r.freshness?.mapGeneratedAt ? ` · 上次归纳 ${relTime(r.freshness.mapGeneratedAt, now)}` : ''}
                    </p>
                  </td>
                  <td className="px-3 py-3">
                    <div className="flex items-center gap-2">
                      <div className="h-1.5 w-40 overflow-hidden rounded-full bg-slate-100">
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
                    <button
                      onClick={() => reinduce(r.repo.id)}
                      disabled={r.reinducing}
                      className="rounded-lg border border-blue-200 bg-blue-50 px-2.5 py-1 text-[10.5px] font-semibold text-blue-600 hover:bg-blue-100 disabled:opacity-40"
                    >
                      {r.reinducing ? '归纳中…' : '重新归纳'}
                    </button>
                  </td>
                </tr>
              )
            })}
            {sorted.length === 0 && (
              <tr>
                <td colSpan={4} className="px-4 py-10 text-center text-[11.5px] text-slate-400">
                  {rows === null ? '加载中…' : '暂无注册仓库，请先在设置中添加仓库。'}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {/* 漂移横幅 */}
      {kpis.needReinduce > 0 && (
        <div className="mt-4 flex items-center gap-3 rounded-xl border border-amber-200 bg-amber-50 px-4 py-3">
          <AlertTriangle size={16} className="shrink-0 text-amber-500" />
          <div className="min-w-0 flex-1">
            <p className="text-[12px] font-bold text-amber-700">检测到仓库漂移</p>
            <p className="mt-0.5 text-[11px] text-amber-600">
              {kpis.needReinduce} 个仓库与当前归纳状态存在显著差异，可能影响智能体的回答准确性与时效性。
            </p>
          </div>
          <span className="shrink-0 text-[11px] font-semibold text-amber-500">查看详情 ↑</span>
        </div>
      )}
    </div>
  )
}
