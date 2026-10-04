import { useEffect, useMemo, useState } from 'react'
import { Gauge, Loader2, TriangleAlert, Lightbulb, ArrowRight } from 'lucide-react'
import type { CodeMap } from '@/types/map'
import { healthColor } from '@/lib/layout'

// 「用量」页（docs/llm-usage-page-design-v1.md U2/L1/L2 施工）：
// KPI + 按日/按类型/按模型/按模块（治理账单 L1）+ 洞察卡（L2 腐化的代价）+ 逐会话明细。
// 数据源：GET /repos/{id}/usage?days=N（一次拉全五个聚合）。
// 诚实降级：cost/tokens 为 NULL（被 kill/非 claude/旧会话）显示「—」而非 0；区间无数据 KPI 显示「—」。

interface UsageTotals {
  sessions: number
  succeeded?: number | null
  failed?: number | null
  failedCost?: number | null
  cost?: number | null
  reported?: number | null
  inputTokens?: number | null
  outputTokens?: number | null
  cacheRead?: number | null
}
interface UsageDaily { day: string; inputTokens?: number | null; outputTokens?: number | null }
interface UsageGroup { name: string; sessions: number; value?: number | null }
interface UsageModule { name: string; sessions: number; cost?: number | null; failed?: number | null }
interface AgentSession {
  id: string; kind: string; label?: string | null; model?: string | null; moduleId?: string | null
  startedAt: string; terminalAt?: string | null; status: string; exitCode?: number | null
  inputTokens?: number | null; outputTokens?: number | null; cacheReadTokens?: number | null
  costUsd?: number | null; durationMs?: number | null; turns?: number | null; usageSource?: string | null
}
interface UsageData {
  since: string
  totals: UsageTotals
  daily: UsageDaily[]
  byKind: UsageGroup[]
  byModel: UsageGroup[]
  byModule: UsageModule[]
  sessions: AgentSession[]
}

const KIND_ZH: Record<string, string> = {
  induce: '归纳', patrol: '巡检', submap: '子图分析', task: '任务执行',
  'subagent-review': '任务执行 · 初审', 'subagent-audit': '任务执行 · 审查', unknown: '其他',
}
const RANGE = [{ d: 1, label: '今天' }, { d: 7, label: '7 天' }, { d: 30, label: '30 天' }, { d: 0, label: '全部' }]

