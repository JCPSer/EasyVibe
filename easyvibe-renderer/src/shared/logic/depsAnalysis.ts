// 依赖关系（模块间耦合）分析层——DepsPage 与画布透镜共用的计算口径单一来源。
// 全部从 map.edges 现算（零 schema 变更）；「推导环」（本文件 Tarjan SCC）须与子模块级
// 声明式 circular_dep 区分，勿混用（2026-10-05 评审 D6）。
import type { CodeMap, MapEdge, Module } from '@/types/map'

export type EdgeLike = Pick<MapEdge, 'from' | 'to'>

/** Tarjan SCC：返回所有强连通分量（含单例），大群在前 */
export function tarjanSCC(nodeIds: string[], edges: EdgeLike[]): string[][] {
  const adj = new Map<string, string[]>()
  for (const id of nodeIds) adj.set(id, [])
  for (const e of edges) {
    if (adj.has(e.from) && adj.has(e.to)) adj.get(e.from)!.push(e.to)
  }
  const index = new Map<string, number>()
  const low = new Map<string, number>()
  const onStack = new Set<string>()
  const stack: string[] = []
  let counter = 0
  const result: string[][] = []
  const strongConnect = (v: string) => {
    index.set(v, counter)
    low.set(v, counter)
    counter += 1
    stack.push(v)
    onStack.add(v)
    for (const w of adj.get(v) ?? []) {
      if (!index.has(w)) {
        strongConnect(w)
        low.set(v, Math.min(low.get(v)!, low.get(w)!))
      } else if (onStack.has(w)) {
        low.set(v, Math.min(low.get(v)!, index.get(w)!))
      }
    }
    if (low.get(v) === index.get(v)) {
      const s: string[] = []
      for (;;) {
        const w = stack.pop()!
        onStack.delete(w)
        s.push(w)
        if (w === v) break
      }
      result.push(s)
    }
  }
  for (const id of nodeIds) if (!index.has(id)) strongConnect(id)
  return result.sort((a, b) => b.length - a.length)
}

export interface HighRiskGroup {
  target: Module
  edges: MapEdge[]
}

export interface CouplingAnalysis {
  violations: MapEdge[]
  sccGroups: string[][]
  cycleModuleIds: Set<string>
  highRisk: HighRiskGroup[]
  crossLayerSkips: MapEdge[]
  topToFoundation: MapEdge[]
  fanIn: Map<string, number>
  fanOut: Map<string, number>
  orderOf: (moduleId: string) => number
  layerNameOf: (layerId: string) => string
  layerNameByModule: (moduleId: string) => string
  moduleById: Map<string, Module>
}

const refCount = (e: MapEdge): number => {
  const m = /(\d+)/.exec(e.label ?? '')
  return m ? Number(m[1]) : 1
}

/** 全量耦合分析：每个指标一行算法，口径即验收依据（评审 B1） */
export function couplingAnalysis(map: CodeMap): CouplingAnalysis {
  const moduleById = new Map(map.modules.map((m) => [m.id, m]))
  const layerById = new Map(map.layers.map((l) => [l.id, l]))
  const orderOfLayer = new Map(map.layers.map((l) => [l.id, l.order]))
  const maxOrder = Math.max(...map.layers.map((l) => l.order))
  const orderOf = (moduleId: string): number => {
    const m = moduleById.get(moduleId)
    return m ? (orderOfLayer.get(m.layer) ?? 0) : 0
  }
  const layerNameOf = (layerId: string): string => layerById.get(layerId)?.name ?? layerId
  const layerNameByModule = (moduleId: string): string => {
    const m = moduleById.get(moduleId)
    return m ? layerNameOf(m.layer) : moduleId
  }

  const fanIn = new Map<string, number>()
  const fanOut = new Map<string, number>()
  for (const e of map.edges) {
    fanOut.set(e.from, (fanOut.get(e.from) ?? 0) + 1)
    fanIn.set(e.to, (fanIn.get(e.to) ?? 0) + 1)
  }

  const violations = map.edges
    .filter((e) => e.direction_violation === true)
    .sort((a, b) => (b.strength === 'strong' ? 1 : 0) - (a.strength === 'strong' ? 1 : 0) || refCount(b) - refCount(a))

  const sccs = tarjanSCC(
    map.modules.map((m) => m.id),
    map.edges,
  )
  const sccGroups = sccs.filter((g) => g.length > 1)
  const cycleModuleIds = new Set(sccGroups.flat())

  const highRiskByTarget = new Map<string, MapEdge[]>()
  for (const e of map.edges) {
    const t = moduleById.get(e.to)
    if (e.strength === 'strong' && t && t.health.score < 60) {
      if (!highRiskByTarget.has(e.to)) highRiskByTarget.set(e.to, [])
      highRiskByTarget.get(e.to)!.push(e)
    }
  }
  const highRisk = [...highRiskByTarget.entries()]
    .map(([id, edges]) => ({ target: moduleById.get(id)!, edges }))
    .filter((g) => g.target)
    .sort((a, b) => b.edges.length - a.edges.length)

  const crossLayerSkips = map.edges.filter((e) => Math.abs(orderOf(e.from) - orderOf(e.to)) >= 2)
  const topToFoundation = map.edges.filter(
    (e) => orderOf(e.to) === maxOrder && orderOf(e.from) <= 1 && Math.abs(orderOf(e.from) - orderOf(e.to)) >= 2,
  )

  return {
    violations, sccGroups, cycleModuleIds, highRisk, crossLayerSkips, topToFoundation,
    fanIn, fanOut, orderOf, layerNameOf, layerNameByModule, moduleById,
  }
}

