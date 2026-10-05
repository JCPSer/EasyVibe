import { useMemo, useState } from 'react'
import {
  Waypoints,
  TriangleAlert,
  Loader2,
  Wrench,
  MessagesSquare,
  Map as MapIcon,
  ChevronDown,
  ChevronRight,
  Repeat,
  Flame,
} from 'lucide-react'
import type { CodeMap, MapEdge } from '@/types/map'
import { healthColor } from '@/shared/logic/layout'
import { buildModuleTask, type TaskDraft } from '@/shared/logic/taskContext'
import type { ChatAboutTarget } from '@/shared/contract/chat'
import { couplingAnalysis, buildDepCards, type CouplingAnalysis, type DepCard } from '@/shared/logic/depsAnalysis'
import { Select } from '@/components/ui/SelectMenu'

// 依赖体检全页（docs/dependency-feature-design-v2.md 施工）：
// KPI 筛选卡 + 结论卡片清单（应当修复 / 值得留意 / ▸全部依赖默认折叠）+ 右半详情。
// 信息单元 = 三段式卡片（人话结论 + 证据 + 后果 + 行动）；decorative 维度全部收进折叠区。

interface Props {
  backendRepo: string | null
  map: CodeMap | null
  onCreateTask: (d: TaskDraft) => void
  onChatAbout: (t: ChatAboutTarget) => void
  /** 在画布中查看透镜（选中该模块 + 打开 solo 聚焦） */
  onInspectModule: (id: string) => void
  onOpenMap: () => void
  /** 画布边浮卡 [详情] 跳入时的聚焦卡片 */
  focusCardId?: string | null
  /** 用户手动选卡后清掉 focusCardId（用户选择优先于跳入聚焦） */
  onFocusConsumed?: () => void
}

type KpiFilter = 'violation' | 'cycle' | 'highRisk' | 'cross' | null

function KpiCard({ label, value, sub, tone, active, onClick }: {
  label: string; value: string; sub: string; tone: 'red' | 'amber'; active: boolean; onClick: () => void
}) {
  const color = tone === 'red' ? 'text-red-500' : 'text-amber-500'
  const bar = tone === 'red' ? 'bg-red-500' : 'bg-amber-500'
  const ring = active ? (tone === 'red' ? 'border-red-400 dark:border-red-700' : 'border-amber-400 dark:border-amber-700') : ''
  return (
    <button onClick={onClick} className={`lift relative rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-4 py-3 text-left ${ring}`}>
      <span className={`absolute left-0 top-[18px] h-9 w-1 rounded-r ${bar}`} />
      <p className="pl-2 text-cap text-slate-400 dark:text-slate-500">{label}</p>
      <p className={`tnum mt-1 pl-2 text-[22px] font-bold leading-6 ${color}`}>{value}</p>
      <p className="mt-0.5 pl-2 text-micro text-slate-400 dark:text-slate-500">{sub}</p>
    </button>
  )
}

function Card({ card, selected, onSelect }: { card: DepCard; selected: boolean; onSelect: () => void }) {
  const tone = card.severity === 'critical' ? 'text-red-500' : 'text-amber-500'
  const Icon = card.kind === 'cycle' ? Repeat : card.kind === 'highRisk' ? Flame : TriangleAlert
  return (
    <button
      onClick={onSelect}
      className={`w-full rounded-xl border bg-white dark:bg-slate-900 p-4 text-left transition-colors ${
        selected
          ? 'border-blue-400 dark:border-blue-700 shadow-sm'
          : 'border-slate-200 dark:border-slate-700 hover:border-slate-300 dark:hover:border-slate-600'
      }`}
    >
      <div className="flex items-start gap-2">
        <Icon size={14} className={`mt-0.5 shrink-0 ${tone}`} />
        <div className="min-w-0 flex-1">
          <p className="text-[13px] font-bold leading-5 text-slate-800 dark:text-slate-100">{card.title}</p>
          <p className="mt-1 text-cap leading-4 text-slate-400 dark:text-slate-500">{card.evidence}</p>
          <p className="mt-1.5 text-cap leading-4 text-slate-500 dark:text-slate-400">{card.consequence}</p>
        </div>
      </div>
    </button>
  )
}

