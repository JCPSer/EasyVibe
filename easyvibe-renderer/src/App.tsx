import { Component, useEffect, useMemo, useRef, useState, useCallback } from 'react'
import {
  ReactFlow,
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  Panel,
  Position,
  MarkerType,
  useReactFlow,
  ReactFlowProvider,
  type Edge,
  type Node,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { Activity, AlertTriangle, GitBranch, Loader2, PanelRightOpen, UnfoldVertical, FoldVertical, RefreshCw, Focus, Play, Pause, RotateCcw, X, Sparkles, Settings, Lightbulb, WifiOff} from 'lucide-react'

import type { CodeMap, GrowthEvent, SubMap } from '@/types/map'
import { layoutMap, healthColor, NODE_W, NODE_H, SUB_W, SUB_H } from '@/lib/layout'
import { ModuleNode, type ModuleNodeType } from '@/components/ModuleNode'
import { BandNode, type BandNodeType } from '@/components/BandNode'
import { ExpandedModuleNode, type ExpandedModuleNodeType } from '@/components/ExpandedModuleNode'
import { SubmoduleNode, type SubmoduleNodeType } from '@/components/SubmoduleNode'
import { DetailPanel, type Selection, type PanelTab } from '@/components/DetailPanel'
import { SettingsPanel } from '@/components/SettingsPanel'
import { TaskFormPanel } from '@/components/TaskFormPanel'
import type { TaskDraft } from '@/lib/taskContext'
import { isIssueModule } from '@/components/IssuesList'
import { emitFreshnessEvent, emitGrowthEvent, emitSessionEvent, emitSessionOutput, emitTaskEvent, notifyWsClosed, onFreshnessEvent, onGrowthEvent, onSessionEvent, onSessionOutput, setWsCloseListener } from '@/lib/growthBus'
import { isValidGrowthEvent, mergeGrowthEvents, parseGrowthText } from '@/lib/growthMerge'

const nodeTypes = { module: ModuleNode, moduleExpanded: ExpandedModuleNode, submodule: SubmoduleNode, band: BandNode }

interface Filters {
  violationsOnly: boolean
  issuesOnly: boolean
  solo: boolean // 只看选中模块的依赖（强聚焦）
}

const MAX_EXPANDED = 3

interface GrowthState {
  events: GrowthEvent[]
  index: number // 已消费事件数
  playing: boolean
  done: boolean
}

function buildFlow(
  map: CodeMap,
  selection: Selection,
  filters: Filters,
  expanded: Map<string, SubMap | 'loading' | 'error'>,
  onSelectLayer: (id: string) => void,
  retrySubmap?: (id: string) => void,
  growth?: { layers: Set<string>; modules: Set<string> } | null,
  analyzeSubmap?: (id: string) => void,
  agentLinesFor?: (id: string) => string[] | undefined,
) {
  // 展开元信息：加载中给 4 个骨架位
  const expandedMeta = new Map<string, { ids: string[]; loading: boolean }>()
  for (const [id, sm] of expanded) {
    expandedMeta.set(id, sm === 'loading' || sm === 'error' ? { ids: ['__s0', '__s1', '__s2', '__s3'], loading: true } : { ids: sm.sub_modules.map((s) => s.id), loading: false })
  }
  const { positions, bands, blocks } = layoutMap(map, expandedMeta)

  const inCount = new Map<string, number>()
  const outCount = new Map<string, number>()
  for (const e of map.edges) {
    outCount.set(e.from, (outCount.get(e.from) ?? 0) + 1)
    inCount.set(e.to, (inCount.get(e.to) ?? 0) + 1)
  }

  // 选中模块的邻域：自身 + 直接依赖/被依赖
  const neighborhood = new Set<string>()
  const selModuleId = selection?.kind === 'module' ? selection.id : selection?.kind === 'submodule' ? selection.parentId : null
  if (selModuleId) {
    neighborhood.add(selModuleId)
    for (const e of map.edges) {
      if (e.from === selModuleId) neighborhood.add(e.to)
      if (e.to === selModuleId) neighborhood.add(e.from)
    }
  }
  const issueIds = new Set(map.modules.filter(isIssueModule).map((m) => m.id))

  const nodeDim = (id: string) => {
    let op = 1
    if (selModuleId && !neighborhood.has(id)) op *= filters.solo ? 0.1 : 0.3
    if (filters.issuesOnly && !issueIds.has(id)) op *= 0.22
    return op
  }

  const nodes: Node[] = [
    ...bands.flatMap((box, i): BandNodeType[] => {
      const layer = map.layers.find((l) => l.id === box.layerId)!
      if (growth && !growth.layers.has(layer.id)) return []
      const mods = map.modules.filter((m) => m.layer === layer.id)
      const avgScore = Math.round(mods.reduce((s, m) => s + m.health.score, 0) / Math.max(mods.length, 1))
      const modIds = new Set(mods.map((m) => m.id))
      const violations = map.edges.filter(
        (e) => e.direction_violation && (modIds.has(e.from) || modIds.has(e.to)),
      ).length
      return [{
        id: `band-${box.layerId}`,
        type: 'band',
        position: { x: box.x, y: box.y },
        data: {
          layer, box, index: i,
          stats: { count: mods.length, avgScore, violations },
          selected: selection?.kind === 'layer' && selection.id === layer.id,
          onSelect: onSelectLayer,
        },
        draggable: false,
        selectable: false,
        zIndex: -1,
        width: box.width,
        height: box.height,
        className: growth ? 'growth-born' : undefined,
      }]
    }),
    ...map.modules.flatMap((mod): Node[] => {
      if (growth && !growth.modules.has(mod.id)) return []
      if (expanded.has(mod.id)) {
        const block = blocks.get(mod.id)!
        const sm = expanded.get(mod.id)!
        return [{
          id: mod.id,
          type: 'moduleExpanded',
          position: positions.get(mod.id)!,
          data: {
            module: mod,
            inCount: inCount.get(mod.id) ?? 0,
            outCount: outCount.get(mod.id) ?? 0,
            loading: sm === 'loading',
            error: sm === 'error',
            subCount: sm === 'loading' || sm === 'error' ? 0 : sm.sub_modules.length,
            onRetry: retrySubmap ? () => retrySubmap(mod.id) : undefined,
            onAnalyze: analyzeSubmap ? () => analyzeSubmap(mod.id) : undefined,
            agentLines: agentLinesFor?.(mod.id),
          },
          sourcePosition: Position.Bottom,
          targetPosition: Position.Top,
          width: block.width,
          height: block.height,
          style: { opacity: nodeDim(mod.id) },
          zIndex: 1,
          className: growth ? 'growth-born' : undefined,
        } satisfies ExpandedModuleNodeType]
      }
      return [{
        id: mod.id,
        type: 'module',
        position: positions.get(mod.id)!,
        data: { module: mod, inCount: inCount.get(mod.id) ?? 0, outCount: outCount.get(mod.id) ?? 0 },
        sourcePosition: Position.Bottom,
        targetPosition: Position.Top,
        width: NODE_W,
        height: NODE_H,
        style: { opacity: nodeDim(mod.id) },
        className: growth ? 'growth-born' : undefined,
      } satisfies ModuleNodeType]
    }),
    // 子模块节点（父节点内部，extent=parent）
    ...[...blocks.entries()].flatMap(([parentId, block]): Node[] => {
      const sm = expanded.get(parentId)!
      const parent = map.modules.find((m) => m.id === parentId)!
      const inDeg = new Map<string, number>()
      const outDeg = new Map<string, number>()
      if (sm !== 'loading' && sm !== 'error') {
        for (const e of sm.edges) {
          outDeg.set(e.from, (outDeg.get(e.from) ?? 0) + 1)
          inDeg.set(e.to, (inDeg.get(e.to) ?? 0) + 1)
        }
      }
      return block.childOrder.map((sid): Node => ({
        id: `sub:${parentId}:${sid}`,
        type: 'submodule',
        position: block.childPositions.get(sid)!,
        data: {
          sub: sm === 'loading' || sm === 'error' ? undefined : sm.sub_modules.find((s) => s.id === sid),
          loading: sm === 'loading' || sm === 'error',
          inCount: inDeg.get(sid) ?? 0,
          outCount: outDeg.get(sid) ?? 0,
          parentName: parent.name,
        },
        parentId,
        extent: 'parent',
        draggable: false,
        width: SUB_W,
        height: SUB_H,
      }) satisfies SubmoduleNodeType)
    }),
  ]

  const inIdx = new Map<string, number>()
  const outIdx = new Map<string, number>()

  const edgeDim = (touches: boolean, isViolation: boolean) => {
    let dim = 1
    if (selModuleId && !touches) dim *= filters.solo ? 0.04 : 0.07
    if (filters.violationsOnly && !isViolation) dim *= 0.06
    if (filters.issuesOnly && !touches) dim *= 0.5
    return dim
  }

  const visibleEdges = growth
    ? map.edges.filter((e) => growth.modules.has(e.from) && growth.modules.has(e.to))
    : map.edges
  const edges: Edge[] = visibleEdges.map((e, i) => {
    const violation = e.direction_violation === true
    const si = Math.min(outIdx.get(e.from) ?? 0, 17) // 与节点 MAX_HANDLES=18 对齐，超出复用最后一个 handle
    outIdx.set(e.from, (outIdx.get(e.from) ?? 0) + 1)
    const ti = Math.min(inIdx.get(e.to) ?? 0, 17)
    inIdx.set(e.to, (inIdx.get(e.to) ?? 0) + 1)
    const strengthStyle =
      e.strength === 'strong'
        ? { strokeWidth: 2.1, stroke: '#64748b', opacity: 0.85 }
        : e.strength === 'normal'
          ? { strokeWidth: 1.6, stroke: '#94a3b8', opacity: 0.7 }
          : { strokeWidth: 1.1, stroke: '#cbd5e1', opacity: 0.6 }
    const dim = edgeDim(e.from === selModuleId || e.to === selModuleId, violation)
    return {
      id: `e-${e.id ?? i}`,
      source: e.from,
      target: e.to,
      sourceHandle: `s${si}`,
      targetHandle: `t${ti}`,
      type: 'default',
      style: violation
        ? { strokeWidth: 1.8, stroke: '#ef4444', strokeDasharray: '6 4', opacity: 0.9 * dim }
        : { ...strengthStyle, opacity: strengthStyle.opacity * dim },
      animated: violation && dim > 0.5,
      label: violation && dim > 0.5 ? '⚠' : undefined,
      labelStyle: { fill: '#ef4444', fontSize: 13, fontWeight: 700 },
      labelBgStyle: { fill: 'rgba(255,255,255,0.9)', fillOpacity: 0.9 },
      markerEnd: { type: MarkerType.ArrowClosed, width: 14, height: 14, color: violation ? '#ef4444' : '#94a3b8' },
    } satisfies Edge
  })

  // 内部连线（展开模块的子图边）
  for (const [parentId, sm] of expanded) {
    if (sm === 'loading' || sm === 'error') continue
    const sin = new Map<string, number>()
    const sout = new Map<string, number>()
    sm.edges.forEach((e, i) => {
      const cyclic = e.circular_dep === true
      const si = Math.min(sout.get(e.from) ?? 0, 9) // 子模块节点 MAX_HANDLES=10
      sout.set(e.from, (sout.get(e.from) ?? 0) + 1)
      const ti = Math.min(sin.get(e.to) ?? 0, 9)
      sin.set(e.to, (sin.get(e.to) ?? 0) + 1)
      const touches = selection?.kind === 'submodule'
        ? (selection.parentId === parentId && (selection.subId === e.from || selection.subId === e.to))
        : selModuleId === parentId || !selModuleId
      let dim = touches ? 1 : selModuleId ? 0.08 : 1
      if (filters.violationsOnly) dim *= 0.15
      const base = cyclic
        ? { strokeWidth: 1.7, stroke: '#f97316', strokeDasharray: '5 4', opacity: 0.9 }
        : e.strength === 'strong'
          ? { strokeWidth: 1.8, stroke: '#64748b', opacity: 0.7 }
          : { strokeWidth: 1.3, stroke: '#94a3b8', opacity: 0.55 }
      edges.push({
        id: `se-${parentId}-${i}`,
        source: `sub:${parentId}:${e.from}`,
        target: `sub:${parentId}:${e.to}`,
        sourceHandle: `s${si}`,
        targetHandle: `t${ti}`,
        type: 'default',
        style: { ...base, opacity: base.opacity * dim },
        animated: cyclic && dim > 0.5,
        label: cyclic && dim > 0.5 ? '↻' : undefined,
        labelStyle: { fill: '#f97316', fontSize: 12, fontWeight: 700 },
        labelBgStyle: { fill: 'rgba(255,255,255,0.9)', fillOpacity: 0.9 },
        markerEnd: { type: MarkerType.ArrowClosed, width: 12, height: 12, color: cyclic ? '#f97316' : '#94a3b8' },
        zIndex: 5,
      })
    })
  }

  return { nodes, edges }
}

function Legend({ violations }: { violations: number }) {
  return (
    <div className="space-y-1.5 rounded-xl border border-slate-200 bg-white/95 px-3.5 py-3 text-[11px] text-slate-600 shadow-sm backdrop-blur">
      <div className="mb-1 text-[10px] font-bold uppercase tracking-wider text-slate-400">图例</div>
      <div className="flex items-center gap-2">
        <span className="inline-block h-2.5 w-2.5 rounded-full" style={{ background: '#10b981' }} /> Healthy（≥75）
      </div>
      <div className="flex items-center gap-2">
        <span className="inline-block h-2.5 w-2.5 rounded-full" style={{ background: '#f59e0b' }} /> Warning（60–74）
      </div>
      <div className="flex items-center gap-2">
        <span className="inline-block h-2.5 w-2.5 rounded-full" style={{ background: '#ef4444' }} /> Error（&lt;60）
      </div>
      <div className="flex items-center gap-2">
        <span className="inline-block w-5 border-t-2 border-slate-400" /> Dependency
      </div>
      <div className="flex items-center gap-2">
        <span className="inline-block w-5 border-t-2 border-dashed border-red-500" />
        <span className="flex items-center gap-1">
          <AlertTriangle size={11} className="text-red-500" /> 逆向依赖 violation（{violations}）
        </span>
      </div>
      <div className="flex items-center gap-2">
        <span className="inline-block w-5 border-t-2 border-dashed border-orange-500" />
        <span className="flex items-center gap-1 text-orange-600">内部循环依赖（子图）</span>
      </div>
    </div>
  )
}

function FilterButton({ active, onClick, label, activeClass }: { active: boolean; onClick: () => void; label: string; activeClass: string }) {
  return (
    <button
      onClick={onClick}
      className={`rounded-full border px-3 py-1 text-[11px] font-semibold transition-colors ${
        active ? activeClass : 'border-transparent text-slate-500 hover:bg-slate-100'
      }`}
    >
      {label}
    </button>
  )
}

function ArchHealthCard({ map }: { map: CodeMap }) {
  const archColor = healthColor(map.health.score)
  return (
    <div
      className="flex items-center gap-2.5 rounded-xl border bg-white/95 px-4 py-2.5 shadow-sm backdrop-blur"
      style={{ borderColor: `${archColor}66` }}
      title={map.health.review_note}
    >
      <Activity size={15} style={{ color: archColor }} />
      <div>
        <div className="text-[10px] font-semibold uppercase tracking-wider text-slate-400">架构健康</div>
        <div className="flex items-baseline gap-1.5">
          <span className="text-[16px] font-bold leading-5" style={{ color: archColor }}>
            {map.health.score}
          </span>
          <span className="text-[10px] text-slate-400">/ 100 · {map.health.coupling}</span>
        </div>
      </div>
      {map.health.decay_flags.length > 0 && (
        <div className="ml-1 flex max-w-[200px] flex-wrap gap-1">
          {map.health.decay_flags.map((f) => (
            <span key={f} className="rounded-full bg-red-50 px-1.5 py-px text-[9px] text-red-600">
              {f}
            </span>
          ))}
        </div>
      )}
    </div>
  )
}

// 选中模块时的横向工具栏（F1a）
function ModuleToolbar({
  moduleName,
  expanded,
  backendActive,
  inducing,
  solo,
  onToggleExpand,
  onToggleSolo,
  onReinduce,
}: {
  moduleName: string
  expanded: boolean
  backendActive: boolean
  inducing: boolean
  solo: boolean
  onToggleExpand: () => void
  onToggleSolo: () => void
  onReinduce: () => void
}) {
  return (
    <div className="flex items-center gap-1 rounded-full border border-slate-200 bg-white/95 py-1.5 pl-4 pr-2 shadow-sm backdrop-blur">
      <span className="mr-1 max-w-[180px] truncate text-[11.5px] font-bold text-slate-700">{moduleName}</span>
      <button
        onClick={onToggleExpand}
        className={`flex items-center gap-1 rounded-full border px-3 py-1 text-[11px] font-semibold transition-colors ${
          expanded ? 'border-blue-300 bg-blue-50 text-blue-600' : 'border-transparent text-slate-600 hover:bg-slate-100'
        }`}
      >
        {expanded ? <FoldVertical size={12} /> : <UnfoldVertical size={12} />}
        {expanded ? '收起内部' : '展开内部结构'}
      </button>
      <button
        onClick={onToggleSolo}
        className={`flex items-center gap-1 rounded-full border px-3 py-1 text-[11px] font-semibold transition-colors ${
          solo ? 'border-indigo-300 bg-indigo-50 text-indigo-600' : 'border-transparent text-slate-500 hover:bg-slate-100'
        }`}
      >
        <Focus size={12} />
        {solo ? '✕ 退出聚焦' : '只看依赖'}
      </button>
      <button
        onClick={onReinduce}
        disabled={!backendActive || inducing}
        title={
          !backendActive
            ? '需要本地后端服务（cargo run 启动后可用）'
            : inducing
              ? '归纳进行中，完成后自动恢复'
              : '重新归纳该仓库：spawn agent 按 v2.2 协议执行，全程直播'
        }
        className={`flex items-center gap-1 rounded-full border px-3 py-1 text-[11px] font-semibold transition-colors ${
          inducing
            ? 'cursor-wait border-amber-300 bg-amber-50 text-amber-700'
            : backendActive
              ? 'border-transparent text-slate-600 hover:bg-slate-100'
              : 'cursor-not-allowed border-transparent text-slate-300'
        }`}
      >
        <RefreshCw size={12} className={inducing ? 'animate-spin' : ''} />
        {inducing ? '归纳中…' : '重新归纳'}
      </button>
    </div>
  )
}

// 生长回放控制条（消费 v2.2 growth.log 事件流）
function GrowthPanel({
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
    <div className="flex w-[460px] items-center gap-3 rounded-xl border border-slate-200 bg-white/95 px-4 py-2.5 shadow-sm backdrop-blur">
      <button
        onClick={onPause}
        disabled={growth.done}
        className="rounded-full p-1.5 text-slate-500 hover:bg-slate-100 hover:text-slate-700 disabled:opacity-30"
        title={growth.playing ? '暂停' : '继续'}
      >
        {growth.playing ? <Pause size={14} /> : <Play size={14} />}
      </button>
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between">
          <span className="truncate text-[11.5px] font-semibold text-slate-700">
            <Sparkles size={11} className="mr-1 inline text-blue-500" />
            {status}
          </span>
          <span className="text-[10px] tabular-nums text-slate-400">{pct}%</span>
        </div>
        <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-slate-100">
          <div
            className={`h-full rounded-full transition-all duration-500 ${growth.done ? 'bg-emerald-500' : 'bg-blue-500'}`}
            style={{ width: `${pct}%` }}
          />
        </div>
      </div>
      <button onClick={onRestart} className="rounded-full p-1.5 text-slate-400 hover:bg-slate-100 hover:text-slate-600" title="重播">
        <RotateCcw size={13} />
      </button>
      <button onClick={onExit} className="rounded-full p-1.5 text-slate-400 hover:bg-slate-100 hover:text-slate-600" title="退出演示">
        <X size={14} />
      </button>
    </div>
  )
}

function Canvas({
  map,
  backendRepo,
  repos,
  onRepoChange,
}: {
  map: CodeMap
  backendRepo: string | null
  repos: { id: string; name: string }[]
  onRepoChange: (id: string) => void
}) {
  const [selection, setSelection] = useState<Selection>(null)
  const [panelOpen, setPanelOpen] = useState(true)
  // 改进#4：右栏可调宽（340–560）
  const [panelWidth, setPanelWidth] = useState(340)
  const startPanelDrag = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    const startX = e.clientX
    const startW = panelWidth
    const onMove = (ev: MouseEvent) => setPanelWidth(Math.min(560, Math.max(340, startW + (startX - ev.clientX))))
    const onUp = () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }, [panelWidth])
  const [tab, setTab] = useState<PanelTab>('issues')
  const [filters, setFilters] = useState<Filters>({ violationsOnly: false, issuesOnly: false, solo: false })
  const [expandedIds, setExpandedIds] = useState<string[]>([])
  const [submaps, setSubmaps] = useState<Record<string, SubMap | 'loading' | 'error'>>({})
  const [growth, setGrowth] = useState<GrowthState | null>(null)
  const [liveActivity, setLiveActivity] = useState(false)
  const [inducing, setInducing] = useState(false)
  const [patrolling, setPatrolling] = useState(false)
  const [freshness, setFreshness] = useState<string | null>(null)
  // 改进#2：agent 过程直播——按会话存最近输出（子图分析/任务执行）
  const [agentLines, setAgentLines] = useState<Record<string, string[]>>({})
  // 子图分析会话号（模块 id → sessionId，用于匹配输出流）
  const [submapSessions, setSubmapSessions] = useState<Record<string, string>>({})
  const [freshnessInfo, setFreshnessInfo] = useState<{ commitsSinceMap?: number | null }>({})
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [taskDraft, setTaskDraft] = useState<TaskDraft | null>(null)
  const growthRef = useRef<GrowthState | null>(null)
  const submapsRef = useRef<typeof submaps | null>(null)
  useEffect(() => {
    growthRef.current = growth
  }, [growth])
  useEffect(() => {
    submapsRef.current = submaps
  }, [submaps])

  // R2：WS 断线 → 退出生长模式（重连后由用户重新进入，startGrowth 拉全量对齐）
  useEffect(() => setWsCloseListener(() => setGrowth(null)), [])

  // 真人测试反馈#1："只看依赖"是单程票——Esc 退出 + 底部常驻指示条
  useEffect(() => {
    if (!filters.solo) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setFilters((f) => ({ ...f, solo: false }))
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [filters.solo])

  // 改进#2：订阅 agent 输出流，按会话保留最近 4 行
  useEffect(
    () =>
      onSessionOutput((e) => {
        setAgentLines((prev) => {
          const cur = prev[e.sessionId] ?? []
          return { ...prev, [e.sessionId]: [...cur.slice(-3), e.line] }
        })
      }),
    [],
  )

  // S2：地图保鲜——启动拉一次 + WS freshness.changed 增量（git 有新提交而地图未更新）
  useEffect(() => {
    if (!backendRepo) return
    fetch(`/api/repos/${backendRepo}/freshness`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: { status: string; commitsSinceMap?: number | null } }) => {
        setFreshness(d.data.status === 'fresh' ? null : d.data.status)
        setFreshnessInfo({ commitsSinceMap: d.data.commitsSinceMap })
      })
      .catch(() => {})
    return onFreshnessEvent((e) => {
      if (e.repo !== backendRepo) return
      setFreshness(e.status === 'fresh' ? null : e.status)
      setFreshnessInfo({ commitsSinceMap: e.commitsSinceMap })
    })
  }, [backendRepo])

  // 会话状态：终态（succeeded/failed）解除"归纳中"/"巡检中"（patrol 会话以 patrol- 前缀区分）
  useEffect(
    () =>
      onSessionEvent((evt) => {
        if (evt.status !== 'succeeded' && evt.status !== 'failed') return
        setInducing(false)
        if (evt.sessionId.startsWith('patrol-')) setPatrolling(false)
      }),
    [],
  )

  // 直播订阅：WS 到达的 growth.event 追加进当前生长会话；未在生长模式则点亮"归纳活动"指示
  useEffect(
    () =>
      onGrowthEvent((event) => {
        if (!isValidGrowthEvent(event)) return // 坏消息直接丢弃（R1）
        const g = growthRef.current
        if (!g) {
          setLiveActivity(true)
          return
        }
        setGrowth({ ...g, events: [...g.events, event] })
      }),
    [],
  )
  const [headerExpanded, setHeaderExpanded] = useState(false)
  const { fitView, setCenter } = useReactFlow()

  // 生长回放：消费 growth.log（v2.2 协议），已到达的层/模块集合
  const arrived = useMemo(() => {
    if (!growth) return null
    const layers = new Set<string>()
    const modules = new Set<string>()
    for (let i = 0; i < growth.index && i < growth.events.length; i++) {
      const e = growth.events[i]
      if (e.type === 'layer') layers.add(e.layer.id)
      if (e.type === 'module') modules.add(e.module.id)
    }
    return { layers, modules }
  }, [growth])

  // 事件推进定时器：500ms 一帧；直播模式下新事件经总线追加后由同一节奏点亮
  useEffect(() => {
    if (!growth?.playing) return
    const t = setInterval(() => {
      setGrowth((g) => {
        if (!g || !g.playing) return g
        if (g.index >= g.events.length) {
          const last = g.events[g.events.length - 1]
          return last?.type === 'done' ? { ...g, playing: false, done: true } : g
        }
        return { ...g, index: g.index + 1 }
      })
    }, 500)
    return () => clearInterval(t)
  }, [growth?.playing])

  const startGrowth = useCallback(() => {
    const url = backendRepo ? `/api/repos/${backendRepo}/growth` : '/data/growth.log'
    fetch(url)
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.text()
      })
      .then((text) => {
        const events = parseGrowthText(text, !!backendRepo) // 坏行/坏消息跳过（Y1/R1）
        setSelection(null)
        setExpandedIds([])
        setLiveActivity(false)
        setGrowth({ events, index: 0, playing: true, done: false })
      })
      .catch(() => setGrowth(null))
  }, [backendRepo])

  const onSelectLayer = useCallback((id: string) => {
    setSelection({ kind: 'layer', id })
    setTab('detail')
    setPanelOpen(true)
  }, [])

  const toggleExpand = useCallback((id: string) => {
    setExpandedIds((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id].slice(-MAX_EXPANDED)))
    setSubmaps((prev) => (prev[id] ? prev : { ...prev, [id]: 'loading' }))
  }, [])

  // 写路径（M2-3）：触发重新归纳 → 后端 spawn agent 按 v2.2 执行；自动进入直播模式看生长
  const startReinduce = useCallback(() => {
    if (!backendRepo || inducing) return
    setInducing(true)
    fetch(`/api/repos/${backendRepo}/reinduce`, { method: 'POST' })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        startGrowth()
      })
      .catch(() => setInducing(false))
  }, [backendRepo, inducing, startGrowth])

  // 巡检（M2-4）：Supervisor 直调 LLM，产出新地图原子写回 + 健康历史落库
  const startPatrol = useCallback(() => {
    if (!backendRepo || patrolling) return
    setPatrolling(true)
    fetch(`/api/repos/${backendRepo}/patrol`, { method: 'POST' })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
      })
      .catch(() => setPatrolling(false))
  }, [backendRepo, patrolling])

  // 懒加载子图：任何 loading 状态触发取数（后端模式走 API，否则静态文件）
  useEffect(() => {
    for (const [id, v] of Object.entries(submaps)) {
      if (v !== 'loading') continue
      const url = backendRepo ? `/api/repos/${backendRepo}/modules/${id}` : `/data/modules/${id}.json`
      fetch(url)
        .then((r) => {
          if (!r.ok) throw new Error(String(r.status))
          return r.json() as Promise<SubMap>
        })
        .then((d) => setSubmaps((p) => ({ ...p, [id]: d })))
        .catch(() => {
          // R4：失败保留展开态并标记 error（容器内显示重试），不再无声消失
          setSubmaps((p) => ({ ...p, [id]: 'error' }))
        })
    }
  }, [submaps, backendRepo])

  const retrySubmap = useCallback((id: string) => {
    setSubmaps((prev) => ({ ...prev, [id]: 'loading' }))
  }, [])

  // 子图深入分析：派透明 agent 扫描模块文件生成子图，落盘后即可加载。
  // 启动后轮询重试（6s×20=2 分钟），产物就绪自动刷新展开态
  const analyzeSubmap = useCallback(
    (id: string) => {
      if (!backendRepo) return
      fetch(`/api/repos/${backendRepo}/modules/${encodeURIComponent(id)}/analyze-submap`, { method: 'POST' })
        .then(async (r) => {
          if (!r.ok) throw new Error(String(r.status))
          const sess = (await r.json()) as { sessionId: string }
          setSubmapSessions((prev) => ({ ...prev, [id]: sess.sessionId }))
          setSubmaps((prev) => ({ ...prev, [id]: 'loading' }))
          let n = 0
          const t = window.setInterval(() => {
            n += 1
            const cur = submapsRef.current?.[id]
            if ((cur && cur !== 'loading' && cur !== 'error') || n >= 20) {
              window.clearInterval(t)
              return
            }
            retrySubmap(id)
          }, 6000)
        })
        .catch(() => setSubmaps((prev) => ({ ...prev, [id]: 'error' })))
    },
    [backendRepo, retrySubmap],
  )

  const expanded = useMemo(() => {
    const m = new Map<string, SubMap | 'loading' | 'error'>()
    for (const id of expandedIds) if (submaps[id]) m.set(id, submaps[id])
    return m
  }, [expandedIds, submaps])

  // 直播合并：生长事件的模块/层/边合入基准地图——map.json 在归纳完成前不含新模块，
  // 生长事件本身才是"进行中模块"的事实源。合并用全量事件（布局稳定），显隐由 arrived 控制。
  const mergedMap = useMemo(
    () => (growth ? mergeGrowthEvents(map, growth.events) : map),
    [map, growth?.events],
  )

  const emptyExpanded = useMemo(() => new Map<string, SubMap | 'loading' | 'error'>(), [])
  const effectiveExpanded = growth ? emptyExpanded : expanded
  const growthVisible = growth ? arrived : null

  const { nodes, edges } = useMemo(
    () => buildFlow(mergedMap, selection, filters, effectiveExpanded, onSelectLayer, retrySubmap, growthVisible, analyzeSubmap, (id) => agentLines[submapSessions[id] ?? '']),
    [mergedMap, selection, filters, effectiveExpanded, onSelectLayer, retrySubmap, growthVisible, analyzeSubmap, agentLines, submapSessions],
  )

  // S1-1 画布定位收口：pan/zoom 到目标模块 + 选中高亮。
  // 对话 refs 芯片 / 问题清单定位 / 视图打开三处共用（此前只切右栏、画布毫无反馈）
  const focusModule = useCallback(
    (id: string) => {
      const node = nodes.find((n) => n.id === id)
      setSelection({ kind: 'module', id })
      setTab('detail')
      setPanelOpen(true)
      if (!node) return
      const w = node.measured?.width ?? node.width ?? 220
      setCenter(node.position.x + w / 2, node.position.y + 70, { zoom: 1.15, duration: 450 })
    },
    [nodes],
  )

  // S1-1 视图打开：定位首模块 + 多模块视图自动 solo 聚焦（复用 dim 机制，兑现跨模块视图）
  const openView = useCallback(
    (ids: string[]) => {
      if (ids.length === 0) return
      // 重置既有过滤（试用测试抓到：旧"只看依赖"会污染视图的 solo 聚焦）
      setFilters({ violationsOnly: false, issuesOnly: false, solo: ids.length > 1 })
      focusModule(ids[0])
    },
    [focusModule],
  )

  useEffect(() => {
    const t = setTimeout(() => fitView({ padding: 0.12, duration: 300 }), 60)
    return () => clearTimeout(t)
  }, [fitView, map])

  const onNodeClick = useCallback((_e: unknown, node: Node) => {
    if (growth) return // 生长回放期间禁用选中
    if (node.type === 'module' || node.type === 'moduleExpanded') {
      setSelection({ kind: 'module', id: node.id })
      setTab('detail')
      setPanelOpen(true)
    } else if (node.type === 'submodule' && node.parentId) {
      // 加载中的骨架不可选中（其 id 是占位符，选中后数据到达会无法匹配）
      if ((node.data as { loading?: boolean }).loading) return
      const subId = node.id.slice(`sub:${node.parentId}:`.length)
      setSelection({ kind: 'submodule', parentId: node.parentId, subId })
      setTab('detail')
      setPanelOpen(true)
    }
  }, [growth])
  const onPaneClick = useCallback(() => setSelection(null), [])

  const toggleFilter = (key: keyof Filters) => setFilters((f) => ({ ...f, [key]: !f[key] }))

  const violations = mergedMap.edges.filter((e) => e.direction_violation).length
  // 工具栏对模块选中与其子模块选中都生效（收起/展开操作的是父模块）
  const toolbarModuleId = selection?.kind === 'module' ? selection.id : selection?.kind === 'submodule' ? selection.parentId : null
  const selModule = toolbarModuleId ? map.modules.find((m) => m.id === toolbarModuleId) : undefined

  return (
    <div className="flex h-screen w-full overflow-hidden bg-slate-50">
      {/* 中央画布 */}
      <div className="relative flex-1">
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onNodeClick={onNodeClick}
          onPaneClick={onPaneClick}
          nodesDraggable={false}
          minZoom={0.08}
          maxZoom={1.6}
          proOptions={{ hideAttribution: true }}
          nodesConnectable={false}
          deleteKeyCode={null}
        >
          <Background variant={BackgroundVariant.Dots} gap={28} size={1.2} color="#cbd5e1" />
          <Controls showInteractive={false} position="bottom-left" />
          <MiniMap
            position="bottom-right"
            pannable
            zoomable
            bgColor="#f8fafc"
            nodeColor={(n) =>
              n.type === 'module' || n.type === 'moduleExpanded'
                ? healthColor((n.data as { module: { health: { score: number } } }).module.health.score)
                : 'rgba(0,0,0,0)'
            }
            maskColor="rgba(226,232,240,0.7)"
            style={{ width: 200, height: 130 }}
          />

          {/* 右上角：架构健康 + 图例 */}
          <Panel position="top-right" className="flex flex-col gap-2">
            <ArchHealthCard map={map} />
            <Legend violations={violations} />
          </Panel>

          {/* 顶部中央：选中模块的横向工具栏（F1a）；mt 让出头部卡片高度（展开简介时更高），窄屏不遮挡 */}
          {selModule && (
            <Panel position="top-center" style={{ marginTop: headerExpanded ? 200 : 110 }}>
              <ModuleToolbar
                moduleName={selModule.name}
                expanded={expandedIds.includes(selModule.id)}
                backendActive={!!backendRepo}
                inducing={inducing}
                solo={filters.solo}
                onToggleExpand={() => toggleExpand(selModule.id)}
                onToggleSolo={() => toggleFilter('solo')}
                onReinduce={startReinduce}
              />
            </Panel>
          )}

          {/* 底部中央：生长回放控制条 或 全局过滤 */}
          <Panel position="bottom-center" className="mb-2">
            {growth ? (
              <GrowthPanel
                growth={growth}
                onPause={() => setGrowth((g) => (g ? { ...g, playing: !g.playing } : g))}
                onRestart={() => setGrowth((g) => (g ? { ...g, index: 0, playing: true, done: false } : g))}
                onExit={() => setGrowth(null)}
              />
            ) : (
              <div className="flex items-center gap-1.5 rounded-full border border-slate-200 bg-white/95 px-2 py-1.5 shadow-sm backdrop-blur">
                {/* 真人测试#1：聚焦常驻指示——任何时刻看得见、一键退得出（Esc 同效） */}
                {filters.solo && selModule && (
                  <button
                    onClick={() => setFilters((f) => ({ ...f, solo: false }))}
                    className="flex items-center gap-1 rounded-full border border-indigo-300 bg-indigo-50 px-2.5 py-1 text-[10.5px] font-semibold text-indigo-600 hover:bg-indigo-100"
                    title="退出聚焦（Esc）"
                  >
                    <Focus size={10} />
                    聚焦：{selModule.name} <span className="text-indigo-400">✕</span>
                  </button>
                )}
                <FilterButton
                  active={filters.violationsOnly}
                  onClick={() => toggleFilter('violationsOnly')}
                  label="只看违规"
                  activeClass="border-red-300 bg-red-50 text-red-600"
                />
                <FilterButton
                  active={filters.issuesOnly}
                  onClick={() => toggleFilter('issuesOnly')}
                  label="问题视图"
                  activeClass="border-amber-300 bg-amber-50 text-amber-700"
                />
                {(filters.violationsOnly || filters.issuesOnly) && (
                  <button
                    onClick={() => setFilters({ violationsOnly: false, issuesOnly: false, solo: filters.solo })}
                    className="rounded-full px-2 py-1 text-[10.5px] text-slate-400 hover:text-slate-600"
                  >
                    重置
                  </button>
                )}
              </div>
            )}
          </Panel>
        </ReactFlow>

        {/* 头部信息条 */}
        <div className="pointer-events-none absolute left-0 top-0 z-10 w-full">
          <div className="px-5 py-3">
            <div className="pointer-events-auto inline-block rounded-xl border border-slate-200 bg-white/95 px-4 py-2.5 shadow-sm backdrop-blur">
              <div className="flex items-center gap-2">
                <span className="text-[10px] font-bold uppercase tracking-widest text-blue-600">EasyVibe</span>
                <span className="text-[10px] text-slate-300">|</span>
                <h1 className="text-[13px] font-bold text-slate-800">{map.meta.repo} · 语义代码地图</h1>
                {repos.length > 1 && (
                  <select
                    value={backendRepo ?? ''}
                    onChange={(e) => onRepoChange(e.target.value)}
                    className="rounded-full border border-slate-200 bg-white px-1.5 py-0.5 text-[10px] text-slate-500 outline-none hover:border-blue-300"
                    title="切换工作仓库（dev.sh 以逗号分隔挂载多个）"
                  >
                    {repos.map((r) => (
                      <option key={r.id} value={r.id}>
                        {r.name}
                      </option>
                    ))}
                  </select>
                )}
              </div>
              <button
                onClick={() => setHeaderExpanded((v) => !v)}
                className="mt-0.5 block max-w-[520px] text-left"
                title={headerExpanded ? '收起简介' : '展开简介'}
              >
                <p className={`text-[11px] text-slate-500 ${headerExpanded ? 'max-h-28 overflow-y-auto' : 'truncate'}`}>
                  {map.meta.description}
                  <span className="ml-1 text-[9.5px] font-medium text-blue-400">
                    {headerExpanded ? '▲ 收起' : '▼ 展开'}
                  </span>
                </p>
              </button>
              <div className="mt-1 flex items-center gap-3 text-[10.5px] text-slate-400">
                <span className="flex items-center gap-1">
                  <GitBranch size={11} /> {map.meta.generator}
                </span>
                <span>{mergedMap.modules.length} 模块</span>
                <span>{mergedMap.layers.length} 层</span>
                <span>{mergedMap.edges.length} 依赖</span>
                <span className="text-red-500">{violations} 逆向</span>
                {!backendRepo && (
                  <span
                    className="flex items-center gap-1 rounded-full bg-amber-100 px-1.5 py-px font-semibold text-amber-700"
                    title="后端不在线：当前为静态演示数据，重新归纳/巡检/任务不可用"
                  >
                    <WifiOff size={9} /> 演示数据 · 后端离线
                  </span>
                )}
                {freshness && (
                  <span
                    className={`flex items-center gap-1 rounded-full px-1.5 py-px font-semibold ${
                      freshness === 'stale' ? 'bg-red-100 text-red-700' : 'bg-amber-100 text-amber-700'
                    }`}
                    title={`git 有 ${freshnessInfo.commitsSinceMap ?? '?'} 个提交在地图生成之后——对话/建议/健康分可能基于过时信息`}
                  >
                    <AlertTriangle size={9} />
                    地图已过时 · {freshness === 'stale' ? '建议重新归纳' : `${freshnessInfo.commitsSinceMap ?? '?'} 个新提交未归纳`}
                    {freshness === 'stale' && backendRepo && (
                      <button onClick={startReinduce} className="ml-0.5 rounded-full bg-white/70 px-1 text-[9px] hover:bg-white">
                        重新归纳
                      </button>
                    )}
                  </span>
                )}
                {inducing && (
                  <span className="flex items-center gap-1 font-semibold text-amber-600">
                    <RefreshCw size={10} className="animate-spin" />
                    归纳中…
                  </span>
                )}
                <button
                  onClick={startGrowth}
                  disabled={!!growth}
                  className={`ml-1 flex items-center gap-1 rounded-full border px-2 py-0.5 font-semibold transition-colors disabled:opacity-40 ${
                    liveActivity && !growth
                      ? 'border-red-300 bg-red-50 text-red-600 animate-pulse'
                      : 'border-blue-200 bg-blue-50 text-blue-600 hover:bg-blue-100'
                  }`}
                  title={backendRepo ? '观看实时生长（直播 growth.log 事件）' : '回放归纳过程（静态 growth.log）'}
                >
                  <Play size={10} />
                  {liveActivity && !growth ? '归纳活动 · 观看生长' : '生长演示'}
                </button>
                {backendRepo && (
                  <button
                    onClick={() => {
                      setTab('suggest')
                      setPanelOpen(true)
                    }}
                    className="ml-1 flex items-center gap-1 rounded-full border border-amber-200 bg-amber-50 px-2 py-0.5 font-semibold text-amber-700 transition-colors hover:bg-amber-100"
                    title="AI 主动发现优化建议，逐条可发起修复"
                  >
                    <Lightbulb size={10} />
                    优化建议
                  </button>
                )}
                <button
                  onClick={() => setSettingsOpen((v) => !v)}
                  className="ml-1 rounded-full border border-slate-200 bg-white px-2 py-0.5 font-semibold text-slate-500 transition-colors hover:bg-slate-50"
                  title="设置（LLM 服务 / 槽位绑定 / 高级）"
                >
                  <Settings size={10} />
                </button>
                {backendRepo && (
                  <button
                    onClick={startPatrol}
                    disabled={patrolling}
                    className={`ml-1 flex items-center gap-1 rounded-full border px-2 py-0.5 font-semibold transition-colors disabled:opacity-60 ${
                      patrolling
                        ? 'border-amber-300 bg-amber-50 text-amber-700'
                        : 'border-emerald-200 bg-emerald-50 text-emerald-700 hover:bg-emerald-100'
                    }`}
                    title="巡检：Supervisor 直调 LLM（带健康基线），产出新地图并落健康历史"
                  >
                    <Activity size={10} className={patrolling ? 'animate-pulse' : ''} />
                    {patrolling ? '巡检中…' : '巡检'}
                  </button>
                )}
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* 设置面板（滑出） */}
      {settingsOpen && <SettingsPanel backendRepo={backendRepo} onClose={() => setSettingsOpen(false)} />}

      {/* 任务表单（指哪打哪：模块/问题/层入口预填） */}
      {taskDraft && (
        <TaskFormPanel
          backendRepo={backendRepo}
          draft={taskDraft}
          map={map}
          onClose={() => setTaskDraft(null)}
          onCreated={() => setTab('tasks')}
          onLocateModule={focusModule}
        />
      )}

      {/* 右侧详情面板 */}
      {panelOpen ? (
        <>
        {/* 改进#4：右栏宽度拖拽手柄 */}
        <div
          onMouseDown={startPanelDrag}
          className="w-1 shrink-0 cursor-col-resize bg-slate-100 transition-colors hover:bg-blue-300"
          title="拖拽调整面板宽度"
        />
        <DetailPanel
          map={map}
          selection={selection}
          tab={tab}
          onTabChange={setTab}
          submaps={submaps}
          backendRepo={backendRepo}
          onCreateTask={(d) => setTaskDraft(d)}
          onLocateModule={focusModule}
          onOpenView={openView}
          onClose={() => setPanelOpen(false)}
          width={panelWidth}
        />
        </>
      ) : (
        <button
          onClick={() => panelOpen === false && setPanelOpen(true)}
          className="flex w-9 shrink-0 flex-col items-center gap-2 border-l border-slate-200 bg-white py-4 text-slate-400 hover:text-blue-600"
          title="展开面板"
        >
          <PanelRightOpen size={15} />
          <span className="text-[10px] [writing-mode:vertical-rl]">{selection ? '详情' : '面板'}</span>
        </button>
      )}
    </div>
  )
}

