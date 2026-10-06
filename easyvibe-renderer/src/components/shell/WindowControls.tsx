// 原生红绿灯占位（参考 multica/Electron hiddenInset 范式）：
// macOS：title_bar_style=Overlay——系统标题栏隐藏、原生交通灯悬浮于内容左上角，
// 前端留 76px 净空；红绿灯的绘制/悬停/双击缩放全部由系统负责。
// Windows/Linux：窗口走原生标题栏（系统按钮在右上），左侧不存在红绿灯——
// 若保留 76px 净空，品牌图标会悬空（2026-10-04 Windows 实弹：用户报"图标位置不正常"）。
// 平台判定用 userAgent（Tauri WebView 与浏览器一致）。
//
// 2026-10-05 失焦修复：macOS 的 Overlay 交通灯在窗口失焦时会被系统隐藏，
// 76px 净空变成死域，左上角「空荡荡很难看」（用户截图实证）。
// 方案：失焦时在净空处画三枚仿交通灯灰点（坐标与真灯完全对齐：x=12/32/52、直径 12），
// 视觉恒有物；聚焦时仿灯隐藏，系统真灯接管同一位置。

import { useEffect, useState } from 'react'

export function TrafficLightsSpacer() {
  const isMac = typeof navigator !== 'undefined' && /mac/i.test(navigator.userAgent)
  return <span className={`${isMac ? 'w-[76px]' : 'w-3'} shrink-0`} aria-hidden data-no-drag />
}

/** 失焦仿交通灯：仅 mac 渲染；返回 null 时不占空间。
 *  焦点判定用 DOM 级 window focus/blur + document.hasFocus() 轮询——
 *  2026-10-05 实弹：Tauri 的 isFocused/onFocusChanged 需要 capabilities 权限且
 *  状态与系统灯可见性不一致（出现过聚焦时仿灯仍显示的双排）；webview 文档焦点
 *  与窗口焦点在单窗口应用里等价，DOM 事件零权限零 IPC，轮询兜底节流 1s。 */
export function FakeTrafficLights() {
  const isMac = typeof navigator !== 'undefined' && /mac/i.test(navigator.userAgent)
  const [focused, setFocused] = useState(() => (typeof document !== 'undefined' ? document.hasFocus() : true))
  useEffect(() => {
    if (!isMac) return
    const on = () => setFocused(true)
    const off = () => setFocused(false)
    window.addEventListener('focus', on)
    window.addEventListener('blur', off)
    const t = window.setInterval(() => setFocused(document.hasFocus()), 1000)
    return () => {
      window.removeEventListener('focus', on)
      window.removeEventListener('blur', off)
      window.clearInterval(t)
    }
  }, [isMac])
  if (!isMac || focused) return null
  // 与系统真灯同坐标：左 12 起、间距 20、直径 12——灰点即 macOS 失焦态交通灯的观感。
  // 相对 header 绝对定位，叠在 76px 净空区上（AppShell header 已 relative）
  return (
    <span className="pointer-events-none absolute left-0 top-0 z-0 h-full w-[76px]" aria-hidden data-no-drag>
      {[12, 32, 52].map((x) => (
        <span
          key={x}
          className="absolute top-1/2 h-3 w-3 -translate-y-1/2 rounded-full border border-slate-300/60 bg-slate-200/80 dark:border-slate-600/60 dark:bg-slate-700/80"
          style={{ left: x }}
        />
      ))}
    </span>
  )
}