/** 违规卡迷你分层示意：from（下）→ to（上）红虚线 */
function MiniDiagram({ card, a }: { card: DepCard; a: CouplingAnalysis }) {
  const from = card.sourceId ? a.moduleById.get(card.sourceId) : undefined
  const to = card.targetId ? a.moduleById.get(card.targetId) : undefined
  if (!from || !to) {
    // 循环群：成员芯片云
    if (card.members) {
      return (
        <div className="rounded-lg bg-slate-50 dark:bg-slate-950/70 p-3">
          <p className="mb-2 text-micro font-semibold text-slate-400 dark:text-slate-500">循环群成员（{card.members.length}）</p>
          <div className="flex flex-wrap gap-1.5">
            {card.members.map((id) => {
              const m = a.moduleById.get(id)
              return (
                <span key={id} className="rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-0.5 text-cap text-slate-600 dark:text-slate-300">
                  {m?.name ?? id}
                </span>
              )
            })}
          </div>
        </div>
      )
    }
    return null
  }
  const fc = healthColor(from.health.score)
  const tc = healthColor(to.health.score)
  return (
    <div className="space-y-2">
      {[
        { mod: to, color: tc, warn: false },
        { mod: from, color: fc, warn: true },
      ].map(({ mod, color, warn }, i) => (
        <div key={mod.id}>
          <div className={`flex items-center gap-2 rounded-lg border px-3 py-2 ${warn ? 'border-red-200 dark:border-red-900/60 bg-red-50 dark:bg-red-950/40' : 'border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900'}`}>
            <span className="h-2 w-2 rounded-full" style={{ background: color }} />
            <span className={`text-[12px] font-semibold ${warn ? 'text-red-600 dark:text-red-400' : 'text-slate-700 dark:text-slate-200'}`}>{mod.name}</span>
            <span className="ml-auto text-micro text-slate-400 dark:text-slate-500">{a.layerNameByModule(mod.id)}</span>
          </div>
          {i === 0 && (
            <div className="my-1 flex items-center gap-1.5 pl-4">
              <span className="h-4 border-l-2 border-dashed border-red-400" />
              <span className="text-micro text-red-500">▲ 下层调用上层 = 逆向</span>
            </div>
          )}
        </div>
      ))}
    </div>
  )
}

