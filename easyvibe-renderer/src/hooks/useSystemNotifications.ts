import { useEffect } from 'react'
import { onQueueChanged, onSessionEvent } from '@/runtime/growthBus'
import { sysNotify } from '@/runtime/notify'
import { t } from '@/runtime/i18n'

/**
 * 系统级通知：窗口失焦/后台时送达通知中心（toast 只有前台可见）。
 * 会话失败 = 需要人来看；排队 drained+started = 离开等排队的用户该回来了。
 */
export function useSystemNotifications(backendRepo: string | null) {
  useEffect(
    () =>
      onSessionEvent((e) => {
        if (e.repo !== backendRepo || e.status !== 'failed') return
        void sysNotify(t('hooks.notify.sessionFailedTitle'), t('hooks.notify.sessionFailedBody', { id: e.sessionId }))
      }),
    [backendRepo],
  )
  useEffect(
    () =>
      onQueueChanged((e) => {
        if (e.repo !== backendRepo || e.type !== 'drained' || !e.started) return
        void sysNotify(t('hooks.notify.queueStartedTitle'), e.job?.label ?? '')
      }),
    [backendRepo],
  )
}
