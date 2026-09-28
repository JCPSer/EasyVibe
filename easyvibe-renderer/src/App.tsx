import { useEffect, useMemo, useState, useCallback } from 'react'
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
import { Activity, AlertTriangle, GitBranch, Loader2, PanelRightOpen, UnfoldVertical, FoldVertical, RefreshCw, Focus, Play, Pause, RotateCcw, X, Sparkles } from 'lucide-react'

import type { CodeMap, SubMap } from '@/types/map'
import { layoutMap, healthColor, NODE_W, NODE_H, SUB_W, SUB_H } from '@/lib/layout'
import { ModuleNode, type ModuleNodeType } from '@/components/ModuleNode'
import { BandNode, type BandNodeType } from '@/components/BandNode'
import { ExpandedModuleNode, type ExpandedModuleNodeType } from '@/components/ExpandedModuleNode'
import { SubmoduleNode, type SubmoduleNodeType } from '@/components/SubmoduleNode'
import { DetailPanel, type Selection, type PanelTab } from '@/components/DetailPanel'
import { isIssueModule } from '@/components/IssuesList'

const nodeTypes = { module: ModuleNode, moduleExpanded: ExpandedModuleNode, submodule: SubmoduleNode, band: BandNode }

interface Filters {
  violationsOnly: boolean
  issuesOnly: boolean
  solo: boolean // 只看选中模块的依赖（强聚焦）
}

const MAX_EXPANDED = 3

