import { useEffect, useRef, useState } from 'react'
import type { PageId } from '@/components/shell/AppShell'
import { listTasks, taskApprovals } from '@/api/task'
import { toast } from '@/runtime/toast'
import { ONBOARDING_COPY } from '@/shared/logic/onboardingCopy'
import { loadOnboarding, markCheck, completeAll, CHECK_KEYS } from '@/lib/onboarding'

/**
 * 新手引导状态机（B5）：版本化状态 + 欢迎页开合 + 事件驱动勾选 ×4 + firstApproval 轮询 + 全部完成收尾。
 * 从 App 抽出；hook 调用必须位于 App 任一提前 return 之前（历史 P0 hooks 序违规口径）。
 */
export function useOnboarding(ctx: { page: PageId; hasMap: boolean; repoCount: number; backendRepo: string | null }) {
  const { page, hasMap, repoCount, backendRepo } = ctx
  // 2026-10-04 新手引导：版本化状态（lib/onboarding）+ 帮助菜单强制重开
  const [onboarding, setOnboarding] = useState(loadOnboarding)
  const [welcomeOpen, setWelcomeOpen] = useState(false)

  // ---------- 新手引导：事件驱动勾选（调研 C2——完成 = 真实激活动作，不是"看过"） ----------
  useEffect(() => {
    if (repoCount > 0) setOnboarding((prev) => markCheck(prev, 'addRepo'))
  }, [repoCount])
  useEffect(() => {
    if (page === 'map' && hasMap) setOnboarding((prev) => markCheck(prev, 'viewMap'))
  }, [page, hasMap])
  useEffect(() => {
    if (page === 'health' || page === 'drift') setOnboarding((prev) => markCheck(prev, 'viewHealth'))
  }, [page])
  // firstApproval：轮询探测"任何任务存在审批记录"（只在本项未完成时跑，30s 节拍）
  useEffect(() => {
    if (!backendRepo || onboarding.checklist.firstApproval === 'done') return
    let dead = false
    const probe = async () => {
      try {
        const r = await listTasks(backendRepo)
        const d: { data?: { id: string }[] } = r.ok ? await r.json() : null
        for (const t of (d?.data ?? []).slice(0, 5)) {
          const ra = await taskApprovals(backendRepo, t.id)
          if (!ra.ok) continue
          const da: { data?: unknown[] } = await ra.json()
          if ((da?.data?.length ?? 0) > 0) {
            if (!dead) setOnboarding((prev) => markCheck(prev, 'firstApproval'))
            return
          }
        }
      } catch {
        /* 后端离线等场景静默 */
      }
    }
    void probe()
    const t = window.setInterval(() => void probe(), 30000)
    return () => {
      dead = true
      window.clearInterval(t)
    }
  }, [backendRepo, onboarding.checklist.firstApproval])
  // 全部完成 → 庆祝 + 自动收尾（只触发一次）
  const onboardDoneRef = useRef(-1)
  useEffect(() => {
    const n = CHECK_KEYS.filter((k) => onboarding.checklist[k] === 'done').length
    if (n === CHECK_KEYS.length && onboardDoneRef.current !== n) {
      toast(ONBOARDING_COPY.checklist.doneToast, 'info')
      setOnboarding((prev) => completeAll(prev))
    }
    onboardDoneRef.current = n
  }, [onboarding])

  /** 任务创建成功 → 勾选 firstTask（App 两处一次性动作共用） */
  const markFirstTask = () => setOnboarding((prev) => markCheck(prev, 'firstTask'))

  return { onboarding, setOnboarding, welcomeOpen, setWelcomeOpen, markFirstTask }
}
