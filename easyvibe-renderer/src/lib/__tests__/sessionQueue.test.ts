import { afterEach, describe, expect, it, vi } from 'vitest'
import { enqueue } from '@/lib/sessionQueue'

// sessionQueue helper：POST /repos/{id}/session-queue 的契约消费
// （docs/requirements-session-bubble-queue.md §5 + 评审 B4/S3）

const enqueueBody = (kind: string, moduleId?: string) =>
  moduleId ? { kind, moduleId } : { kind }

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('enqueue', () => {
  it('202 + queued:true（无替换）→ outcome=queued', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ data: { queued: true, replaced: null } }), { status: 202 }),
    )
    vi.stubGlobal('fetch', fetchMock)
    const res = await enqueue('demo', 'patrol')
    expect(res).toEqual({ outcome: 'queued', replacedLabel: null })
    expect(fetchMock).toHaveBeenCalledWith('/api/repos/demo/session-queue', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(enqueueBody('patrol')),
    })
  })

  it('202 + replaced 带旧 label（S3）→ outcome=replaced 且透传 label', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ data: { queued: true, replaced: { label: '巡检' } } }), { status: 202 }),
      ),
    )
    const res = await enqueue('demo', 'reinduce')
    expect(res).toEqual({ outcome: 'replaced', replacedLabel: '巡检' })
  })

  it('202 + started:true（无活动会话，后端直接执行，B4）→ outcome=started', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(new Response(JSON.stringify({ data: { started: true } }), { status: 202 })),
    )
    const res = await enqueue('demo', 'submap', 'api-gateway')
    expect(res).toEqual({ outcome: 'started', replacedLabel: null })
  })

  it('submap 入队带 moduleId，patrol 不带', async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ data: { queued: true, replaced: null } }), { status: 202 }),
    )
    vi.stubGlobal('fetch', fetchMock)
    await enqueue('demo', 'submap', 'api-gateway')
    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({ kind: 'submap', moduleId: 'api-gateway' })
  })

  it('非 202（如 500）→ 返回 null 且打错误 toast', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: '队列不可用' }), { status: 500 })),
    )
    const res = await enqueue('demo', 'patrol')
    expect(res).toBeNull()
  })

  it('网络异常 → 返回 null 且打错误 toast', async () => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('offline')))
    const res = await enqueue('demo', 'patrol')
    expect(res).toBeNull()
  })
})
