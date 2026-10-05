import { useEffect, useState } from 'react'

/** 暗黑模式主题状态（localStorage 持久化，默认跟随系统）；html.dark 驱动 Tailwind class 策略 */
export function useTheme() {
  const [dark, setDark] = useState(() => {
    try {
      const saved = window.localStorage.getItem('ev.theme')
      if (saved === 'dark' || saved === 'light') return saved === 'dark'
      return window.matchMedia('(prefers-color-scheme: dark)').matches
    } catch {
      return false
    }
  })
  useEffect(() => {
    document.documentElement.classList.toggle('dark', dark)
    try {
      window.localStorage.setItem('ev.theme', dark ? 'dark' : 'light')
    } catch { /* 静默 */ }
  }, [dark])
  return { dark, setDark }
}
