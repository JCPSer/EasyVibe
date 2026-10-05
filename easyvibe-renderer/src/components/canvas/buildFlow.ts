import { MarkerType, Position, type Edge, type Node } from '@xyflow/react'
import type { CodeMap, SubMap } from '@/types/map'
import { layoutMap, NODE_W, NODE_H, SUB_W, SUB_H } from '@/shared/logic/layout'
import type { ModuleNodeType } from '@/components/ModuleNode'
import type { BandNodeType } from '@/components/BandNode'
import type { ExpandedModuleNodeType } from '@/components/ExpandedModuleNode'
import type { SubmoduleNodeType } from '@/components/SubmoduleNode'
import type { Selection } from '@/shared/contract/selection'
import { isIssueModule } from '@/components/IssuesList'
import { couplingAnalysis } from '@/shared/logic/depsAnalysis'
import type { Filters } from './types'

export function buildFlow(
  map: CodeMap,
  selection: Selection,
  filters: Filters,
  expanded: Map<string, SubMap | 'loading' | 'error'>,
  onSelectLayer: (id: string) => void,
  retrySubmap?: (id: string) => void,
  growth?: { layers: Set<string>; modules: Set<string> } | null,
  analyzeSubmap?: (id: string) => void,
  agentLinesFor?: (id: string) => string[] | undefined,
  analyzeErrorFor?: (id: string) => string | undefined,
  onCollapse?: (id: string) => void,
  onOpenRuns?: (sessionId: string) => void,
  submapSessionIdFor?: (id: string) => string | undefined,
) {
  // 展开元信息：加载中给 4 个骨架位；错误态不再给骨架（M4-1 诚实三态——此前 error 也渲染"分析中…"骨架，
  // 造成"头部报错 + 身体永远转圈"的撕裂画面）
  const expandedMeta = new Map<string, { ids: string[]; loading: boolean }>()
  for (const [id, sm] of expanded) {
    expandedMeta.set(id, sm === 'loading' ? { ids: ['__s0', '__s1', '__s2', '__s3'], loading: true } : sm === 'error' ? { ids: [], loading: false } : { ids: sm.sub_modules.map((s) => s.id), loading: false })
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
            analyzeError: analyzeErrorFor?.(mod.id),
            onCollapse: onCollapse ? () => onCollapse(mod.id) : undefined,
            analyzeSessionId: submapSessionIdFor?.(mod.id),
            onOpenRuns: onOpenRuns ? (sid: string) => onOpenRuns(sid) : undefined,
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
  // v2 依赖透镜：solo 聚焦态下邻域边按「相对层序方向」着色（口径单一来源 depsAnalysis；
  // 通道裁决 2026-10-05 评审 S1——色相=方向，strength 只留线宽差）
  const lens = filters.solo && selModuleId ? couplingAnalysis(map) : null
  const edges: Edge[] = visibleEdges.map((e, i) => {
    const violation = e.direction_violation === true
    const si = Math.min(outIdx.get(e.from) ?? 0, 17) // 与节点 MAX_HANDLES=18 对齐，超出复用最后一个 handle
    outIdx.set(e.from, (outIdx.get(e.from) ?? 0) + 1)
    const ti = Math.min(inIdx.get(e.to) ?? 0, 17)
    inIdx.set(e.to, (inIdx.get(e.to) ?? 0) + 1)
    const touches = e.from === selModuleId || e.to === selModuleId
    if (lens && selModuleId && touches) {
      const fo = lens.orderOf(e.from)
      const toOrder = lens.orderOf(e.to)
      // 违规红 > 环成员琥珀 > 同层蓝 > 顺层灰
      const stroke = violation ? '#ef4444' : lens.cycleModuleIds.has(e.from) && lens.cycleModuleIds.has(e.to) ? '#d97706' : fo === toOrder ? '#3b82f6' : '#64748b'
      const width = e.strength === 'strong' ? 2.5 : e.strength === 'normal' ? 1.8 : 1.2
      return {
        id: `e-${e.id ?? i}`,
        source: e.from,
        target: e.to,
        sourceHandle: `s${si}`,
        targetHandle: `t${ti}`,
        type: 'default',
        style: violation
          ? { strokeWidth: width, stroke, strokeDasharray: '6 4', opacity: 0.95 }
          : { strokeWidth: width, stroke, opacity: 0.9 },
        animated: violation,
        label: violation ? '⚠' : undefined,
        labelStyle: { fill: '#ef4444', fontSize: 13, fontWeight: 700 },
        labelBgStyle: { fill: 'rgba(255,255,255,0.9)', fillOpacity: 0.9 },
        markerEnd: { type: MarkerType.ArrowClosed, width: 14, height: 14, color: stroke },
        interactionWidth: 20,
      } satisfies Edge
    }
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
      interactionWidth: 20,
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