export function DepsPage({ backendRepo, map, onCreateTask, onChatAbout, onInspectModule, onOpenMap, focusCardId, onFocusConsumed }: Props) {
  const a = useMemo(() => (map ? couplingAnalysis(map) : null), [map])
  const cards = useMemo(() => (map && a ? buildDepCards(map, a) : []), [map, a])
  const [kpiFilter, setKpiFilter] = useState<KpiFilter>(null)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [showAllViolations, setShowAllViolations] = useState(false)
  const [foldOpen, setFoldOpen] = useState(false)
  const [foldFilter, setFoldFilter] = useState<'all' | 'violation' | 'cross' | 'strong'>('all')

  // 选中派生（React adjust-state-during-render 模式——lint 禁止 setState-in-effect）：
  // 画布浮卡跳入 > 用户已选 > 默认首卡；用户点卡片时经 onFocusConsumed 清掉 focusCardId
  const focusId = focusCardId && cards.some((c) => c.id === focusCardId) ? focusCardId : null
  const derived =
    focusId ?? (selectedId && cards.some((c) => c.id === selectedId) ? selectedId : cards[0]?.id ?? null)
  if (derived !== selectedId) setSelectedId(derived)
  const selected = cards.find((c) => c.id === derived) ?? null

  const violations = cards.filter((c) => c.kind === 'violation')
  const shownViolations = showAllViolations ? violations : violations.slice(0, 2)
  const notices = cards.filter((c) => c.kind !== 'violation')

  const foldEdges = useMemo(() => {
    if (!map || !a) return []
    let list: MapEdge[] = map.edges
    if (foldFilter === 'violation') list = list.filter((e) => e.direction_violation)
    if (foldFilter === 'cross') list = a.crossLayerSkips
    if (foldFilter === 'strong') list = list.filter((e) => e.strength === 'strong')
    return [...list].sort((x, y) => (y.direction_violation ? 1 : 0) - (x.direction_violation ? 1 : 0))
  }, [map, a, foldFilter])

  const nameOf = (id: string) => a?.moduleById.get(id)?.name ?? id

  if (!backendRepo) {
    return <div className="flex h-full items-center justify-center text-[12px] text-slate-400 dark:text-slate-500">先在左侧选择一个项目。</div>
  }
  if (!map || !a) {
    return (
      <div className="flex h-full items-center justify-center gap-2 text-[12px] text-slate-400 dark:text-slate-500">
        <Loader2 size={13} className="animate-spin" /> 地图加载中…
      </div>
    )
  }

  const groupVisible = (kind: DepCard['kind']) => kpiFilter === null || kpiFilter === kind
  const primaryModuleId = selected?.targetId ?? selected?.sourceId ?? null
  const inCycle = primaryModuleId ? a.cycleModuleIds.has(primaryModuleId) : false

  return (
    <div className="h-full overflow-y-auto p-5">
      <div className="mb-4 flex items-start justify-between">
        <div>
          <h2 className="flex items-center gap-1.5 text-[15px] font-bold text-slate-800 dark:text-slate-100">
            <Waypoints size={15} className="text-blue-500" /> 依赖体检
          </h2>
          <p className="mt-0.5 text-[11px] text-slate-400 dark:text-slate-500">
            这个仓库模块间的耦合体检：哪里被穿透、哪里在循环、哪里在传染。
          </p>
        </div>
        <button
          onClick={onOpenMap}
          className="flex items-center gap-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1.5 text-[11px] font-semibold text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70"
        >
          <MapIcon size={11} /> 在画布中查看
        </button>
      </div>

      {/* KPI 行：点击 = 筛选下方清单 */}
      <div className="mb-4 grid grid-cols-4 gap-3">
        <KpiCard label="逆向依赖" value={String(a.violations.length)} sub="条下层调用上层" tone="red" active={kpiFilter === 'violation'} onClick={() => setKpiFilter(kpiFilter === 'violation' ? null : 'violation')} />
        <KpiCard label="循环群" value={a.sccGroups.length ? `${a.sccGroups.length} 群` : '0'} sub={a.sccGroups[0] ? `${a.sccGroups[0].length} 个模块互相可达` : '无循环依赖'} tone="amber" active={kpiFilter === 'cycle'} onClick={() => setKpiFilter(kpiFilter === 'cycle' ? null : 'cycle')} />
        <KpiCard label="高风险耦合" value={`${a.highRisk.reduce((s, g) => s + g.edges.length, 0)} 条`} sub={`${a.highRisk.length} 个低分模块被强依赖`} tone="red" active={kpiFilter === 'highRisk'} onClick={() => setKpiFilter(kpiFilter === 'highRisk' ? null : 'highRisk')} />
        <KpiCard label="跨层直达" value={`${a.topToFoundation.length} 条`} sub="顶层模块直连基础支撑层" tone="amber" active={kpiFilter === 'cross'}
          onClick={() => { setKpiFilter(kpiFilter === 'cross' ? null : 'cross'); setFoldOpen(true); setFoldFilter('cross') }} />
      </div>

      <div className="grid grid-cols-5 gap-3">
        {/* 左列：卡片清单 */}
        <div className="col-span-3 space-y-4">
          {violations.length === 0 && notices.length === 0 && (
            <div className="rounded-xl border border-dashed border-slate-200 dark:border-slate-700 bg-white/60 px-4 py-8 text-center">
              <p className="text-[12px] font-semibold text-slate-500 dark:text-slate-400">当前架构没有耦合问题</p>
              <p className="mt-1 text-[11px] text-slate-400 dark:text-slate-500">模块间方向约束良好，循环与高风险耦合均未检出。</p>
            </div>
          )}

          {groupVisible('violation') && violations.length > 0 && (
            <section>
              <p className="mb-2 text-[13px] font-bold text-red-500">
                应当修复 <span className="ml-1 font-normal text-slate-400">（共 {violations.length} 条{showAllViolations ? '' : ' · 此处列最严重 2 条'}）</span>
              </p>
              <div className="space-y-2">
                {shownViolations.map((c) => (
                  <Card key={c.id} card={c} selected={selectedId === c.id} onSelect={() => { setSelectedId(c.id); onFocusConsumed?.() }} />
                ))}
              </div>
              {!showAllViolations && violations.length > 2 && (
                <button onClick={() => setShowAllViolations(true)} className="mt-2 text-[11px] font-semibold text-blue-600 hover:text-blue-700">
                  其余 {violations.length - 2} 条逆向违规 →
                </button>
              )}
            </section>
          )}

          {(kpiFilter === null || kpiFilter === 'cycle' || kpiFilter === 'highRisk') && notices.length > 0 && (
            <section>
              <p className="mb-2 text-[13px] font-bold text-amber-500">
                值得留意 <span className="ml-1 font-normal text-slate-400">（不紧急，但趋势值得盯）</span>
              </p>
              <div className="space-y-2">
                {notices
                  .filter((c) => (kpiFilter === 'cycle' ? c.kind === 'cycle' : kpiFilter === 'highRisk' ? c.kind === 'highRisk' : true))
                  .map((c) => (
                    <Card key={c.id} card={c} selected={selectedId === c.id} onSelect={() => { setSelectedId(c.id); onFocusConsumed?.() }} />
                  ))}
              </div>
            </section>
          )}

          {/* ▸ 全部依赖：v1 清单的收容所，默认折叠，一行一边不聚合 */}
          <section>
            <button onClick={() => setFoldOpen(!foldOpen)} className="flex items-center gap-1 text-[12px] font-semibold text-slate-500 dark:text-slate-400 hover:text-slate-700">
              {foldOpen ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
              全部依赖（{map.edges.length}）—— 内行展开后按类型 / 强度 / 层过滤
            </button>
            {foldOpen && (
              <div className="anim-fade-in-fast mt-2 rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
                <div className="flex items-center gap-2 border-b border-slate-100 dark:border-slate-800 px-3 py-2">
                  <Select
                    className="w-28"
                    value={foldFilter}
                    onChange={(v) => setFoldFilter(v as typeof foldFilter)}
                    options={[
                      { value: 'all', label: '全部' },
                      { value: 'violation', label: '仅违规' },
                      { value: 'cross', label: '跨层直达' },
                      { value: 'strong', label: '仅强耦合' },
                    ]}
                  />
                  <span className="text-micro text-slate-400 dark:text-slate-500">{foldEdges.length} 条</span>
                </div>
                <div className="max-h-72 overflow-y-auto">
                  {foldEdges.map((e) => (
                    <button
                      key={e.id}
                      onClick={() => setSelectedId(`vio-${e.id}`)}
                      className="flex w-full items-center gap-2 border-b border-slate-50 last:border-0 px-3 py-1.5 text-left hover:bg-slate-50 dark:hover:bg-slate-800/70"
                    >
                      {e.direction_violation && <TriangleAlert size={11} className="shrink-0 text-red-500" />}
                      <span className="truncate text-[12px] text-slate-600 dark:text-slate-300">{nameOf(e.from)}</span>
                      <span className="text-slate-300 dark:text-slate-600">→</span>
                      <span className="truncate text-[12px] text-slate-600 dark:text-slate-300">{nameOf(e.to)}</span>
                      <span className="ml-auto shrink-0 text-micro text-slate-400 dark:text-slate-500">
                        {e.type} · {e.label ?? '1 处引用'}
                      </span>
                    </button>
                  ))}
                  {foldEdges.length === 0 && <p className="px-3 py-6 text-center text-[11px] text-slate-300 dark:text-slate-600">无匹配项</p>}
                </div>
              </div>
            )}
          </section>
        </div>

        {/* 右列：详情 */}
        <div className="col-span-2">
          {selected && (
            <div className="sticky top-0 rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
              <p className="text-[13px] font-bold leading-5 text-slate-800 dark:text-slate-100">{selected.title}</p>
              <p className="mt-1 text-cap text-slate-400 dark:text-slate-500">
                {selected.kind === 'violation' ? `${selected.edge?.type} · ${selected.edge?.label ?? '1 处引用'} · 上次巡检 ${map.meta.last_patrol_at?.slice(0, 10) ?? '—'}` : selected.evidence}
              </p>

              <div className="mt-3">
                <MiniDiagram card={selected} a={a} />
              </div>

              {primaryModuleId && (
                <>
                  <p className="mt-4 text-cap font-semibold text-slate-400 dark:text-slate-500">度量 · {nameOf(primaryModuleId)}</p>
                  <div className="mt-1.5 grid grid-cols-4 gap-1.5">
                    {[
                      ['扇入', String(a.fanIn.get(primaryModuleId) ?? 0)],
                      ['扇出', String(a.fanOut.get(primaryModuleId) ?? 0)],
                      ['循环群', inCycle ? `${a.sccGroups[0]?.length ?? 0} 模块` : '不在环上'],
                      ['跨层跳数', selected.edge ? String(Math.abs(a.orderOf(selected.edge.from) - a.orderOf(selected.edge.to))) : '—'],
                    ].map(([k, v]) => (
                      <div key={k} className="rounded-lg bg-slate-50 dark:bg-slate-950/70 px-2 py-1.5">
                        <p className="text-micro text-slate-400 dark:text-slate-500">{k}</p>
                        <p className="tnum text-[13px] font-bold text-slate-700 dark:text-slate-200">{v}</p>
                      </div>
                    ))}
                  </div>
                </>
              )}

              <p className="mt-4 text-cap text-slate-400 dark:text-slate-500">即将支持精确定位到具体文件与代码行。</p>

              <div className="mt-3 flex flex-wrap items-center gap-1.5">
                {selected.targetId && (
                  <button
                    onClick={() => onCreateTask(buildModuleTask(map, selected.targetId!))}
                    className="flex items-center gap-1 rounded-md bg-blue-600 px-2.5 py-1.5 text-micro font-bold text-white hover:bg-blue-700"
                  >
                    <Wrench size={10} /> 发起修复
                  </button>
                )}
                {selected.targetId && (
                  <button
                    onClick={() => onChatAbout({ refId: selected.targetId!, refName: nameOf(selected.targetId!), kind: 'module' })}
                    className="flex items-center gap-1 rounded-md border border-slate-200 dark:border-slate-700 px-2.5 py-1.5 text-micro font-bold text-slate-600 dark:text-slate-300 hover:border-blue-300 hover:text-blue-600"
                  >
                    <MessagesSquare size={10} /> 问 agent
                  </button>
                )}
                <button
                  onClick={() => onInspectModule(selected.targetId ?? selected.sourceId ?? primaryModuleId ?? '')}
                  disabled={!primaryModuleId}
                  className="flex items-center gap-1 rounded-md border border-slate-200 dark:border-slate-700 px-2.5 py-1.5 text-micro font-bold text-slate-600 dark:text-slate-300 hover:border-blue-300 hover:text-blue-600 disabled:opacity-40"
                >
                  <MapIcon size={10} /> 在画布上看
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
