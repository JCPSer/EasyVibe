// 原生红绿灯占位（参考 multica/Electron hiddenInset 范式）：
// macOS：title_bar_style=Overlay——系统标题栏隐藏、原生交通灯悬浮于内容左上角，
// 前端留 76px 净空；红绿灯的绘制/悬停/双击缩放全部由系统负责。
// Windows/Linux：窗口走原生标题栏（系统按钮在右上），左侧不存在红绿灯——
// 若保留 76px 净空，品牌图标会悬空（2026-10-04 Windows 实弹：用户报"图标位置不正常"）。
// 平台判定用 userAgent（Tauri WebView 与浏览器一致）。
export function TrafficLightsSpacer() {
  const isMac = typeof navigator !== 'undefined' && /mac/i.test(navigator.userAgent)
  return <span className={`${isMac ? 'w-[76px]' : 'w-3'} shrink-0`} aria-hidden data-no-drag />
}
