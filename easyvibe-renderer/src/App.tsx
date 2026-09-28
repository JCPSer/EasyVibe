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
import { Activity, AlertTriangle, GitBranch, Loader2, PanelRightClose, PanelRightOpen } from 'lucide-react'

import type { CodeMap } from '@/types/map'
import { layoutMap, healthColor, NODE_W, NODE_H } from '@/lib/layout'
import { ModuleNode, type ModuleNodeType } from '@/components/ModuleNode'
import { BandNode, type BandNodeType } from '@/components/BandNode'
import { DetailPanel, type Selection, type PanelTab } from '@/components/DetailPanel'
import { isIssueModule } from '@/components/IssuesList'

const nodeTypes = { module: ModuleNode, band: BandNode }

interface Filters {
  violationsOnly: boolean
  issuesOnly: boolean
}

function buildFlow(
  map: CodeMap,
  selection: Selection,
  filters: Filters,
  onSelectLayer: (id: string) => void,
) {
  const { positions, bands } = layoutMap(map)

  const inCount = new Map<string, number>()
  const outCount = new Map<string, number>()
  for (const e of map.edges) {
    outCount.set(e.from, (outCount.get(e.from) ?? 0) + 1)
    inCount.set(e.to, (inCount.get(e.to) ?? 0) + 1)
  }

  const layerOf = new Map(map.modules.map((m) => [m.id, m.layer]))

  // 选中模块的邻域：自身 + 直接依赖/被依赖
  const neighborhood = new Set<string>()
  if (selection?.kind === 'module') {
    neighborhood.add(selection.id)
    for (const e of map.edges) {
      if (e.from === selection.id) neighborhood.add(e.to)
      if (e.to === selection.id) neighborhood.add(e.from)
    }
  }
  const issueIds = new Set(map.modules.filter(isIssueModule).map((m) => m.id))

  const nodeDim = (id: string) => {
    let op = 1
    if (selection?.kind === 'module' && !neighborhood.has(id)) op *= 0.3
    if (filters.issuesOnly && !issueIds.has(id)) op *= 0.22
    return op
  }

  const nodes: Node[] = [
    ...bands.map((box, i): BandNodeType => {
      const layer = map.layers.find((l) => l.id === box.layerId)!
      const mods = map.modules.filter((m) => m.layer === layer.id)
      const avgScore = Math.round(mods.reduce((s, m) => s + m.health.score, 0) / Math.max(mods.length, 1))
      const modIds = new Set(mods.map((m) => m.id))
      const violations = map.edges.filter(
        (e) => e.direction_violation && (modIds.has(e.from) || modIds.has(e.to)),
      ).length
      return {
        id: `band-${box.layerId}`,
        type: 'band',
        position: { x: box.x, y: box.y },
        data: {
          layer,
          box,
          index: i,
          stats: { count: mods.length, avgScore, violations },
          selected: selection?.kind === 'layer' && selection.id === layer.id,
          onSelect: onSelectLayer,
        },
        draggable: false,
        selectable: false,
        zIndex: -1,
        width: box.width,
        height: box.height,
      }
    }),
    ...map.modules.map((mod): ModuleNodeType => ({
      id: mod.id,
      type: 'module',
      position: positions.get(mod.id)!,
      data: { module: mod, inCount: inCount.get(mod.id) ?? 0, outCount: outCount.get(mod.id) ?? 0 },
      sourcePosition: Position.Bottom,
      targetPosition: Position.Top,
      width: NODE_W,
      height: NODE_H,
      style: { opacity: nodeDim(mod.id) },
    })),
  ]

  const inIdx = new Map<string, number>()
  const outIdx = new Map<string, number>()

  const edges: Edge[] = map.edges.map((e, i) => {
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

    // 过滤与聚焦：淡化（不隐藏）不满足条件的连线
    let dim = 1
    if (selection?.kind === 'module' && e.from !== selection.id && e.to !== selection.id) dim *= 0.07
    if (filters.violationsOnly && !violation) dim *= 0.06
    if (filters.issuesOnly && !issueIds.has(e.from) && !issueIds.has(e.to)) dim *= 0.06

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

  void layerOf
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
          <AlertTriangle size={11} className="text-red-500" /> Architecture violation（{violations}）
        </span>
      </div>
      <div className="flex items-center gap-2 border-t border-slate-100 pt-1.5 text-slate-400">
        <span className="inline-block h-2.5 w-2.5 rounded-full border border-slate-300 bg-white" /> 层健康 = 成员模块聚合
      </div>
    </div>
  )
}

function FilterButton({
  active,
  onClick,
  label,
  activeClass,
}: {
  active: boolean
  onClick: () => void
  label: string
  activeClass: string
}) {
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

function Canvas({ map }: { map: CodeMap }) {
  const [selection, setSelection] = useState<Selection>(null)
  const [panelOpen, setPanelOpen] = useState(true)
  const [tab, setTab] = useState<PanelTab>('issues')
  const [filters, setFilters] = useState<Filters>({ violationsOnly: false, issuesOnly: false })
  const { fitView } = useReactFlow()

  const onSelectLayer = useCallback((id: string) => {
    setSelection({ kind: 'layer', id })
    setTab('detail')
    setPanelOpen(true)
  }, [])

  const { nodes, edges } = useMemo(
    () => buildFlow(map, selection, filters, onSelectLayer),
    [map, selection, filters, onSelectLayer],
  )

  useEffect(() => {
    const t = setTimeout(() => fitView({ padding: 0.12, duration: 300 }), 60)
    return () => clearTimeout(t)
  }, [fitView, map])

  const onNodeClick = useCallback((_e: unknown, node: Node) => {
    if (node.type === 'module') {
      setSelection({ kind: 'module', id: node.id })
      setTab('detail')
      setPanelOpen(true)
    }
  }, [])
  const onPaneClick = useCallback(() => setSelection(null), [])

  const toggleFilter = (key: keyof Filters) => setFilters((f) => ({ ...f, [key]: !f[key] }))

  const violations = map.edges.filter((e) => e.direction_violation).length

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
              n.type === 'module'
                ? healthColor((n.data as { module: { health: { score: number } } }).module.health.score)
                : 'rgba(0,0,0,0)'
            }
            maskColor="rgba(226,232,240,0.7)"
            style={{ width: 200, height: 130 }}
          />

          {/* 右上角：架构健康 + 图例 同栏纵向堆叠，永不重叠 */}
          <Panel position="top-right" className="flex flex-col gap-2">
            <ArchHealthCard map={map} />
            <Legend violations={violations} />
          </Panel>

          {/* 底部中央：全局过滤开关 */}
          <Panel position="bottom-center" className="mb-2">
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
                  onClick={() => setFilters({ violationsOnly: false, issuesOnly: false })}
                  className="rounded-full px-2 py-1 text-[10.5px] text-slate-400 hover:text-slate-600"
                >
                  重置
                </button>
              )}
            </div>
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
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* 右侧详情面板：可折叠，收起后留细栏可随时 reopen */}
      {panelOpen ? (
        <DetailPanel
          map={map}
          selection={selection}
          tab={tab}
          onTabChange={setTab}
          onLocateModule={(id) => {
            setSelection({ kind: 'module', id })
            setTab('detail')
          }}
          onClose={() => setPanelOpen(false)}
        />
      ) : (
        <button
          onClick={() => selection && setPanelOpen(true)}
          disabled={!selection}
          className="flex w-9 shrink-0 flex-col items-center gap-2 border-l border-slate-200 bg-white py-4 text-slate-400 hover:text-blue-600 disabled:cursor-default disabled:opacity-40"
          title={selection ? '展开详情面板' : '点击画布中的模块或层标签查看详情'}
        >
          {panelOpen ? <PanelRightClose size={15} /> : <PanelRightOpen size={15} />}
          <span className="text-[10px] [writing-mode:vertical-rl]">
            {selection ? '详情' : '未选中'}
          </span>
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
