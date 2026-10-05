// 运行页选中逻辑（2026-10-05 用户反馈）：点「运行」导航应直接看到内容而不是空白右栏。
//  ① deeplink：initialSessionId 在场时选中它（一次性消费）；
//  ② 自动选中：未选中时优先"运行中"头条，其次最近历史。
// 抽 hooks/ 保 RunsPage 体量红线（componentGuard LEGACY ratchet 只降不升）。
import { useEffect } from 'react'

export function useRunsAutoSelect(opts: {
  selectedId: string | null
  initialSessionId?: string | null
  active: { sessionId: string; repo: string }[]
  history: { id: string; repo: string }[]
  select: (id: string, repo?: string) => void
  onInitialConsumed?: () => void
}) {
  const { selectedId, initialSessionId, active, history, select, onInitialConsumed } = opts
  // deeplink：打开即选中对应会话（一次性消费）
  useEffect(() => {
    if (!initialSessionId) return
    const id = initialSessionId
    const t = window.setTimeout(() => {
      select(id)
      onInitialConsumed?.()
    }, 0)
    return () => window.clearTimeout(t)
  }, [initialSessionId, onInitialConsumed, select])
  // 自动选中
  useEffect(() => {
    if (selectedId || initialSessionId) return
    const firstActive = active[0]
    if (firstActive) {
      select(firstActive.sessionId, firstActive.repo)
      return
    }
    const firstHist = history[0]
    if (firstHist) select(firstHist.id, firstHist.repo)
  }, [selectedId, initialSessionId, active, history, select])
}
