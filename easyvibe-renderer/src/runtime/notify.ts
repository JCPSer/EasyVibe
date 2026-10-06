// 系统级通知（tauri-plugin-notification）：窗口在后台/失焦时，关键事件送达 macOS 通知中心。
// 触发点（App.tsx 集中订阅，页面卸载也不断流）：
//   · 会话失败（归纳/巡检/任务执行挂了，用户可能正盯着别的窗口）
//   · 排队任务轮到执行（用户离开等排队时最需要）
// 权限：首次调用请求一次；浏览器/插件缺失时静默降级为无操作。
//
// 实现已收敛进宿主门面 @/runtime/host（唯一接触 tauri 包处）；本文件保留 sysNotify 语义入口，
// 调用方（runtime/ws.ts、hooks/useSystemNotifications.ts）零改动。

import { notify } from '@/runtime/host'

export async function sysNotify(title: string, body: string) {
  return notify(title, body)
}
