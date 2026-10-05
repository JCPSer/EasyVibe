import { useCallback, useState } from 'react'
import { toast } from '@/runtime/toast'
import { enqueue } from '@/runtime/sessionQueue'
import { patrol } from '@/api/canvas'

/** 巡检触发（M4-1 从 Canvas 上移：顶栏"巡检"按钮由壳层持有） */
export function usePatrol(backendRepo: string | null) {
  const [patrolling, setPatrolling] = useState(false)
  const startPatrol = useCallback(() => {
    if (!backendRepo || patrolling) return
    setPatrolling(true)
    patrol(backendRepo)
      .then((r) => {
        // S2：抛 Response 本体——catch 里判别 409（单会话纪律）入队
        if (!r.ok) throw r
      })
      .catch(async (e) => {
        setPatrolling(false)
        // S2：409 不再静默——入队，当前会话结束后自动接续
        if ((e as Response)?.status === 409) {
          const res = await enqueue(backendRepo, 'patrol')
          if (res?.outcome === 'replaced')
            toast(`已加入队列：巡检将在当前会话结束后自动开始（已替换排队：${res.replacedLabel ?? '旧任务'}）`)
          else if (res?.outcome === 'queued') toast('已加入队列：巡检将在当前会话结束后自动开始')
          else if (res?.outcome === 'started') {
            setPatrolling(true)
            toast('已直接开始巡检')
          }
          return
        }
        toast('巡检启动失败（请确认后端在线后重试）。', 'error')
      })
  }, [backendRepo, patrolling])
  return { patrolling, setPatrolling, startPatrol }
}
