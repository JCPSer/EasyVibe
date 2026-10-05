// depsAnalysis 口径回归：算法与数字必须可复算（评审 B1 验收红线）
import { describe, it, expect } from 'vitest'
import type { CodeMap } from '@/types/map'
import { tarjanSCC, couplingAnalysis, buildDepCards } from '@/shared/logic/depsAnalysis'

// 三层结构：a(L0 顶层) → b(L1) → c(L2)；另有 d 单例；
// 环：m1 ↔ m2（同层 L1）；违规：c → a（底层调顶层）；高风险：a -strong-> h（h 55 分）
function fixture(): CodeMap {
  const mk = (id: string, layer: string, score: number) => ({
    id, name: id.toUpperCase(), layer, responsibility: '', files: ['x.ts'] as [string, ...string[]], key_entries: [], dependencies: [], health: { score, coupling: 'low' as const, complexity: 'low' as const, decay_flags: [] },
  })
  return {
    version: '1.0',
    meta: { repo: 't', generated_at: '', generator: 'test' },
    layers: [
      { id: 'L0', name: '顶层', order: 0, description: '' },
      { id: 'L1', name: '中层', order: 1, description: '' },
      { id: 'L2', name: '底层', order: 2, description: '' },
    ],
    modules: [mk('a', 'L0', 90), mk('b', 'L1', 80), mk('c', 'L2', 70), mk('d', 'L1', 85), mk('m1', 'L1', 75), mk('m2', 'L1', 75), mk('h', 'L1', 55)],
    edges: [
      { id: 'e1', from: 'a', to: 'b', type: 'import', strength: 'normal', label: '3 imports' },
      { id: 'e2', from: 'b', to: 'c', type: 'import', strength: 'normal' },
      { id: 'e3', from: 'd', to: 'a', type: 'import', strength: 'weak', direction_violation: true, label: '2 imports' },
      { id: 'e4', from: 'm1', to: 'm2', type: 'call', strength: 'strong' },
      { id: 'e5', from: 'm2', to: 'm1', type: 'call', strength: 'strong' },
      { id: 'e6', from: 'a', to: 'h', type: 'import', strength: 'strong', label: '9 imports' },
      { id: 'e7', from: 'd', to: 'c', type: 'import', strength: 'weak' },
      { id: 'e8', from: 'a', to: 'c', type: 'api', strength: 'normal' }, // L0→L2 跨层直达
    ],
    health: { score: 80, coupling: 'medium', complexity: 'low', decay_flags: [] },
  }
}

describe('tarjanSCC', () => {
  it('检出双向环与单例', () => {
    const map = fixture()
    const sccs = tarjanSCC(map.modules.map((m) => m.id), map.edges)
    const groups = sccs.filter((g) => g.length > 1)
    expect(groups).toHaveLength(1)
    expect([...groups[0]].sort()).toEqual(['m1', 'm2'])
  })
  it('空边全单例', () => {
    expect(tarjanSCC(['x', 'y'], [])).toHaveLength(2)
  })
})

describe('couplingAnalysis', () => {
  const a = couplingAnalysis(fixture())
  it('逆向违规计数与排序', () => {
    expect(a.violations).toHaveLength(1)
    expect(a.violations[0].id).toBe('e3')
  })
  it('高风险耦合 = strong 且目标 <60', () => {
    expect(a.highRisk).toHaveLength(1)
    expect(a.highRisk[0].target.id).toBe('h')
    expect(a.highRisk[0].edges.map((e) => e.id)).toEqual(['e6'])
  })
  it('跨层直达 = 顶层(L0/L1)直连最大层序层', () => {
    expect(a.topToFoundation.map((e) => e.id)).toEqual(['e8']) // a→c L0→L2
    expect(a.crossLayerSkips.map((e) => e.id)).toEqual(['e8'])
  })
  it('扇入扇出', () => {
    expect(a.fanIn.get('c')).toBe(3) // b→c, d→c, a→c(跨层直达)
    expect(a.fanOut.get('a')).toBe(3) // a→b, a→h, a→c
  })
  it('层序查找', () => {
    expect(a.orderOf('a')).toBe(0)
    expect(a.orderOf('c')).toBe(2)
    expect(a.layerNameByModule('b')).toBe('中层')
  })
})

describe('buildDepCards', () => {
  const map = fixture()
  const cards = buildDepCards(map, couplingAnalysis(map))
  it('卡片构成：1 违规 + 1 循环 + 1 高风险', () => {
    expect(cards.filter((c) => c.kind === 'violation')).toHaveLength(1)
    expect(cards.filter((c) => c.kind === 'cycle')).toHaveLength(1)
    expect(cards.filter((c) => c.kind === 'highRisk')).toHaveLength(1)
  })
  it('违规卡三段式与引用计数', () => {
    const v = cards.find((c) => c.kind === 'violation')!
    expect(v.title).toContain('反向调用了')
    expect(v.evidence).toContain('2 处引用')
    expect(v.evidence).toContain('中层 → 顶层')
    expect(v.severity).toBe('critical')
  })
  it('循环卡带成员清单，措辞不含「伪」', () => {
    const c = cards.find((x) => x.kind === 'cycle')!
    expect([...c.members!].sort()).toEqual(['m1', 'm2'])
    expect(c.title).not.toContain('伪')
    expect(c.title).toContain('2 个模块互相可达')
  })
  it('高风险卡后果叙事带调用方数', () => {
    const r = cards.find((c) => c.kind === 'highRisk')!
    expect(r.title).toContain('（55 分）')
    expect(r.consequence).toContain('1 个调用方')
  })
})
