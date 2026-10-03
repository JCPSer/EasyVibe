import { describe, expect, it } from 'vitest'
import { stageOf } from '../taskStage'

// 五阶段映射（方案 v3 §4.1 验收 #6）：全 (status, gate) 组合覆盖。
// 核心纪律：status 优先——failed 残留 gate 值不得制造假阶段（复审意见）。

describe('stageOf', () => {
  it('计划关与矩阵评审同属①需求分析（分阶段流）', () => {
    expect(stageOf('awaiting_approval', 'plan')).toBe(0)
    expect(stageOf('awaiting_approval', 'analysis')).toBe(0)
  })

  it('方案评审属②需求方案', () => {
    expect(stageOf('awaiting_approval', 'solution')).toBe(1)
  })

  it('运行期按阶段标记：p:analysis→① p:solution→② 实施→③', () => {
    expect(stageOf('running', 'p:analysis')).toBe(0)
    expect(stageOf('running', 'p:solution')).toBe(1)
    expect(stageOf('running', null)).toBe(2)
    expect(stageOf('running', 'p:implement')).toBe(2)
    expect(stageOf('running', 'plan'), 'supervised 低危 running 可能残留 plan gate').toBe(2)
  })

  it('diff 关与 report 关分属④⑤', () => {
    expect(stageOf('awaiting_approval', 'diff')).toBe(3)
    expect(stageOf('awaiting_approval', 'report')).toBe(4)
  })

  it('done 是全完成态', () => {
    expect(stageOf('done', 'done')).toBe('done')
  })

  it('status 优先：failed 残留 gate 值不制造假阶段', () => {
    // update_status 不清 gate——failed 任务可能带着 gate=diff 或 p:analysis
    expect(stageOf('failed', 'diff')).toBe('error')
    expect(stageOf('failed', 'plan')).toBe('error')
    expect(stageOf('failed', 'p:analysis')).toBe('error')
  })

  it('未启动/驳回/中断是灰态', () => {
    expect(stageOf('pending', null)).toBe('error')
    expect(stageOf('rejected', 'diff')).toBe('error')
    expect(stageOf('interrupted', null)).toBe('error')
  })

  it('未知 status/gate 兜底灰态，不崩溃不造假', () => {
    expect(stageOf('weird', null)).toBe('error')
    expect(stageOf('awaiting_approval', 'nonsense')).toBe('error')
  })
})
