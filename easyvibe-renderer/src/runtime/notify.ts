// 系统级通知（tauri-plugin-notification）：窗口在后台/失焦时，关键事件送达 macOS 通知中心。
// 触发点（App.tsx 集中订阅，页面卸载也不断流）：
//   · 会话失败（归纳/巡检/任务执行挂了，用户可能正盯着别的窗口）
//   · 排队任务轮到执行（用户离开等排队时最需要）
// 权限：首次调用请求一次；浏览器/插件缺失时静默降级为无操作。

import { isTauriRuntime } from '@/runtime/env'

let asked = false

export async function sysNotify(title: string, body: string) {
  if (!isTauriRuntime()) return
  try {
    const m = await import('@tauri-apps/plugin-notification')
    if (!asked) {
      asked = true
      if (!(await m.isPermissionGranted())) await m.requestPermission()
    }
    if (await m.isPermissionGranted()) {
      await m.sendNotification({ title, body })
    }
  } catch {
    // 插件缺失/权限拒绝——系统通知不可用时静默（界面内 toast 仍是底线）
  }
}
