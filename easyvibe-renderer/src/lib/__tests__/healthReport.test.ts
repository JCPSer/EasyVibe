import { describe, expect, it } from 'vitest'
import { buildHealthReport } from '@/lib/healthReport'
import type { CodeMap } from '@/types/map'

const demoMap = {
  version: '1.0',
  meta: {
    repo: 'demo',
    generated_at: '2026-10-01T00:00:00Z',
    last_patrol_at: '2026-10-01T01:00:00Z',
    stats: { files_total: 100, files_covered: 90, coverage_ratio: 0.9 },
  },
  layers: [
    { id: 'l1', name: '应用层', order: 0, description: '' },
    { id: 'l2', name: '基础层', order: 1, description: '' },
  ],
  modules: [
    {
      id: 'm1',
      name: '核心',
      layer: 'l1',
      responsibility: 'r',
      files: ['src/a/**'],
      key_entries: [],
      dependencies: ['m2'],
      health: {
        score: 45,
        coupling: 'high',
        complexity: 'high',
        decay_flags: ['god_module'],
        concerns: [{ severity: 'critical', finding: '职责过载', suggestion: '拆分' }],
      },
    },
    {
      id: 'm2',
      name: '工具',
      layer: 'l2',
      responsibility: 'r',
      files: ['src/b/**'],
      key_entries: [],
      dependencies: [],
      health: { score: 90, coupling: 'low', complexity: 'low', decay_flags: [] },
    },
  ],
  edges: [
    { id: 'e1', from: 'm2', to: 'm1', type: 'import', strength: 'strong', direction_violation: true },
  ],
  health: {
    score: 58,
    coupling: 'high',
    complexity: 'medium',
    decay_flags: ['layer_violation'],
    review_note: '分层被多处打破',
    concerns: [{ severity: 'high', finding: '逆向依赖密集', suggestion: '引入接口层' }],
  },
} as unknown as CodeMap

describe('buildHealthReport', () => {
  it('包含架构健康、模块表、问题与逆向依赖章节', () => {
    const md = buildHealthReport(demoMap)
    expect(md).toContain('# 架构健康报告 · demo')
    expect(md).toContain('综合健康分：58')
    expect(md).toContain('逆向依赖密集')
    // 模块按分数升序：核心(45) 在 工具(90) 之前
    expect(md.indexOf('| 核心 |')).toBeLessThan(md.indexOf('| 工具 |'))
    expect(md).toContain('职责过载')
    expect(md).toContain('## 逆向依赖（分层违规信号）')
    expect(md).toContain('工具 → 核心')
    expect(md).toContain('90%')
  })

  it('无 concerns / 无违规边时不产生空章节', () => {
    const clean = {
      ...demoMap,
      health: { score: 95, coupling: 'low', complexity: 'low', decay_flags: [] },
      edges: [],
      modules: demoMap.modules.map((m) => ({ ...m, health: { score: 90, coupling: 'low', complexity: 'low', decay_flags: [] } })),
    } as unknown as CodeMap
    const md = buildHealthReport(clean)
    expect(md).not.toContain('## 模块级问题与建议')
    expect(md).not.toContain('## 逆向依赖')
  })
})
