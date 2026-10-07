import { useCallback, useEffect, useRef, useState } from 'react'
import type { SubMap } from '@/types/map'
import { t } from '@/runtime/i18n'
import { enqueue } from '@/runtime/sessionQueue'
import { onQueueChanged, onSessionEvent, onSessionOutput as onSessionOutputListener } from '@/runtime/growthBus'
import { analyzeSubmap as analyzeSubmapRequest, submap } from '@/api/canvas'
import { fetchStatic } from '@/api/core'

/**
 * 子图（模块内部结构）分析域：懒加载 / 深入分析 / 诚实三态错误 / agent 输出直播缓冲。
 * 从 Canvas 抽出，行为、依赖数组与事件订阅语义逐处保持（拆分为独立订阅，互不影响）。
 */
export function useSubmaps(backendRepo: string | null) {
  const [submaps, setSubmaps] = useState<Record<string, SubMap | 'loading' | 'error'>>({})
  // 改进#2：agent 过程直播——按会话存最近输出（子图分析/任务执行）
  const [agentLines, setAgentLines] = useState<Record<string, string[]>>({})
  // 子图分析会话号（模块 id → sessionId，用于匹配输出流）
  const [submapSessions, setSubmapSessions] = useState<Record<string, string>>({})
  // M4-1 诚实三态：分析错误原因（启动失败/会话失败/超时）按模块记录，UI 必须说人话
  const [submapErrors, setSubmapErrors] = useState<Record<string, string>>({})
  const submapsRef = useRef<typeof submaps | null>(null)

  useEffect(() => {
    submapsRef.current = submaps
  }, [submaps])

  // 切仓库清掉上一仓库的子图分析状态（原 Canvas 重置 effect 的子集）
  useEffect(() => {
    setSubmaps({})
    setSubmapErrors({})
    setSubmapSessions({})
    setAgentLines({})
  }, [backendRepo])

  // 改进#2：订阅 agent 输出流，按会话保留最近 4 行。
  // P1 审查 2#13 性能修复：两道闸——① 只接收"当前在地图上可见的子图分析会话"的行；
  // ② 500ms 尾随节流合并突发（分析高峰时行率可达数行/秒）。
  const submapSessionsRef = useRef<Record<string, string>>({})
  const agentBufferRef = useRef<Record<string, string[]>>({})
  const agentFlushTimerRef = useRef<number | undefined>(undefined)
  useEffect(() => {
    submapSessionsRef.current = submapSessions
    // 分析结束/重试的会话：其输出行不再展示，缓冲与状态同步剪枝
    const alive = new Set(Object.values(submapSessions))
    agentBufferRef.current = Object.fromEntries(Object.entries(agentBufferRef.current).filter(([k]) => alive.has(k)))
    setAgentLines((prev) => {
      const next = Object.fromEntries(Object.entries(prev).filter(([k]) => alive.has(k)))
      return Object.keys(next).length === Object.keys(prev).length ? prev : next
    })
  }, [submapSessions])
  useEffect(() => {
    const flush = () => {
      agentFlushTimerRef.current = undefined
      const buf = agentBufferRef.current
      if (Object.keys(buf).length === 0) return
      setAgentLines({ ...buf })
    }
    return onSessionOutputListener((e) => {
      const visible = Object.values(submapSessionsRef.current).includes(e.sessionId)
      if (!visible) return
      const cur = agentBufferRef.current[e.sessionId] ?? []
      agentBufferRef.current[e.sessionId] = [...cur.slice(-3), e.line]
      if (agentFlushTimerRef.current === undefined) {
        agentFlushTimerRef.current = window.setTimeout(flush, 500)
      }
    })
  }, [])

  // M4-1 诚实三态：子图分析会话终态失败 → 立即报"会话失败"，不等轮询超时
  useEffect(
    () =>
      onSessionEvent((evt) => {
        setSubmapSessions((prev) => {
          const hit = Object.entries(prev).find(([, sid]) => sid === evt.sessionId)
          if (hit && evt.status === 'failed') {
            setSubmapErrors((e) => ({ ...e, [hit[0]]: t('canvas.submap.sessionFailed') }))
            setSubmaps((p) => ({ ...p, [hit[0]]: 'error' }))
            setSubmapSessions((prev) => {
              const n = { ...prev }
              delete n[hit![0]]
              return n
            })
          }
          return prev
        })
      }),
    [],
  )

  // 懒加载子图：任何 loading 状态触发取数（后端模式走 API，否则静态文件）
  useEffect(() => {
    for (const [id, v] of Object.entries(submaps)) {
      if (v !== 'loading') continue
      const req = backendRepo ? submap(backendRepo, id) : fetchStatic(`/data/modules/${id}.json`)
      req
        .then((r) => {
          if (!r.ok) throw new Error(String(r.status))
          return r.json() as Promise<SubMap>
        })
        .then((d) => setSubmaps((p) => ({ ...p, [id]: d })))
        .catch(() => {
          // R4：失败保留展开态并标记 error（容器内显示重试），不再无声消失
          // M4-1.5 修：分析会话进行中（已启动未失败）时 404 只是产物未到——保持 loading
          setSubmaps((p) => (submapSessions[id] ? p : { ...p, [id]: 'error' }))
        })
    }
  }, [submaps, backendRepo, submapSessions])

  const retrySubmap = useCallback((id: string) => {
    setSubmaps((prev) => ({ ...prev, [id]: 'loading' }))
  }, [])

  /** 展开模块时确保进入 loading（原 toggleExpand 的子集） */
  const ensureSubmapLoading = useCallback((id: string) => {
    setSubmaps((prev) => (prev[id] ? prev : { ...prev, [id]: 'loading' }))
  }, [])

  // 子图深入分析：派透明 agent 扫描模块文件生成子图，落盘后即可加载。
  // M4-1 诚实三态：① POST 失败 → 立即报"启动失败"；② 会话终态失败 → 立即报"会话失败"；
  // ③ 轮询 6s×40=4 分钟无产物 → 报"超时"并停止。杜绝"永远转圈"。
  const analyzeSubmap = useCallback(
    (id: string) => {
      if (!backendRepo) return
      setSubmapErrors((prev) => ({ ...prev, [id]: '' }))
      analyzeSubmapRequest(backendRepo, id)
        .then(async (r) => {
          // 必须抛 Response 本体：catch 里要读 body 判别 409（单会话纪律）
          if (!r.ok) throw r
          const sess = (await r.json()) as { sessionId?: string; session_id?: string }
          const sid = sess.sessionId ?? sess.session_id ?? ''
          setSubmapSessions((prev) => ({ ...prev, [id]: sid }))
          setSubmaps((prev) => ({ ...prev, [id]: 'loading' }))
          let n = 0
          const pollTimer = window.setInterval(() => {
            n += 1
            const cur = submapsRef.current?.[id]
            if (cur && cur !== 'loading' && cur !== 'error') {
              window.clearInterval(pollTimer)
              return
            }
            if (n >= 40) {
              window.clearInterval(pollTimer)
              // 超时必须显式告之——此前静默停轮询，界面永远"分析中"
              setSubmapErrors((prev) => ({
                ...prev,
                [id]: t('canvas.submap.timeout'),
              }))
              setSubmaps((prev) => ({ ...prev, [id]: 'error' }))
              setSubmapSessions((prev) => {
                const n = { ...prev }
                delete n[id]
                return n
              })
              return
            }
            retrySubmap(id)
          }, 6000)
        })
        .catch(async (e) => {
          // 409=单会话纪律（另一个分析/归纳在跑）——一键入队，当前会话结束后自动接续
          let msg = t('canvas.submap.startFailed')
          try {
            const body = await (e as Response)?.json?.()
            if (body?.code === 'CONFLICT' || /conflict|活动会话/.test(String(body?.error ?? ''))) {
              const res = await enqueue(backendRepo, 'submap', id)
              if (res?.outcome === 'replaced')
                msg = t('canvas.submap.queuedReplaced', { label: res.replacedLabel ?? t('common.replacedFallback') })
              else if (res?.outcome === 'queued') msg = t('canvas.submap.queued')
              else if (res?.outcome === 'started') msg = t('canvas.submap.started')
            }
          } catch { /* 保持默认文案 */ }
          setSubmapErrors((prev) => ({ ...prev, [id]: msg }))
          setSubmaps((prev) => ({ ...prev, [id]: 'error' }))
        })
    },
    [backendRepo, retrySubmap],
  )

  // I3：排队任务被排空（当前会话终态 → 后端自动接续）——submap 分支：
  // 重新调用正常分析流程（此时会话可注册，自然进入既有 6s 轮询成功路径，I4 不重建状态机）
  useEffect(
    () =>
      onQueueChanged((evt) => {
        if (evt.repo !== backendRepo || evt.type !== 'drained' || !evt.job) return
        if (evt.job.kind === 'submap' && evt.job.moduleId) {
          analyzeSubmap(evt.job.moduleId)
        }
      }),
    [backendRepo, analyzeSubmap],
  )

  return { submaps, submapSessions, submapErrors, agentLines, retrySubmap, analyzeSubmap, ensureSubmapLoading }
}
