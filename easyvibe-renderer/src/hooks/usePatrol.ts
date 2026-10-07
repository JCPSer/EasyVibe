import { useCallback, useState } from 'react'
import { toast } from '@/runtime/toast'
import { t } from '@/runtime/i18n'
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
            toast(t('hooks.patrol.queuedReplaced', { label: res.replacedLabel ?? t('common.replacedFallback') }))
          else if (res?.outcome === 'queued') toast(t('hooks.patrol.queued'))
          else if (res?.outcome === 'started') {
            setPatrolling(true)
            toast(t('hooks.patrol.started'))
          }
          return
        }
        toast(t('hooks.patrol.startFailed'), 'error')
      })
  }, [backendRepo, patrolling])
  return { patrolling, setPatrolling, startPatrol }
}
