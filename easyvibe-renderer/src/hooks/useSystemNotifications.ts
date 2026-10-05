import { useEffect } from 'react'
import { onQueueChanged, onSessionEvent } from '@/lib/growthBus'
import { sysNotify } from '@/lib/notify'

/**
 * 系统级通知：窗口失焦/后台时送达通知中心（toast 只有前台可见）。
 * 会话失败 = 需要人来看；排队 drained+started = 离开等排队的用户该回来了。
 */
export function useSystemNotifications(backendRepo: string | null) {
  useEffect(
    () =>
      onSessionEvent((e) => {
        if (e.repo !== backendRepo || e.status !== 'failed') return
        void sysNotify('EasyVibe · 会话失败', `会话 ${e.sessionId} 执行失败——回来看看原因`)
      }),
    [backendRepo],
  )
  useEffect(
    () =>
      onQueueChanged((e) => {
        if (e.repo !== backendRepo || e.type !== 'drained' || !e.started) return
        void sysNotify('EasyVibe · 排队任务已开始', e.job?.label ?? '')
      }),
    [backendRepo],
  )
}