// growth.log 事件（v2.2 协议）
type GrowthEvent =
  | { type: 'layer'; layer: CodeMap['layers'][number] }
  | { type: 'module'; module: CodeMap['modules'][number]; out_edges: CodeMap['edges'] }
  | { type: 'arch_health'; health: CodeMap['health'] }
  | { type: 'done' }

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
  expanded: Map<string, SubMap | 'loading'>,
  onSelectLayer: (id: string) => void,
  growth?: { layers: Set<string>; modules: Set<string> } | null,
) {
  // 展开元信息：加载中给 4 个骨架位
  const expandedMeta = new Map<string, { ids: string[]; loading: boolean }>()
  for (const [id, sm] of expanded) {
    expandedMeta.set(id, sm === 'loading' ? { ids: ['__s0', '__s1', '__s2', '__s3'], loading: true } : { ids: sm.sub_modules.map((s) => s.id), loading: false })
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
            subCount: sm === 'loading' ? 0 : sm.sub_modules.length,
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
      if (sm !== 'loading') {
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
          sub: sm === 'loading' ? undefined : sm.sub_modules.find((s) => s.id === sid),
          loading: sm === 'loading',
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
    const si = outIdx.get(e.from) ?? 0
    outIdx.set(e.from, si + 1)
    const ti = inIdx.get(e.to) ?? 0
    inIdx.set(e.to, ti + 1)
    const strengthStyle =
      e.strength === 'strong'
        ? { strokeWidth: 2.1, stroke: '#64748b', opacity: 0.85 }
        : e.strength === 'normal'
          ? { strokeWidth: 1.6, stroke: '#94a3b8', opacity: 0.7 }
          : { strokeWidth: 1.1, stroke: '#cbd5e1', opacity: 0.6 }
    const dim = edgeDim(e.from === selModuleId || e.to === selModuleId, violation)
    return {
      id: `e-${i}`,
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
    if (sm === 'loading') continue
    const sin = new Map<string, number>()
    const sout = new Map<string, number>()
    sm.edges.forEach((e, i) => {
      const cyclic = e.circular_dep === true
      const si = sout.get(e.from) ?? 0
      sout.set(e.from, si + 1)
      const ti = sin.get(e.to) ?? 0
      sin.set(e.to, ti + 1)
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
  hasSubmap,
  solo,
  onToggleExpand,
  onToggleSolo,
}: {
  moduleName: string
  expanded: boolean
  hasSubmap: boolean
  solo: boolean
  onToggleExpand: () => void
  onToggleSolo: () => void
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
        只看依赖
      </button>
      <button
        disabled
        title={hasSubmap ? '重新归纳需要 Supervisor（M2 提供）' : '子图归纳需要 Supervisor（M2 提供）'}
        className="flex cursor-not-allowed items-center gap-1 rounded-full border border-transparent px-3 py-1 text-[11px] font-semibold text-slate-300"
      >
        <RefreshCw size={12} />
        重新归纳
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
  const pct = Math.round((growth.index / total) * 100)
  const cur = growth.index < growth.events.length ? growth.events[growth.index] : null
  const status = growth.done
    ? '归纳完成'
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

function Canvas({ map }: { map: CodeMap }) {
  const [selection, setSelection] = useState<Selection>(null)
  const [panelOpen, setPanelOpen] = useState(true)
  const [tab, setTab] = useState<PanelTab>('issues')
  const [filters, setFilters] = useState<Filters>({ violationsOnly: false, issuesOnly: false, solo: false })
  const [expandedIds, setExpandedIds] = useState<string[]>([])
  const [submaps, setSubmaps] = useState<Record<string, SubMap | 'loading'>>({})
  const [growth, setGrowth] = useState<GrowthState | null>(null)
  const { fitView } = useReactFlow()

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

  // 事件推进定时器
  useEffect(() => {
    if (!growth?.playing) return
    const t = setInterval(() => {
      setGrowth((g) => {
        if (!g || !g.playing) return g
        const next = g.index + 1
        if (next >= g.events.length) return { ...g, index: g.events.length, playing: false, done: true }
        return { ...g, index: next }
      })
    }, 700)
    return () => clearInterval(t)
  }, [growth?.playing])

  const startGrowth = useCallback(() => {
    fetch('/data/growth.log')
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.text()
      })
      .then((text) => {
        const events = text.trim().split('\n').map((l) => JSON.parse(l) as GrowthEvent)
        setSelection(null)
        setExpandedIds([])
        setGrowth({ events, index: 0, playing: true, done: false })
      })
      .catch(() => setGrowth(null))
  }, [])

  const onSelectLayer = useCallback((id: string) => {
    setSelection({ kind: 'layer', id })
    setTab('detail')
    setPanelOpen(true)
  }, [])

  const toggleExpand = useCallback((id: string) => {
    setExpandedIds((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id].slice(-MAX_EXPANDED)))
    setSubmaps((prev) => (prev[id] ? prev : { ...prev, [id]: 'loading' }))
  }, [])

  // 懒加载子图：任何 loading 状态触发取数
  useEffect(() => {
    for (const [id, v] of Object.entries(submaps)) {
      if (v !== 'loading') continue
      fetch(`/data/modules/${id}.json`)
        .then((r) => {
          if (!r.ok) throw new Error(String(r.status))
          return r.json() as Promise<SubMap>
        })
        .then((d) => setSubmaps((p) => ({ ...p, [id]: d })))
        .catch(() => {
          setSubmaps((p) => {
            const { [id]: _dropped, ...rest } = p
            return rest
          })
          setExpandedIds((prev) => prev.filter((x) => x !== id))
        })
    }
  }, [submaps])

  const expanded = useMemo(() => {
    const m = new Map<string, SubMap | 'loading'>()
    for (const id of expandedIds) if (submaps[id]) m.set(id, submaps[id])
    return m
  }, [expandedIds, submaps])

  const emptyExpanded = useMemo(() => new Map<string, SubMap | 'loading'>(), [])
  const effectiveExpanded = growth ? emptyExpanded : expanded
  const growthVisible = growth ? arrived : null

  const { nodes, edges } = useMemo(
    () => buildFlow(map, selection, filters, effectiveExpanded, onSelectLayer, growthVisible),
    [map, selection, filters, effectiveExpanded, onSelectLayer, growthVisible],
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

  const violations = map.edges.filter((e) => e.direction_violation).length
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

          {/* 顶部中央：选中模块的横向工具栏（F1a） */}
          {selModule && (
            <Panel position="top-center" className="mt-1">
              <ModuleToolbar
                moduleName={selModule.name}
                expanded={expandedIds.includes(selModule.id)}
                hasSubmap={!!submaps[selModule.id] && submaps[selModule.id] !== 'loading'}
                solo={filters.solo}
                onToggleExpand={() => toggleExpand(selModule.id)}
                onToggleSolo={() => toggleFilter('solo')}
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
              </div>
              <p className="mt-0.5 max-w-[520px] truncate text-[11px] text-slate-500">{map.meta.description}</p>
              <div className="mt-1 flex items-center gap-3 text-[10.5px] text-slate-400">
                <span className="flex items-center gap-1">
                  <GitBranch size={11} /> {map.meta.generator}
                </span>
                <span>{map.modules.length} 模块</span>
                <span>{map.layers.length} 层</span>
                <span>{map.edges.length} 依赖</span>
                <span className="text-red-500">{violations} 逆向</span>
                <button
                  onClick={startGrowth}
                  disabled={!!growth}
                  className="ml-1 flex items-center gap-1 rounded-full border border-blue-200 bg-blue-50 px-2 py-0.5 font-semibold text-blue-600 transition-colors hover:bg-blue-100 disabled:opacity-40"
                  title="回放归纳过程（消费 growth.log，v2.2 协议）"
                >
                  <Play size={10} />
                  生长演示
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* 右侧详情面板 */}
      {panelOpen ? (
        <DetailPanel
          map={map}
          selection={selection}
          tab={tab}
          onTabChange={setTab}
          submaps={submaps}
          onLocateModule={(id) => {
            setSelection({ kind: 'module', id })
            setTab('detail')
          }}
          onClose={() => setPanelOpen(false)}
        />
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

export default function App() {
  const [map, setMap] = useState<CodeMap | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    fetch('/data/map.json')
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`)
        return r.json() as Promise<CodeMap>
      })
      .then(setMap)
      .catch((e) => setError(String(e)))
  }, [])

  if (error) {
    return (
      <div className="flex h-screen items-center justify-center text-[13px] text-red-500">
        地图数据加载失败（/data/map.json）：{error}
      </div>
    )
  }
  if (!map) {
    return (
      <div className="flex h-screen items-center justify-center gap-2 text-[13px] text-slate-500">
        <Loader2 size={16} className="animate-spin" /> 正在加载代码地图…
      </div>
    )
  }
  return (
    <ReactFlowProvider>
      <Canvas map={map} />
    </ReactFlowProvider>
  )
}