const fmtCost = (v?: number | null): string => (v == null ? '—' : v < 0.01 ? `$${v.toFixed(4)}` : `$${v.toFixed(2)}`)
const fmtTokens = (v?: number | null): string => {
  if (v == null) return '—'
  if (v >= 1_000_000) return `${(v / 1_000_000).toFixed(1)}M`
  if (v >= 1_000) return `${Math.round(v / 1_000)}k`
  return String(v)
}
const fmtDate = (iso: string): string => {
  const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})/.exec(iso)
  return m ? `${m[2]}-${m[3]} ${m[4]}:${m[5]}` : iso.slice(0, 16)
}
const fmtDur = (ms?: number | null): string => {
  if (ms == null) return '—'
  const s = Math.round(ms / 1000)
  return s >= 3600 ? `${Math.floor(s / 3600)}:${String(Math.floor((s % 3600) / 60)).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}` : `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

function Kpi({ label, value, sub, tone }: { label: string; value: string; sub: string; tone: 'amber' | 'blue' | 'green' | 'indigo' }) {
  const bar = { amber: 'bg-amber-500', blue: 'bg-blue-500', green: 'bg-emerald-500', indigo: 'bg-indigo-500' }[tone]
  const color = { amber: 'text-amber-500', blue: 'text-blue-600', green: 'text-emerald-500', indigo: 'text-indigo-500' }[tone]
  return (
    <div className="lift relative rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-4 py-3">
      <span className={`absolute left-0 top-[18px] h-9 w-1 rounded-r ${bar}`} />
      <p className="pl-2 text-cap text-slate-400 dark:text-slate-500">{label}</p>
      <p className={`tnum mt-1 pl-2 text-[22px] font-bold leading-6 ${value === '—' ? 'text-slate-300 dark:text-slate-600' : color}`}>{value}</p>
      <p className="mt-0.5 truncate pl-2 text-micro text-slate-400 dark:text-slate-500">{sub}</p>
    </div>
  )
}

/** 纯 SVG 堆积柱（按日 tokens 趋势）：输入浅蓝在下、输出深蓝在上——TrendChart 同款零依赖范式 */
function DailyChart({ daily }: { daily: UsageDaily[] }) {
  const W = 620, H = 170, PAD = { l: 34, r: 10, t: 10, b: 20 }
  const iw = W - PAD.l - PAD.r, ih = H - PAD.t - PAD.b
  const max = Math.max(1, ...daily.map((d) => (d.inputTokens ?? 0) + (d.outputTokens ?? 0)))
  const bw = daily.length > 0 ? Math.min(26, (iw / daily.length) * 0.62) : 0
  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="w-full">
      {[0, 0.5, 1].map((g) => (
        <g key={g}>
          <line x1={PAD.l} x2={W - PAD.r} y1={PAD.t + ih * g} y2={PAD.t + ih * g} stroke="#f1f5f9" strokeWidth={1} />
          <text x={PAD.l - 5} y={PAD.t + ih * g + 3} textAnchor="end" fontSize={8} fill="#cbd5e1" className="tnum">
            {fmtTokens(Math.round(max * (1 - g)))}
          </text>
        </g>
      ))}
      {daily.map((d, i) => {
        const x = PAD.l + (daily.length === 1 ? iw / 2 : (i / (daily.length - 1)) * iw) - bw / 2
        const inH = ((d.inputTokens ?? 0) / max) * ih
        const outH = ((d.outputTokens ?? 0) / max) * ih
        return (
          <g key={d.day}>
            <rect x={x} y={PAD.t + ih - inH} width={bw} height={Math.max(1, inH)} rx={1.5} fill="#93c5fd">
              <title>{`${d.day} 输入 ${fmtTokens(d.inputTokens)} · 输出 ${fmtTokens(d.outputTokens)}`}</title>
            </rect>
            <rect x={x} y={PAD.t + ih - inH - outH} width={bw} height={Math.max(1, outH)} rx={1.5} fill={outH > 0 ? '#2563eb' : 'none'}>
              <title>{`${d.day} 输入 ${fmtTokens(d.inputTokens)} · 输出 ${fmtTokens(d.outputTokens)}`}</title>
            </rect>
            {(i % Math.ceil(daily.length / 8) === 0 || i === daily.length - 1) && (
              <text x={x + bw / 2} y={H - 5} textAnchor="middle" fontSize={8} fill="#cbd5e1">{d.day.slice(5)}</text>
            )}
          </g>
        )
      })}
      {daily.length === 0 && <text x={W / 2} y={H / 2} textAnchor="middle" fontSize={10} fill="#cbd5e1">区间内暂无会话</text>}
    </svg>
  )
}

/** 横向分布条（按类型/按模型/按模块共用） */
function GroupBars({ rows, total, unit, valueFmt, renderMeta }: {
  rows: UsageGroup[] | UsageModule[]
  total: number
  unit: 'cost' | 'tokens' | 'sessions'
  valueFmt: (v?: number | null) => string
  renderMeta?: (r: UsageGroup | UsageModule) => React.ReactNode
}) {
  return (
    <div className="space-y-2">
      {rows.map((r) => {
        const v = unit === 'cost' ? (r as UsageModule).cost ?? null : (r as UsageGroup).value ?? null
        const pct = total > 0 && v != null ? v / total : 0
        return (
          <div key={r.name} className="flex items-center gap-2">
            <span className="w-36 shrink-0 truncate text-[12px] text-slate-600 dark:text-slate-300">{KIND_ZH[r.name] ?? r.name}</span>
            <div className="h-2.5 flex-1 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800">
              <div className="h-full rounded-full bg-blue-500" style={{ width: `${Math.max(2, pct * 100)}%` }} />
            </div>
            <span className="tnum w-20 shrink-0 text-right text-micro text-slate-400 dark:text-slate-500">{Math.round(pct * 100)}%</span>
            <span className="tnum w-20 shrink-0 text-right text-[11px] text-slate-600 dark:text-slate-300">{valueFmt(v)}</span>
            {renderMeta?.(r)}
          </div>
        )
      })}
      {rows.length === 0 && <p className="py-4 text-center text-[11px] text-slate-300 dark:text-slate-600">暂无数据</p>}
    </div>
  )
}

export function UsagePage({ backendRepo, map, onOpenModule, onOpenSession }: {
  backendRepo: string | null
  map: CodeMap | null
  onOpenModule: (moduleId: string) => void
  onOpenSession: (sessionId: string) => void
}) {
  const [days, setDays] = useState(30)
  const [data, setData] = useState<UsageData | null>(null)
  // 首次加载态由 data===null 派生（避免 effect 内同步 setState 的级联渲染告警）；
  // 切换时间范围时保留旧数据展示，新数据到位即换
  const loading = data === null

  useEffect(() => {
    if (!backendRepo) return
    let stale = false
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/usage?days=${days}`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: UsageData } | null) => {
        if (!stale) setData(d?.data ?? null)
      })
      .catch(() => {
        if (!stale) setData(null)
      })
    return () => {
      stale = true
    }
  }, [backendRepo, days])

  const t = data?.totals
  const modById = useMemo(() => new Map((map?.modules ?? []).map((m) => [m.id, m])), [map])

  // L2 洞察自动生成为人话
  const insights = useMemo(() => {
    if (!data || !t || t.sessions === 0) return []
    const out: { title: string; body: string; tone: 'amber' | 'red' | 'green' }[] = []
    const kindCostTotal = data.byKind.reduce((s, k) => s + (k.value ?? 0), 0)
    const top = [...data.byKind].sort((a, b) => (b.value ?? 0) - (a.value ?? 0))[0]
    if (top && (top.value ?? 0) > 0) {
      const failed = t.failed ?? 0
      const failedCost = t.failedCost ?? 0
      out.push({
        title: `${KIND_ZH[top.name] ?? top.name}是本期最贵的动作`,
        body: `花掉 ${fmtCost(top.value)}（占 ${kindCostTotal > 0 ? Math.round(((top.value ?? 0) / kindCostTotal) * 100) : 0}%）${failed > 0 ? `；其中 ${failed} 次失败白跑 ${fmtCost(failedCost)}` : ''}。`,
        tone: 'amber',
      })
    }
    // 腐化的代价：健康分 <60 的模块吃掉的花费占比
    const modTotal = data.byModule.reduce((s, m) => s + (m.cost ?? 0), 0)
    const decayCost = data.byModule.reduce((s, m) => {
      const mod = modById.get(m.name)
      return s + (mod && mod.health.score < 60 ? (m.cost ?? 0) : 0)
    }, 0)
    const decayMods = data.byModule.filter((m) => (modById.get(m.name)?.health.score ?? 100) < 60)
    if (modTotal > 0 && decayCost > 0 && decayMods.length > 0) {
      out.push({
        title: '腐化的代价',
        body: `${Math.round((decayCost / modTotal) * 100)}% 的模块治理花费集中在 ${decayMods.length} 个健康分 <60 的模块上——钱在替腐化买单。`,
        tone: 'red',
      })
    }
    if ((t.cacheRead ?? 0) > 0 && (t.inputTokens ?? 0) > 0) {
      const hit = Math.round((t.cacheRead! / (t.cacheRead! + t.inputTokens!)) * 100)
      out.push({ title: `缓存命中率 ${hit}%`, body: '重复上下文命中缓存，这部分输入按缓存价计。', tone: 'green' })
    }
    return out.slice(0, 3)
  }, [data, t, modById])

  if (!backendRepo) {
    return <div className="flex h-full items-center justify-center text-[12px] text-slate-400 dark:text-slate-500">先在左侧选择一个项目。</div>
  }

  const cacheHit = t && (t.cacheRead ?? 0) > 0 && (t.inputTokens ?? 0) > 0
    ? Math.round((t.cacheRead! / (t.cacheRead! + t.inputTokens!)) * 100)
    : null
  const kindCostTotal = data?.byKind.reduce((s, k) => s + (k.value ?? 0), 0) ?? 0
  const modelTokenTotal = data?.byModel.reduce((s, m) => s + (m.value ?? 0), 0) ?? 0
  const modCostTotal = data?.byModule.reduce((s, m) => s + (m.cost ?? 0), 0) ?? 0

  return (
    <div className="h-full overflow-y-auto p-5">
      <div className="mb-4 flex items-start justify-between">
        <div>
          <h2 className="flex items-center gap-1.5 text-[15px] font-bold text-slate-800 dark:text-slate-100">
            <Gauge size={15} className="text-blue-500" /> 用量
          </h2>
          <p className="mt-0.5 text-[11px] text-slate-400 dark:text-slate-500">
            这个项目的 agent 花掉了多少模型额度：按时间、按类型、按模型的统计与逐次会话明细。
          </p>
        </div>
        <div className="flex items-center gap-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-0.5">
          {RANGE.map(({ d, label }) => (
            <button
              key={d}
              onClick={() => setDays(d)}
              className={`rounded-md px-2.5 py-1.5 text-[12px] font-semibold transition-colors ${
                days === d ? 'bg-blue-50 dark:bg-blue-950/40 text-blue-600' : 'text-slate-400 dark:text-slate-500 hover:text-slate-600'
              }`}
            >
              {label}
            </button>
          ))}
        </div>
      </div>

      {loading && (
        <div className="flex items-center justify-center gap-2 py-16 text-[12px] text-slate-400 dark:text-slate-500">
          <Loader2 size={13} className="animate-spin" /> 加载中…
        </div>
      )}

      {data && t && t.sessions === 0 && (
        <div className="rounded-xl border border-dashed border-slate-200 dark:border-slate-700 bg-white/60 px-4 py-10 text-center">
          <p className="text-[12px] font-semibold text-slate-500 dark:text-slate-400">这个时间范围内还没有会话</p>
          <p className="mt-1 text-[11px] text-slate-400 dark:text-slate-500">归纳 / 巡检 / 任务执行跑起来后，这里会自动统计 tokens 与花费。</p>
        </div>
      )}

      {data && t && t.sessions > 0 && (
        <>
          {/* KPI 行 */}
          <div className="mb-4 grid grid-cols-4 gap-3">
            <Kpi
              label="本期花费"
              value={fmtCost(t.cost)}
              sub={t.cost == null ? '跑几次会话后自动填充' : `自报 ${t.reported ?? 0} 次 · 口径见页脚`}
              tone="amber"
            />
            <Kpi
              label="总消耗 tokens"
              value={t.inputTokens == null && t.outputTokens == null ? '—' : fmtTokens((t.inputTokens ?? 0) + (t.outputTokens ?? 0))}
              sub={`输入 ${fmtTokens(t.inputTokens)} · 输出 ${fmtTokens(t.outputTokens)}`}
              tone="blue"
            />
            <Kpi
              label="会话次数"
              value={String(t.sessions)}
              sub={`成功 ${t.succeeded ?? 0} · 失败 ${t.failed ?? 0}${(t.failedCost ?? 0) > 0 ? `（白跑 ${fmtCost(t.failedCost)}）` : ''}`}
              tone="green"
            />
            <Kpi
              label="缓存命中率"
              value={cacheHit === null ? '—' : `${cacheHit}%`}
              sub={cacheHit === null ? '暂无缓存数据' : `缓存读 ${fmtTokens(t.cacheRead)}`}
              tone="indigo"
            />
          </div>

          {/* 按日趋势 + 按类型分布 */}
          <div className="mb-3 grid grid-cols-3 gap-3">
            <div className="col-span-2 rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
              <div className="mb-2 flex items-center justify-between">
                <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">按日消耗趋势</span>
                <span className="flex items-center gap-3 text-micro text-slate-400 dark:text-slate-500">
                  <span className="flex items-center gap-1"><i className="h-[3px] w-4 rounded bg-blue-300" /> 输入</span>
                  <span className="flex items-center gap-1"><i className="h-[3px] w-4 rounded bg-blue-600" /> 输出</span>
                </span>
              </div>
              <DailyChart daily={data.daily} />
            </div>
            <div className="rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
              <p className="mb-1 text-[13px] font-bold text-slate-700 dark:text-slate-200">按类型分布</p>
              <p className="mb-3 text-micro text-slate-400 dark:text-slate-500">本期花费占比 · 子 agent 折入父类型</p>
              <GroupBars rows={data.byKind} total={kindCostTotal} unit="cost" valueFmt={fmtCost} />
            </div>
          </div>

          {/* 按模型 + 洞察卡 */}
          <div className="mb-3 grid grid-cols-5 gap-3">
            <div className="col-span-3 rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
              <p className="mb-1 text-[13px] font-bold text-slate-700 dark:text-slate-200">按模型分布</p>
              <p className="mb-3 text-micro text-slate-400 dark:text-slate-500">tokens 占比 · 来自会话上报的真实模型名</p>
              <GroupBars rows={data.byModel} total={modelTokenTotal} unit="tokens" valueFmt={fmtTokens} />
            </div>
            <div className="col-span-2 space-y-2 rounded-xl border border-amber-200 bg-amber-50 p-4 dark:border-amber-900/60 dark:bg-amber-950/30">
              <p className="flex items-center gap-1 text-[13px] font-bold text-amber-600 dark:text-amber-400">
                <Lightbulb size={13} /> 本期洞察
              </p>
              {insights.map((ins, i) => (
                <div key={i} className={i > 0 ? 'border-t border-amber-200 pt-2 dark:border-amber-900/60' : ''}>
                  <p className={`text-[12px] font-bold ${ins.tone === 'red' ? 'text-red-500' : ins.tone === 'green' ? 'text-emerald-600' : 'text-amber-600 dark:text-amber-400'}`}>
                    {ins.tone === 'red' && <TriangleAlert size={11} className="mr-1 inline" />}
                    {ins.title}
                  </p>
                  <p className="mt-0.5 text-cap leading-4 text-slate-600 dark:text-slate-300">{ins.body}</p>
                </div>
              ))}
            </div>
          </div>

          {/* L1 按模块治理账单 */}
          <div className="mb-3 rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
            <p className="mb-1 text-[13px] font-bold text-slate-700 dark:text-slate-200">按模块 TopN · 治理账单</p>
            <p className="mb-3 text-micro text-slate-400 dark:text-slate-500">花费经「任务→影响模块」与子图会话归属归因 · 点击行跳模块详情</p>
            <div className="space-y-2">
              {data.byModule.map((m) => {
                const mod = modById.get(m.name)
                const score = mod?.health.score
                const unhealthy = score !== undefined && score < 60
                const pct = modCostTotal > 0 && m.cost != null ? m.cost / modCostTotal : 0
                return (
                  <button key={m.name} onClick={() => mod && onOpenModule(m.name)} className="flex w-full items-center gap-2 text-left">
                    <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: score !== undefined ? healthColor(score) : '#cbd5e1' }} />
                    <span className="w-36 shrink-0 truncate text-[12px] text-slate-700 dark:text-slate-200">{mod?.name ?? m.name}</span>
                    <span className="tnum w-10 shrink-0 text-micro" style={{ color: score !== undefined ? healthColor(score) : '#94a3b8' }}>
                      {score ?? '—'}
                    </span>
                    <div className="h-2.5 flex-1 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800">
                      <div className={`h-full rounded-full ${unhealthy ? 'bg-red-400' : 'bg-blue-500'}`} style={{ width: `${Math.max(2, pct * 100)}%` }} />
                    </div>
                    <span className="tnum w-16 shrink-0 text-right text-[11px] text-slate-600 dark:text-slate-300">{fmtCost(m.cost)}</span>
                    <span className="w-32 shrink-0 truncate text-right text-micro text-slate-400 dark:text-slate-500">
                      {m.sessions} 次{m.failed ? <span className="text-red-400"> · {m.failed} 次失败</span> : ''}
                    </span>
                    <ArrowRight size={11} className="shrink-0 text-slate-300 dark:text-slate-600" />
                  </button>
                )
              })}
              {data.byModule.length === 0 && (
                <p className="py-4 text-center text-[11px] text-slate-300 dark:text-slate-600">
                  暂无模块归因数据（任务与子图分析跑起来后自动归因）
                </p>
              )}
            </div>
          </div>

          {/* 明细表 */}
          <div className="overflow-hidden rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
            <div className="flex items-center justify-between border-b border-slate-100 dark:border-slate-800 px-4 py-2.5">
              <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">会话明细</span>
              <span className="text-micro text-slate-400 dark:text-slate-500">共 {t.sessions} 次 · 点击行看会话流水 · 按开始时间倒序</span>
            </div>
            <div className="max-h-80 overflow-y-auto">
              <table className="w-full text-left">
                <thead className="sticky top-0 bg-slate-50/90 dark:bg-slate-900/90 text-[10px] font-semibold text-slate-400 dark:text-slate-500">
                  <tr>
                    <th className="px-4 py-2">时间</th>
                    <th className="px-3 py-2">类型</th>
                    <th className="px-3 py-2">模块</th>
                    <th className="px-3 py-2">模型</th>
                    <th className="px-3 py-2 text-right">tokens 进/出</th>
                    <th className="px-3 py-2 text-right">缓存命中</th>
                    <th className="px-3 py-2 text-right">耗时</th>
                    <th className="px-3 py-2 text-right">轮数</th>
                    <th className="px-3 py-2 text-right">花费</th>
                    <th className="px-3 py-2">状态</th>
                  </tr>
                </thead>
                <tbody>
                  {data.sessions.map((s) => {
                    const cachePct = s.cacheReadTokens != null && s.inputTokens != null && s.cacheReadTokens + s.inputTokens > 0
                      ? `${Math.round((s.cacheReadTokens / (s.cacheReadTokens + s.inputTokens)) * 100)}%`
                      : '—'
                    return (
                      <tr key={s.id} onClick={() => onOpenSession(s.id)} className="cursor-pointer border-b border-slate-50 last:border-0 hover:bg-slate-50/60 dark:hover:bg-slate-800/40">
                        <td className="tnum px-4 py-2 text-[12px] text-slate-600 dark:text-slate-300">{fmtDate(s.startedAt)}</td>
                        <td className="px-3 py-2 text-[12px] text-slate-600 dark:text-slate-300">{KIND_ZH[s.kind] ?? s.label ?? s.kind}</td>
                        <td className="max-w-[120px] truncate px-3 py-2 text-[12px] text-slate-500 dark:text-slate-400">
                          {s.moduleId ? (modById.get(s.moduleId)?.name ?? s.moduleId) : '—'}
                        </td>
                        <td className="px-3 py-2 font-mono text-[11px] text-slate-500 dark:text-slate-400">{s.model ?? '—'}</td>
                        <td className="tnum px-3 py-2 text-right text-[12px] text-slate-600 dark:text-slate-300">
                          {fmtTokens(s.inputTokens)} / {fmtTokens(s.outputTokens)}
                        </td>
                        <td className="tnum px-3 py-2 text-right text-[12px] text-slate-500 dark:text-slate-400">{cachePct}</td>
                        <td className="tnum px-3 py-2 text-right text-[12px] text-slate-600 dark:text-slate-300">{fmtDur(s.durationMs)}</td>
                        <td className="tnum px-3 py-2 text-right text-[12px] text-slate-500 dark:text-slate-400">{s.turns ?? '—'}</td>
                        <td className="tnum px-3 py-2 text-right text-[12px] text-slate-600 dark:text-slate-300">{fmtCost(s.costUsd)}</td>
                        <td className="px-3 py-2">
                          <span className={`rounded-full px-1.5 py-px text-micro font-semibold ${
                            s.status === 'succeeded' ? 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-600'
                            : s.status === 'failed' ? 'bg-red-50 dark:bg-red-950/40 text-red-500'
                            : 'bg-slate-100 dark:bg-slate-800 text-slate-500 dark:text-slate-400'
                          }`}>
                            {s.status === 'succeeded' ? '成功' : s.status === 'failed' ? '失败' : s.status}
                          </span>
                        </td>
                      </tr>
                    )
                  })}
                </tbody>
              </table>
            </div>
          </div>

          {/* 页脚口径 */}
          <p className="mt-3 rounded-lg bg-slate-50 dark:bg-slate-900/60 px-3 py-2 text-center text-micro text-slate-400 dark:text-slate-500">
            口径：花费优先取 agent 自报成本，缺失时按模型价格表估算；订阅制计费与实际账单可能有差异，仅供参考。被终止的会话无用量回报，显示「—」。
          </p>
        </>
      )}
    </div>
  )
}
