// 新手引导状态层（2026-10-04 调研定稿：版本化 schema，替代 ev.m4.guide 单一布尔值）
// 设计约束（调研报告 C2）：
// - "完成"由真实激活事件驱动（markCheck 只被事件调用，不以"用户点过"为完成）
// - 任何引导 Esc/关闭必可退出且不复活；dismissedAt 记录关闭时点
// - 新功能引导与首次引导解耦（本 schema 只管首次引导；版本迁移保活老状态）
// - 任务想法（等待期预填）独立 key，归纳完成后带入任务对话即清

export type CheckKey = 'addRepo' | 'viewMap' | 'viewHealth' | 'firstTask' | 'firstApproval'

export interface OnboardingState {
  version: 2
  startedAt: string
  /** 欢迎页是否已展示过（false = 首启；帮助菜单可重置重看） */
  welcomeShown: boolean
  checklist: Record<CheckKey, 'pending' | 'done' | 'skipped'>
  /** 全部完成/用户关闭的时间；null = 仍活跃 */
  dismissedAt: string | null
}

const KEY = 'ev.onboarding.v2'

export const CHECK_KEYS: CheckKey[] = ['addRepo', 'viewMap', 'viewHealth', 'firstTask', 'firstApproval']

function fresh(): OnboardingState {
  return {
    version: 2,
    startedAt: new Date().toISOString(),
    welcomeShown: false,
    checklist: { addRepo: 'pending', viewMap: 'pending', viewHealth: 'pending', firstTask: 'pending', firstApproval: 'pending' },
    dismissedAt: null,
  }
}

export function loadOnboarding(): OnboardingState {
  try {
    const raw = typeof window !== 'undefined' ? window.localStorage.getItem(KEY) : null
    if (!raw) return fresh()
    const s = JSON.parse(raw) as OnboardingState
    if (s?.version !== 2 || !s.checklist) return fresh()
    // 补全新增的检查项（向前兼容）
    for (const k of CHECK_KEYS) if (!(k in s.checklist)) s.checklist[k] = 'pending'
    return s
  } catch {
    return fresh()
  }
}

export function saveOnboarding(s: OnboardingState): void {
  try {
    window.localStorage.setItem(KEY, JSON.stringify(s))
  } catch {
    /* localStorage 不可用时静默降级（引导不阻断主流程） */
  }
}

/** 事件驱动标记：某项激活动作真实发生（调用方负责只在事件真实发生时调用） */
export function markCheck(s: OnboardingState, key: CheckKey): OnboardingState {
  if (s.checklist[key] === 'done') return s
  const next: OnboardingState = { ...s, checklist: { ...s.checklist, [key]: 'done' } }
  saveOnboarding(next)
  return next
}

/** 全部完成 → 收尾（记录时间，UI 据此隐藏并庆祝一次） */
export function completeAll(s: OnboardingState): OnboardingState {
  if (s.dismissedAt && CHECK_KEYS.every((k) => s.checklist[k] === 'done')) return s
  const next: OnboardingState = { ...s, dismissedAt: new Date().toISOString() }
  saveOnboarding(next)
  return next
}

/** 用户主动关闭（没做完也尊重——引导是权力不是义务） */
export function dismiss(s: OnboardingState): OnboardingState {
  const next: OnboardingState = { ...s, dismissedAt: new Date().toISOString() }
  saveOnboarding(next)
  return next
}

/** 帮助菜单"重新查看引导"：欢迎页与 checklist 全部重置（不含任务想法） */
export function resetForReview(): OnboardingState {
  const next = fresh()
  next.welcomeShown = true // 欢迎页由 UI 显式打开，不重复自动弹
  saveOnboarding(next)
  return next
}

/** 欢迎页首启完成（用户点了主 CTA 或跳过） */
export function markWelcomeShown(s: OnboardingState): OnboardingState {
  if (s.welcomeShown) return s
  const next: OnboardingState = { ...s, welcomeShown: true }
  saveOnboarding(next)
  return next
}

export function checkDoneCount(s: OnboardingState): number {
  return CHECK_KEYS.filter((k) => s.checklist[k] === 'done').length
}

// ---------- 等待期任务想法（归纳黄金空窗的"提前参与"，调研报告 B 节第 3 层） ----------
const IDEA_KEY = 'ev.onboarding.taskIdea'

export function saveTaskIdea(text: string): void {
  try {
    window.localStorage.setItem(IDEA_KEY, text)
  } catch {
    /* 同上 */
  }
}

export function loadTaskIdea(): string | null {
  try {
    return window.localStorage.getItem(IDEA_KEY)
  } catch {
    return null
  }
}

export function clearTaskIdea(): void {
  try {
    window.localStorage.removeItem(IDEA_KEY)
  } catch {
    /* 同上 */
  }
}

/** 尊重系统减弱动效偏好（轮播/打字机全部降级为静态） */
export function prefersReducedMotion(): boolean {
  try {
    return typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches
  } catch {
    return false
  }
}
