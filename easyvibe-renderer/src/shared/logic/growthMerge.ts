// 生长事件纯函数：从 App.tsx 抽出以便测试（审查报告建议）
import type { CodeMap, GrowthEvent, MapEdge } from '@/types/map'

// R1：growth 事件最小形状校验——坏消息返回 false，调用方丢弃
export function isValidGrowthEvent(e: unknown): e is GrowthEvent {
  if (!e || typeof e !== 'object') return false
  const t = (e as Record<string, unknown>).type
  if (t === 'layer') return typeof (e as { layer?: { id?: unknown } }).layer?.id === 'string'
  if (t === 'module') return typeof (e as { module?: { id?: unknown } }).module?.id === 'string'
  if (t === 'arch_health') return !!(e as { health?: unknown }).health
  if (t === 'done') return true
  return false
}

// R3 修复：无 id 旧数据的合并键回退，防止边集合坍缩成一条
const edgeKey = (e: Pick<MapEdge, 'id' | 'from' | 'to' | 'type'>) => e.id ?? `${e.from}->${e.to}:${e.type}`

/** 把生长事件的层/模块/边合入基准地图（全量合并保布局稳定，显隐由 arrived 单独控制） */
export function mergeGrowthEvents(map: CodeMap, events: GrowthEvent[]): CodeMap {
  const layers = new Map(map.layers.map((l) => [l.id, l]))
  const modules = new Map(map.modules.map((m) => [m.id, m]))
  const edges = new Map(map.edges.map((e) => [edgeKey(e as MapEdge), e]))
  for (const e of events) {
    if (!isValidGrowthEvent(e)) continue
    if (e.type === 'layer') layers.set(e.layer.id, e.layer)
    if (e.type === 'module') {
      modules.set(e.module.id, e.module)
      for (const oe of e.out_edges ?? []) edges.set(edgeKey(oe), oe)
    }
  }
  return {
    ...map,
    layers: [...layers.values()],
    modules: [...modules.values()],
    edges: [...edges.values()],
  } as CodeMap
}

/** growth 负载解析：后端 JSON 数组或静态 NDJSON；坏行/坏消息一律跳过（Y1） */
export function parseGrowthText(text: string, isBackend: boolean): GrowthEvent[] {
  if (isBackend) {
    try {
      return (JSON.parse(text) as unknown[]).filter(isValidGrowthEvent)
    } catch {
      return []
    }
  }
  return text
    .trim()
    .split('\n')
    .flatMap((l) => {
      try {
        const e = JSON.parse(l) as unknown
        return isValidGrowthEvent(e) ? [e] : []
      } catch {
        return []
      }
    })
}
