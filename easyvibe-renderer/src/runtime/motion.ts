// 运行时动作偏好（自 lib/onboarding.ts 拆出，供 map-canvas 与 console-ui 共用而不反向依赖）。
/** 尊重系统减弱动效偏好（轮播/打字机全部降级为静态） */
export function prefersReducedMotion(): boolean {
  try {
    return typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches
  } catch {
    return false
  }
}
