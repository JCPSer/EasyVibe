// 白屏根因复现 v2（2026-10-05）：模拟真实启动时序——/api/repos 先成功、/data/map.json 先失败后恢复。
// 生产环境 #310 在 jsdom 朴素挂载下不复现（fetch 全败），必须按真实数据流驱动。
// @vitest-environment jsdom
import { describe, expect, it, vi, afterEach } from 'vitest'
import { render } from '@testing-library/react'
import App from '@/App'

// @ts-expect-error vitest 运行时支持 node:fs（tsconfig 无 node 类型）
import { readFileSync } from 'node:fs'
// 真实 hover-client 地图（12 模块 / 77 边 / 6 层 / 5 逆向违规）——小地图测不出的渲染分支靠它
const REAL_MAP = readFileSync('/Users/liyuhang/Documents/git_projects/language-band/hover-client/.easyvibe/map/map.json', 'utf-8')


// jsdom 缺 ResizeObserver / DOMMatrix——ReactFlow（ZoomPane）需要，打桩即可
class RO {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal('ResizeObserver', RO)
vi.stubGlobal('DOMMatrixReadOnly', class {})
vi.stubGlobal('DOMMatrix', class {})
vi.stubGlobal('DOMPointReadOnly', class {})
if (!('matchMedia' in window)) {
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener() {}, removeEventListener() {} }))
}

function mockFetchSequence() {
  let mapCalls = 0
  vi.stubGlobal('fetch', vi.fn((url: RequestInfo | URL) => {
    const u = String(url)
    if (u.includes('/api/repos') && !u.includes('/repos/')) {
      return Promise.resolve(new Response(JSON.stringify({ success: true, data: [{ id: 'demo', name: 'demo' }] }), { status: 200 }))
    }
    if (u.endsWith('/map') || u.includes('/data/map.json')) {
      mapCalls += 1
      // 第一次失败（模拟后端未就绪窗口期），之后成功；/map 端点返回裸 CodeMap（非信封）
      if (mapCalls < 2) return Promise.resolve(new Response('not found', { status: 404 }))
      return Promise.resolve(new Response(REAL_MAP, { status: 200 }))
    }
    if (u.includes('/session-queue')) {
      return Promise.resolve(new Response(JSON.stringify({ success: true, data: { active: null, queued: null } }), { status: 200 }))
    }
    return Promise.resolve(new Response(JSON.stringify({ success: true, data: null }), { status: 200 }))
  }))
}

describe('App 真实启动时序（白屏回归）', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('repos 先到 / map 失败后恢复：全程无 hooks 序违规', async () => {
    mockFetchSequence()
    const spy = vi.spyOn(console, 'error').mockImplementation(() => {})
    render(<App />)
    await new Promise((r) => setTimeout(r, 2500))
    const fmt = (c: unknown[]) => c.map((a) => (typeof a === 'string' ? a : String(a))).join(' ')
    const all = spy.mock.calls.map(fmt)
    const hooksErr = all.filter((s) => (s.includes('Rendered') && s.includes('hooks')) || s.includes('The above error occurred'))
    spy.mockRestore()
    if (hooksErr.length > 0) {
      throw new Error('HOOKS 违规复现（真实时序）\n' + hooksErr.slice(0, 4).join('\n---\n'))
    }
    expect(hooksErr).toEqual([])
  })
})
