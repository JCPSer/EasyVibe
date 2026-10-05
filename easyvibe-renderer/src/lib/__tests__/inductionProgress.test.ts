// 归纳原位过场纯逻辑：阶段标签表（v2.2 协议词汇）/ 陈旧 done 解读 / agent 输出行过滤口径。
import { describe, expect, it } from 'vitest'
import {
  INDUCTION_PHASE_LABELS,
  formatTickerLine,
  inductionPhaseLabel,
  interpretInductionProgress,
  type InductionProgress,
} from '@/shared/logic/inductionProgress'

const prog = (over: Partial<InductionProgress> = {}): InductionProgress => ({
  phase: 'scanning',
  percent: 40,
  modules_done: 4,
  modules_total: 10,
  ...over,
})

describe('归纳阶段标签表（v2.2 真实协议词汇）', () => {
  it('8 个真实 phase 全部映射到中文阶段名', () => {
    expect(INDUCTION_PHASE_LABELS).toEqual({
      init: '初始化',
      scanning: '模块扫描',
      clustering: '结构聚类',
      'module-analysis': '模块归纳',
      edging: '依赖边推导',
      health: '健康度评估',
      assembling: '收尾组装',
    })
  })

  it('旧协议的假键（layering/module_scan/emit 等）不在表中——存量 bug 不回潮', () => {
    for (const stale of ['layering', 'module_scan', 'emit', 'consistency', 'finalize']) {
      expect(INDUCTION_PHASE_LABELS[stale]).toBeUndefined()
    }
  })

  it('未知 phase 原样展示（不吞信息）', () => {
    expect(inductionPhaseLabel('whatever')).toBe('whatever')
  })
})

describe('interpretInductionProgress：进度文件 → 阶段卡视图', () => {
  it('progress.json 缺失（null）退回"归纳中 · agent 执行中"，不显示百分比', () => {
    expect(interpretInductionProgress(null)).toEqual({
      title: '归纳中',
      percent: null,
      subline: 'agent 执行中，通常数分钟',
    })
  })

  it('陈旧 done 陷阱：会话仍活动但 phase=done（上一次残留的进度文件）→ 写盘收尾中，不显示百分比', () => {
    const v = interpretInductionProgress(prog({ phase: 'done', percent: 100 }))
    expect(v.title).toBe('写盘收尾中')
    expect(v.percent).toBeNull()
  })

  it('failed 不展示（终态由退出逻辑处理），退回兜底文案', () => {
    const v = interpretInductionProgress(prog({ phase: 'failed', percent: 30 }))
    expect(v.title).toBe('归纳中')
    expect(v.percent).toBeNull()
  })

  it('module-analysis 且带 current_module → 副行"正在归纳模块：xxx"', () => {
    const v = interpretInductionProgress(prog({ phase: 'module-analysis', current_module: 'auth/session' }))
    expect(v.title).toBe('模块归纳')
    expect(v.subline).toBe('正在归纳模块：auth/session')
  })

  it('无 current_module 时按 modules_done/modules_total 出副行', () => {
    const v = interpretInductionProgress(prog({ phase: 'clustering', modules_done: 7, modules_total: 12 }))
    expect(v.subline).toBe('已归纳 7/12 个模块')
  })

  it('modules_total 为 0（无数据）退回 agent 执行中', () => {
    const v = interpretInductionProgress(prog({ modules_done: 0, modules_total: 0 }))
    expect(v.subline).toBe('agent 执行中，通常数分钟')
  })

  it('百分比钳到 0-100 且取整', () => {
    expect(interpretInductionProgress(prog({ percent: 134.6 })).percent).toBe(100)
    expect(interpretInductionProgress(prog({ percent: -3 })).percent).toBe(0)
  })
})

describe('formatTickerLine：agent 实时输出行过滤（复用 RunsPage 口径）', () => {
  it('[思考] 前缀剥掉并改"正在思考：…"', () => {
    expect(formatTickerLine('[思考] 让我看看这个模块的职责', 'stdout')).toBe('正在思考：让我看看这个模块的职责')
  })

  it('[思考] 后无正文 → 跳过', () => {
    expect(formatTickerLine('[思考]   ', 'stdout')).toBeNull()
  })

  it('stderr 行不显示（协议噪音）', () => {
    expect(formatTickerLine('some noise', 'stderr')).toBeNull()
  })

  it('[err] 前缀行不显示', () => {
    expect(formatTickerLine('[err] boom', 'stdout')).toBeNull()
  })

  it('空行/纯空白跳过', () => {
    expect(formatTickerLine('', 'stdout')).toBeNull()
    expect(formatTickerLine('   ', 'stdout')).toBeNull()
  })

  it('普通 stdout 行原样通过', () => {
    expect(formatTickerLine('扫描 src/auth（12 个文件）', 'stdout')).toBe('扫描 src/auth（12 个文件）')
  })
})
