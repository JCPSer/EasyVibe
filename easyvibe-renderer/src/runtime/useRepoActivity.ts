import { useCallback, useEffect, useState } from 'react'
import { onQueueChanged, onSessionEvent } from '@/runtime/growthBus'
import { kindFromLabel } from '@/runtime/sessionQueue'

// 跨页共享的"归纳进行中"状态（2026-10-05 实弹修复：漂移洞察点了归纳，
// 地图头部因 inducing 只是本页局部态仍在放"立即归纳"，二次点击入队成重复归纳）。
// 数据源与 SessionBubble 同一份 GET /sessions/overview（活动会话 label → kind），
// 事件驱动刷新（会话/队列事件）+ 20s 低频兜底。

export interface RepoActivity {
  /** 有归纳类活动会话（含别页发起的） */
  inducing: boolean
  /** 该仓库已排队归纳（等待接续） */
  reinduceQueued: boolean
  /** 归纳活动会话标识（ticker 订阅/失败匹配/跳运行页用；会话终态后轮询自然清掉） */
  inductionSession?: { sessionId: string; label: string }
}

interface OverviewActive {
  sessionId: string
  repo: string
  label: string
  status: string
  startedAt?: string | null
}
interface OverviewQueued {
  repo: string
  kind: string
  label: string
}

const EMPTY_MAP = new Map<string, RepoActivity>()

export function useRepoActivityMap(): Map<string, RepoActivity> {
  const [map, setMap] = useState<Map<string, RepoActivity>>(EMPTY_MAP)

  const pull = useCallback(() => {
    fetch('/api/sessions/overview')
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { active?: OverviewActive[]; queued?: OverviewQueued[] } } | null) => {
        const next = new Map<string, RepoActivity>()
        for (const a of d?.data?.active ?? []) {
          if (kindFromLabel(a.label) !== 'reinduce') continue
          const cur = next.get(a.repo) ?? { inducing: false, reinduceQueued: false }
          cur.inducing = true
          cur.inductionSession = { sessionId: a.sessionId, label: a.label }
          next.set(a.repo, cur)
        }
        for (const q of d?.data?.queued ?? []) {
          if (q.kind !== 'reinduce') continue
          const cur = next.get(q.repo) ?? { inducing: false, reinduceQueued: false }
          cur.reinduceQueued = true
          next.set(q.repo, cur)
        }
        setMap(next)
      })
      .catch(() => {})
  }, [])

  useEffect(() => {
    pull()
    const offQ = onQueueChanged(() => pull())
    const offS = onSessionEvent(() => pull())
    const t = window.setInterval(pull, 20000)
    return () => {
      offQ()
      offS()
      window.clearInterval(t)
    }
  }, [pull])

  return map
}

const EMPTY: RepoActivity = { inducing: false, reinduceQueued: false }

export function useRepoActivity(repo: string | null): RepoActivity {
  const map = useRepoActivityMap()
  return (repo && map.get(repo)) || EMPTY
}
