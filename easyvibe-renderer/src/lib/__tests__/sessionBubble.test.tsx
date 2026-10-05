import { describe, expect, it } from 'vitest'
import { renderToStaticMarkup } from 'react-dom/server'
import { SessionBubble } from '@/components/SessionBubble'
import { formatElapsed, isEmptyState, kindFromLabel } from '@/runtime/sessionQueue'

// SessionBubble 纯逻辑：时长计算 / 图标类型推断 / 空态渲染
// （时长 ticker 与事件驱动刷新属 hooks 副作用，由实弹验证覆盖）

describe('formatElapsed（已运行时长）', () => {
  it('零与负值钳到 0:00', () => {
    expect(formatElapsed(0)).toBe('0:00')
    expect(formatElapsed(-5000)).toBe('0:00')
  })

  it('秒内取整、分秒补零', () => {
    expect(formatElapsed(999)).toBe('0:00')
    expect(formatElapsed(60_000)).toBe('1:00')
    expect(formatElapsed(155_000)).toBe('2:35')
  })

  it('满 1 小时切 h:mm:ss', () => {
    expect(formatElapsed(3_723_000)).toBe('1:02:03')
  })
})

describe('kindFromLabel（active 无 kind 字段，从 label 推导图标类型）', () => {
  it('巡检 → patrol', () => {
    expect(kindFromLabel('巡检')).toBe('patrol')
  })
  it('分析模块 X → submap', () => {
    expect(kindFromLabel('分析模块 数据访问')).toBe('submap')
  })
  it('归纳/重新归纳/未知 → reinduce（Sparkles 兜底）', () => {
    expect(kindFromLabel('归纳')).toBe('reinduce')
    expect(kindFromLabel('重新归纳')).toBe('reinduce')
    expect(kindFromLabel('会话 ind-3')).toBe('reinduce')
  })
})

describe('isEmptyState（空态不渲染）', () => {
  it('null（未探测）视为空态', () => {
    expect(isEmptyState(null)).toBe(true)
  })
  it('active/queued 全 null → 空态', () => {
    expect(isEmptyState({ active: null, queued: null })).toBe(true)
  })
  it('仅有活动或仅有排队 → 非空态', () => {
    expect(isEmptyState({ active: { sessionId: 'ind-1', label: '归纳', status: 'running' }, queued: null })).toBe(false)
    expect(isEmptyState({ active: null, queued: { kind: 'patrol', label: '巡检' } })).toBe(false)
  })
})

describe('SessionBubble 空态渲染', () => {
  it('无后端仓库 → 不渲染', () => {
    expect(renderToStaticMarkup(<SessionBubble backendRepo={null} />)).toBe('')
  })
  it('首帧未探测（快照为 null）→ 不渲染，避免空泡闪烁', () => {
    // SSR 下 useEffect 不执行，等价于"GET 尚未返回"的初值态
    expect(renderToStaticMarkup(<SessionBubble backendRepo="demo" />)).toBe('')
  })
})
