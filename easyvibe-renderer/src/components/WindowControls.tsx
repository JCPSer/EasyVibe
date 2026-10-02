// macOS 原生红绿灯占位（参考 multica/Electron hiddenInset 范式）：
// 桌面壳 title_bar_style=Overlay——系统标题栏隐藏、原生交通灯悬浮于内容左上角，
// 前端只需留出 76px 净空；红绿灯的绘制/悬停/双击缩放全部由系统负责。
// 浏览器环境渲染同一占位，布局与桌面壳完全一致。
export function TrafficLightsSpacer() {
  return <span className="w-[76px] shrink-0" aria-hidden data-no-drag />
}