export interface DepCard {
  id: string
  kind: 'violation' | 'cycle' | 'highRisk'
  severity: 'critical' | 'warning'
  title: string
  evidence: string
  consequence: string
  /** violation / highRisk 卡带边信息 */
  edge?: MapEdge
  targetId?: string
  sourceId?: string
  members?: string[]
}

const TYPE_LABEL: Record<MapEdge['type'], string> = {
  call: '函数调用', import: 'import', api: 'API 调用', event: '消息事件', db: '共享数据库', config: '配置依赖',
}

/** 文案翻译注入（英文化第二批）：shared 是 archGuard 叶子，不得 import runtime/i18n——
 *  由调用方（页面/组件）把 t() 注进来；缺省回退中文词表（存量测试与中文界面行为不变）。
 *  约定：tx 返回 key 本身视为缺 key，回退默认中文文案。 */
export type DepsTx = (key: string, vars?: Record<string, string | number>) => string

const txOr = (tx: DepsTx | undefined, key: string, vars: Record<string, string | number>, fallback: string): string => {
  if (!tx) return fallback
  const s = tx(key, vars)
  return s === key ? fallback : s
}

/** 结论卡片（人话结论 + 证据 + 后果 + 行动），默认视图唯一信息单元 */
export function buildDepCards(map: CodeMap, a: CouplingAnalysis, tx?: DepsTx): DepCard[] {
  const cards: DepCard[] = []
  const nameOf = (id: string) => a.moduleById.get(id)?.name ?? id
  const typeLabel = (type: MapEdge['type']) => txOr(tx, `deps.type.${type}`, {}, TYPE_LABEL[type])

  for (const e of a.violations) {
    cards.push({
      id: `vio-${e.id}`,
      kind: 'violation',
      severity: 'critical',
      title: txOr(tx, 'deps.card.violation.title', { from: nameOf(e.from), to: nameOf(e.to) }, `「${nameOf(e.from)}」反向调用了「${nameOf(e.to)}」`),
      evidence: txOr(
        tx,
        'deps.card.violation.evidence',
        { type: typeLabel(e.type), count: refCount(e), layerFrom: a.layerNameByModule(e.from), layerTo: a.layerNameByModule(e.to) },
        `${TYPE_LABEL[e.type]} · ${refCount(e)} 处引用 · 违反「${a.layerNameByModule(e.from)} → ${a.layerNameByModule(e.to)}」分层`,
      ),
      consequence: txOr(tx, 'deps.card.violation.consequence', { from: nameOf(e.from), to: nameOf(e.to) }, `改「${nameOf(e.to)}」时「${nameOf(e.from)}」被一起拖着改，下层无法独立替换、独立测试。`),
      edge: e, sourceId: e.from, targetId: e.to,
    })
  }

  const biggest = a.sccGroups[0]
  if (biggest) {
    const singles = map.modules.filter((m) => !a.cycleModuleIds.has(m.id))
    const singleNames = singles.slice(0, 2).map((m) => m.name).join('、')
    const singlesPart = singles.length > 0
      ? txOr(tx, 'deps.card.cycle.singles', { names: singleNames }, `，全仓仅「${singleNames}」独善其身`)
      : ''
    cards.push({
      id: 'cycle-0',
      kind: 'cycle',
      severity: 'warning',
      title: txOr(tx, 'deps.card.cycle.title', { count: biggest.length, singles: singlesPart }, `循环群：${biggest.length} 个模块互相可达${singles.length > 0 ? `，全仓仅「${singleNames}」独善其身` : ''}`),
      evidence: txOr(tx, 'deps.card.cycle.evidence', { count: a.violations.length }, `闭环由 ${a.violations.length} 条逆向依赖互相打通；直连跨度越大，环越难拆。`),
      consequence: txOr(tx, 'deps.card.cycle.consequence', {}, '发布与测试互相绑架，任何一环改动都可能波及全链。'),
      members: [...biggest].sort(),
    })
  }

  for (const g of a.highRisk) {
    const froms = g.edges.map((e) => nameOf(e.from))
    cards.push({
      id: `risk-${g.target.id}`,
      kind: 'highRisk',
      severity: 'warning',
      title: txOr(tx, 'deps.card.highRisk.title', { name: g.target.name, score: g.target.health.score, count: g.edges.length }, `「${g.target.name}」（${g.target.health.score} 分）被 ${g.edges.length} 条强耦合依赖——腐化在传染`),
      evidence: txOr(tx, 'deps.card.highRisk.evidence', { froms: froms.join(' / ') }, `strong 依赖来自：${froms.join(' / ')}`),
      consequence: txOr(tx, 'deps.card.highRisk.consequence', { count: froms.length }, `它的腐化会顺着强耦合传给 ${froms.length} 个调用方。`),
      targetId: g.target.id,
      edge: g.edges[0],
    })
  }
  return cards
}