// R1 兜底：渲染异常拦截，白屏换成可恢复提示
class CanvasBoundary extends Component<{ children: React.ReactNode }, { err: Error | null }> {
  state = { err: null as Error | null }
  static getDerivedStateFromError(err: Error) {
    return { err }
  }
  render() {
    if (this.state.err) {
      return (
        <div className="flex h-screen flex-col items-center justify-center gap-2 text-[13px] text-slate-500">
          <span className="font-semibold text-slate-700">画布渲染出错（已拦截白屏）</span>
          <span className="max-w-[420px] text-center text-slate-400">{String(this.state.err).slice(0, 200)}</span>
          <button className="rounded-lg border border-slate-200 px-3 py-1.5 text-blue-600 hover:bg-slate-50" onClick={() => location.reload()}>
            刷新恢复
          </button>
        </div>
      )
    }
    return this.props.children
  }
}

// 首归纳等待页：轮询 progress.json 展示真实阶段与百分比（"正在边推导 80%"而非干转圈）
function InductionWaiting({ repo }: { repo: string }) {
  const [prog, setProg] = useState<{ phase: string; percent: number; modulesDone: number; modulesTotal: number } | null>(null)
  useEffect(() => {
    let stale = false
    const tick = () => {
      fetch(`/api/repos/${repo}/progress`)
        .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
        .then((d: { data: { phase: string; percent: number; modules_done: number; modules_total: number } | null }) => {
          if (stale || !d.data) return
          setProg({ phase: d.data.phase, percent: d.data.percent, modulesDone: d.data.modules_done, modulesTotal: d.data.modules_total })
        })
        .catch(() => {})
    }
    tick()
    const t = window.setInterval(tick, 2000)
    return () => {
      stale = true
      window.clearInterval(t)
    }
  }, [repo])

  const PHASE_LABEL: Record<string, string> = {
    init: '准备中',
    layering: '分层分析',
    module_scan: '模块扫描',
    emit: '模块归纳',
    edging: '依赖边推导',
    consistency: '一致性校验',
    finalize: '收尾写盘',
    done: '完成',
  }
  return (
    <div className="flex h-screen flex-col items-center justify-center gap-3 text-[13px] text-slate-500">
      <Loader2 size={18} className="animate-spin text-blue-500" />
      <span className="font-semibold text-slate-700">
        正在归纳代码地图{prog ? `：${PHASE_LABEL[prog.phase] ?? prog.phase} ${prog.percent}%` : '…'}
      </span>
      {prog && (
        <div className="h-1.5 w-64 overflow-hidden rounded-full bg-slate-100">
          <div className="h-full rounded-full bg-blue-500 transition-all duration-700" style={{ width: `${prog.percent}%` }} />
        </div>
      )}
      <span className="max-w-[420px] text-center text-[11px] leading-4 text-slate-400">
        {prog && prog.modulesTotal > 0
          ? `已归纳 ${prog.modulesDone}/${prog.modulesTotal} 个模块`
          : '后台 agent 执行中（通常数分钟，取决于仓库规模）'}
        ，完成后地图会自动出现
      </span>
    </div>
  )
}

