// 终端环形缓冲回归（2026-10-05 实弹#2）：WS 与 HTTP 补拉交错时的 seq 幂等去重。
import { beforeEach, describe, expect, it } from 'vitest'
import { clearTerminal, pushTerminalLine, terminalLastSeq, terminalLines } from '@/runtime/terminalBuffer'

describe('terminalBuffer', () => {
  beforeEach(() => {
    clearTerminal('s1')
    clearTerminal('s2')
  })

  it('按 seq 追加并可读出', () => {
    pushTerminalLine('s1', 1, 'stdout', 'a')
    pushTerminalLine('s1', 2, 'stdout', 'b')
    expect(terminalLines('s1').map((l) => l.line)).toEqual(['a', 'b'])
    expect(terminalLastSeq('s1')).toBe(2)
  })

  it('seq 幂等去重——WS 推送与 HTTP 补拉交错不重复', () => {
    pushTerminalLine('s1', 1, 'stdout', 'a')
    pushTerminalLine('s1', 2, 'stdout', 'b') // WS 先到
    pushTerminalLine('s1', 2, 'stdout', 'b') // HTTP 补拉带回同 seq → 丢弃
    pushTerminalLine('s1', 3, 'stdout', 'c')
    expect(terminalLines('s1').map((l) => l.line)).toEqual(['a', 'b', 'c'])
  })

  it('保头丢尾且去重集同步驱逐（长跑不泄漏）', () => {
    for (let i = 1; i <= 250; i++) pushTerminalLine('s1', i, 'stdout', `l${i}`)
    const lines = terminalLines('s1')
    expect(lines.length).toBe(200)
    expect(lines[0].seq).toBe(51)
    expect(lines[199].seq).toBe(250)
    // 被逐出的旧 seq 重新出现（极端回放）时不会被误判重复
    pushTerminalLine('s1', 1, 'stdout', 'l1')
    expect(terminalLines('s1').length).toBe(201 - 1) // 51..250 共 200 + 新行 1 - 头挤掉 1
    expect(terminalLines('s1').at(-1)?.line).toBe('l1')
  })

  it('lastSeq：空缓冲返回 -1（从头拉）', () => {
    expect(terminalLastSeq('s2')).toBe(-1)
  })

  it('会话间隔离', () => {
    pushTerminalLine('s1', 1, 'stdout', 'x')
    pushTerminalLine('s2', 1, 'stderr', 'y')
    expect(terminalLines('s1')[0].line).toBe('x')
    expect(terminalLines('s2')[0].stream).toBe('stderr')
  })
})
