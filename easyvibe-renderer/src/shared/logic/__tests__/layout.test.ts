import { describe, it, expect } from 'vitest'
import { layoutMap, NODE_W } from '../layout'
import type { CodeMap, Module } from '@/types/map'

// 造 n 个同层模块的地图
const makeMap = (n: number): CodeMap => {
  const modules = Array.from({ length: n }, (_, i): Module => ({
    id: `m${i}`,
    name: `模块${i}`,
    layer: 'L1',
    responsibility: '',
    files: [`src/m${i}.ts`],
    key_entries: [],
    dependencies: [],
    health: { score: 80, coupling: 'low', complexity: 'low', decay_flags: [] },
  }))
  return {
    version: '1.0',
    meta: { repo: 'test', generated_at: '2026-10-07T00:00:00Z', generator: 'test' },
    layers: [{ id: 'L1', name: '层', order: 1, description: '' }],
    modules: modules as [Module, ...Module[]],
    edges: [],
    health: { score: 80, coupling: 'low', complexity: 'low', decay_flags: [] },
  }
}

// 提取某层各行（按 y 分组且保持行内 x 序）
const rowsOf = (map: CodeMap) => {
  const { positions } = layoutMap(map)
  const byY = new Map<number, string[]>()
  for (const [id, p] of positions) {
    const row = byY.get(p.y) ?? []
    row.push(id)
    byY.set(p.y, row)
  }
  return [...byY.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(([, ids]) => ids.sort((a, b) => positions.get(a)!.x - positions.get(b)!.x))
}

describe('layoutMap 普通模块分行', () => {
  it('≤5 个模块单行', () => {
    expect(rowsOf(makeMap(5))).toHaveLength(1)
  })

  it('6 个模块均衡为 3+3 两行', () => {
    const rows = rowsOf(makeMap(6))
    expect(rows.map((r) => r.length)).toEqual([3, 3])
  })

  it('7 个模块为 4+3，8 个为 4+4', () => {
    expect(rowsOf(makeMap(7)).map((r) => r.length)).toEqual([4, 3])
    expect(rowsOf(makeMap(8)).map((r) => r.length)).toEqual([4, 4])
  })

  it('11 个模块三行 4+4+3，12 个 4+4+4', () => {
    expect(rowsOf(makeMap(11)).map((r) => r.length)).toEqual([4, 4, 3])
    expect(rowsOf(makeMap(12)).map((r) => r.length)).toEqual([4, 4, 4])
  })

  it('每行不超过 5 个，且行内等距排列', () => {
    for (const n of [6, 7, 9, 10, 13, 16]) {
      const { positions } = layoutMap(makeMap(n))
      const rows = rowsOf(makeMap(n))
      for (const row of rows) {
        expect(row.length).toBeLessThanOrEqual(5)
        for (let i = 1; i < row.length; i++) {
          expect(positions.get(row[i])!.x - positions.get(row[i - 1])!.x).toBe(NODE_W + 44) // GAP_X
        }
      }
    }
  })
})
