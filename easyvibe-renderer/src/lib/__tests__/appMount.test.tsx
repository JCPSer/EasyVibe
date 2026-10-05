// 白屏根因复现 v2（2026-10-05）：模拟真实启动时序——/api/repos 先成功、/data/map.json 先失败后恢复。
// 生产环境 #310 在 jsdom 朴素挂载下不复现（fetch 全败），必须按真实数据流驱动。
// @vitest-environment jsdom
import { describe, expect, it, vi, afterEach } from 'vitest'
import { render } from '@testing-library/react'
import App from '@/App'

// 仓库内的真实项目地图 fixture，避免依赖维护者本机的绝对路径。
import REAL_MAP from '../../../../fixtures/hover-client-v2.1-pilot/expected/map.json?raw'


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
