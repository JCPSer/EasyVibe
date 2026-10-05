import { describe, expect, it } from 'vitest'
import { isValidGrowthEvent, mergeGrowthEvents, parseGrowthText } from '@/shared/logic/growthMerge'
import type { CodeMap, GrowthEvent } from '@/types/map'

const baseMap = {
  version: '1.0',
  meta: { repo: 'demo', generated_at: 't', generator: 'g/test' },
  layers: [{ id: 'l1', name: '层一', order: 0, description: '' }],
  modules: [
    {
      id: 'm1',
      name: 'M1',
      layer: 'l1',
      responsibility: 'r',
      files: ['src/**'],
      key_entries: [],
      dependencies: [],
      health: { score: 80, coupling: 'low', complexity: 'low', churn: 'low', decay_flags: [] },
    },
  ],
  edges: [{ id: 'e1', from: 'm1', to: 'm1', type: 'call', strength: 'weak' }],
  health: { score: 70, coupling: 'low', complexity: 'low', churn: 'low', decay_flags: [] },
} as unknown as CodeMap

const moduleEvent = (id: string, outEdges: unknown[] = []): GrowthEvent =>
  ({
    type: 'module',
    module: {
      id,
      name: id,
      layer: 'l1',
      responsibility: 'r',
      files: ['x/**'],
      key_entries: [],
      dependencies: [],
      health: { score: 60, coupling: 'high', complexity: 'high', churn: 'low', decay_flags: [] },
    },
    out_edges: outEdges,
  }) as GrowthEvent

describe('isValidGrowthEvent（R1）', () => {
  it('接受四种合法事件', () => {
    expect(isValidGrowthEvent({ type: 'layer', layer: { id: 'x' } })).toBe(true)
    expect(isValidGrowthEvent({ type: 'module', module: { id: 'x' }, out_edges: [] })).toBe(true)
    expect(isValidGrowthEvent({ type: 'arch_health', health: {} })).toBe(true)
    expect(isValidGrowthEvent({ type: 'done' })).toBe(true)
  })
  it('拒绝坏消息：null/缺字段/未知类型', () => {
    expect(isValidGrowthEvent(null)).toBe(false)
    expect(isValidGrowthEvent({})).toBe(false)
    expect(isValidGrowthEvent({ type: 'module' })).toBe(false)
    expect(isValidGrowthEvent({ type: 'mystery' })).toBe(false)
    expect(isValidGrowthEvent('string')).toBe(false)
  })
})

describe('mergeGrowthEvents', () => {
  it('合入新模块与出边，保留基础数据', () => {
    const merged = mergeGrowthEvents(baseMap, [
      { type: 'layer', layer: { id: 'l2', name: '层二', order: 1, description: '' } },
      moduleEvent('m2', [{ id: 'e2', from: 'm2', to: 'm1', type: 'call', strength: 'strong' }]),
      { type: 'done' },
    ])
    expect(merged.modules.map((m) => m.id).sort()).toEqual(['m1', 'm2'])
    expect(merged.layers.map((l) => l.id).sort()).toEqual(['l1', 'l2'])
    expect(merged.edges.map((e) => e.id).sort()).toEqual(['e1', 'e2'])
  })
  it('同 id 覆盖（去重）', () => {
    const merged = mergeGrowthEvents(baseMap, [moduleEvent('m1')])
    expect(merged.modules).toHaveLength(1)
    expect(merged.modules[0].name).toBe('m1')
  })
  it('R3 回归：无 id 旧数据的边不坍缩', () => {
    const legacy = {
      ...baseMap,
      edges: [
        { from: 'm1', to: 'm1', type: 'call', strength: 'weak' },
        { from: 'm1', to: 'm1', type: 'import', strength: 'strong' },
      ],
    } as unknown as CodeMap
    const merged = mergeGrowthEvents(legacy, [])
    expect(merged.edges).toHaveLength(2) // 两条不同 type 的边，靠 from->to 回退键区分
  })
  it('R1 回归：坏消息事件被跳过不抛异常', () => {
    const merged = mergeGrowthEvents(baseMap, [null, { type: 'module' }, moduleEvent('mx'), 42] as unknown as GrowthEvent[])
    expect(merged.modules.map((m) => m.id)).toContain('mx')
  })
})

describe('parseGrowthText（Y1）', () => {
  it('NDJSON 容忍坏行', () => {
    const text = '{"type":"done"}\nnot-json\n{"type":"layer","layer":{"id":"l9"}}\n'
    const events = parseGrowthText(text, false)
    expect(events).toHaveLength(2)
  })
  it('后端 JSON 数组容忍坏消息', () => {
    const events = parseGrowthText(JSON.stringify([{ type: 'done' }, { bad: 1 }, null]), true)
    expect(events).toHaveLength(1)
  })
  it('完全坏输入返回空数组而不是抛异常', () => {
    expect(parseGrowthText('garbage', true)).toEqual([])
    expect(parseGrowthText('', false)).toEqual([])
  })
})