export default function App() {
  const [map, setMap] = useState<CodeMap | null>(null)
  const [error, setError] = useState<string | null>(null)
  // 后端模式：探测 /api/health 成功且仓库列表非空则启用；失败降级静态 demo 数据
  const [backendRepo, setBackendRepo] = useState<string | null>(null)
  const [repos, setRepos] = useState<{ id: string; name: string }[]>([])
  const [reloadTick, setReloadTick] = useState(0)

  useEffect(() => {
    let cancelled = false
    fetch('/api/health')
      .then((r) => (r.ok ? fetch('/api/repos') : Promise.reject(new Error('no backend'))))
      .then((r) => r.json())
      .then((d: { data?: { id: string; name: string }[] }) => {
        if (!cancelled && d.data && d.data.length > 0) {
          setRepos(d.data)
          setBackendRepo((cur) => cur ?? d.data![0].id) // 保留当前选择（切换器驱动）
        }
      })
      .catch(() => {
        if (!cancelled) setBackendRepo(null)
      })
    return () => {
      cancelled = true
    }
  }, [])

  // WS 订阅：map.changed 触发重取 / growth 与 session 事件转发；指数退避重连 + 重连后全量重同步
  useEffect(() => {
    if (!backendRepo) return
    let closed = false
    let ws: WebSocket | null = null
    let retry = 0
    let timer: number | undefined

    const connect = () => {
      if (closed) return
      const proto = location.protocol === 'https:' ? 'wss' : 'ws'
      ws = new WebSocket(`${proto}://${location.host}/ws`)
      ws.onopen = () => {
        retry = 0
        setReloadTick((t) => t + 1) // 全量重同步（覆盖断线期间的变更）
      }
      ws.onmessage = (e) => {
        try {
          const msg = JSON.parse(e.data)
          if (msg.name === 'map.changed' && msg.data?.repo === backendRepo) setReloadTick((t) => t + 1)
          if (msg.name === 'growth.event' && msg.data?.repo === backendRepo) emitGrowthEvent(msg.data.event)
          if (msg.name === 'session.statusChanged') {
            const d = msg.data
            if (d?.repo === backendRepo) emitSessionEvent({ repo: d.repo, sessionId: d.sessionId, status: d.status })
          }
          if (msg.name === 'session.output') {
            emitSessionOutput({ sessionId: msg.data.sessionId, line: msg.data.line })
          }
          if (msg.name === 'freshness.changed' && msg.data?.repo === backendRepo) {
            emitFreshnessEvent({
              repo: msg.data.repo,
              status: msg.data.status,
              latestCommitAt: msg.data.latestCommitAt,
              commitsSinceMap: msg.data.commitsSinceMap,
            })
          }
          if (msg.name === 'task.statusChanged') {
            const d = msg.data
            if (d?.repo === backendRepo) emitTaskEvent({ repo: d.repo, taskId: d.taskId, status: d.status, gate: d.gate })
          }
        } catch {
          /* 忽略坏消息 */
        }
      }
      ws.onclose = () => {
        if (closed) return
        notifyWsClosed() // R2：断线时通知 Canvas 退出生长模式（重连后重新进入会拉全量）
        retry += 1
        timer = window.setTimeout(connect, Math.min(15000, 1000 * 2 ** retry))
      }
    }
    connect()
    return () => {
      closed = true
      window.clearTimeout(timer)
      ws?.close()
    }
  }, [backendRepo])

  useEffect(() => {
    const url = backendRepo ? `/api/repos/${backendRepo}/map` : '/data/map.json'
    fetch(url)
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`)
        return r.json() as Promise<CodeMap>
      })
      .then((m) => {
        setError(null) // 实弹#4 前端根因：成功后必须清错误态，否则 (error && backendRepo) 恒真永远白屏等待
        setMap(m)
      })
      .catch((e) => setError(String(e)))
  }, [backendRepo, reloadTick])

  if (error && !backendRepo) {
    return (
      <div className="flex h-screen items-center justify-center text-[13px] text-red-500">
        静态数据加载失败（/data/map.json）：{error}
      </div>
    )
  }
  const switchRepo = (id: string) => {
    if (id === backendRepo) return
    setMap(null)
    setError(null)
    setBackendRepo(id)
  }

  if ((error && backendRepo) || (!map && backendRepo)) {
    // 后端在线但地图尚未生成：归纳进行中，map.changed 会触发自动重试
    return <InductionWaiting repo={backendRepo} />
  }
  if (!map) {
    return (
      <div className="flex h-screen items-center justify-center gap-2 text-[13px] text-slate-500">
        <Loader2 size={16} className="animate-spin" /> 正在加载代码地图…
      </div>
    )
  }
  return (
    <CanvasBoundary>
      <ReactFlowProvider>
        <Canvas map={map} backendRepo={backendRepo} repos={repos} onRepoChange={switchRepo} />
      </ReactFlowProvider>
    </CanvasBoundary>
  )
}
