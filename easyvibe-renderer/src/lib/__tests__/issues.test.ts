import { describe, expect, it } from 'vitest'
import { collectIssues } from '@/components/IssuesList'
import type { CodeMap } from '@/types/map'

const mkModule = (id: string, score: number, flags: string[], deps: string[] = []) =>
  ({
    id,
    name: id,
    layer: 'l1',
    responsibility: 'r',
    files: ['x/**'],
    key_entries: [],
    dependencies: deps,
    health: { score, coupling: 'medium', complexity: 'medium', churn: 'low', decay_flags: flags, review_note: 'note' },
  }) as unknown as CodeMap['modules'][number]

const mkMap = (modules: CodeMap['modules'], archFlags: string[] = []): CodeMap =>
  ({
    version: '1.0',
    meta: { repo: 'demo', generated_at: 't', generator: 'g' },
    layers: [{ id: 'l1', name: 'L', order: 0, description: '' }],
    modules,
    edges: modules.flatMap((m) => m.dependencies.map((d) => ({ id: `e-${m.id}-${d}`, from: m.id, to: d, type: 'call', strength: 'weak' }))),
    health: { score: 55, coupling: 'high', complexity: 'high', churn: 'high', decay_flags: archFlags, review_note: 'arch note' },
  }) as unknown as CodeMap

describe('collectIssues 排序', () => {
  it('concerns 优先于 decay_flags 兜底', () => {
    const m1 = mkModule('m1', 70, ['x'])
    ;(m1.health as { concerns?: unknown[] }).concerns = [{ severity: 'high', finding: 'f', suggestion: 's' }]
    const issues = collectIssues(mkMap([m1]))
    // 有 concerns 的模块不应再产生兜底条目
    expect(issues.filter((i) => i.moduleId === 'm1')).toHaveLength(1)
    expect(issues[0].finding).toBe('f')
  })

  it('critical 先于 high；同级按影响面（被依赖数）降序', () => {
    const a = mkModule('a', 70, ['flag']) // 被依赖 2 次
    const b = mkModule('b', 70, ['flag']) // 被依赖 1 次
    const c = mkModule('c', 70, ['flag'])
    const d = mkModule('d', 70, ['flag'])
    const m = mkMap([a, b, c, d])
    // a 被 b、c 依赖；b 被 d 依赖
    m.edges.push(
      { id: 'x1', from: 'b', to: 'a', type: 'call', strength: 'weak' },
      { id: 'x2', from: 'c', to: 'a', type: 'call', strength: 'weak' },
      { id: 'x3', from: 'd', to: 'b', type: 'call', strength: 'weak' },
    )
    ;(m.modules[0].health as { concerns?: unknown[] }).concerns = [{ severity: 'critical', finding: 'crit', suggestion: 's' }]
    const issues = collectIssues(m)
    expect(issues[0].severity).toBe('critical')
    const highs = issues.filter((i) => i.severity === 'high')
    expect(highs[0].moduleId).toBe('b') // 影响面 1 > c/d 的 0（a 走 concerns 通道不占兜底）
    expect(highs[0].impact).toBeGreaterThanOrEqual(highs[1].impact)
  })

  it('架构级 decay_flags 产生 critical 兜底条目', () => {
    const issues = collectIssues(mkMap([mkModule('m1', 80, [])], ['layer_violation']))
    expect(issues.some((i) => i.scope === 'arch' && i.severity === 'critical')).toBe(true)
  })

  it('干净地图返回空数组', () => {
    expect(collectIssues(mkMap([mkModule('m1', 90, [])]))).toEqual([])
  })
})
