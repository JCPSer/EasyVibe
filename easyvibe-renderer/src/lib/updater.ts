// D5-2 桌面壳自动更新（Multica 蓝本：静默下载 + 下载完成才提示重启）。
// 仅桌面端生效：纯浏览器（开发流）没有 __TAURI_INTERNALS__，整体 no-op。
// 检查节奏：启动 5s 后首检 + 每小时轮询；并发检查 single-flight 合并（防重）。
// 插件模块动态 import——浏览器构建不会触碰 Tauri 专属代码。
import { toast } from '@/lib/toast'

let inFlight: Promise<void> | null = null
let started = false

export function initUpdater(): void {
  if (started) return
  started = true
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return
  setTimeout(() => void checkOnce(), 5000)
  setInterval(() => void checkOnce(), 3_600_000)
}

function checkOnce(): Promise<void> {
  if (inFlight) return inFlight
  inFlight = doCheck().finally(() => {
    inFlight = null
  })
  return inFlight
}

async function doCheck(): Promise<void> {
  try {
    const { check } = await import('@tauri-apps/plugin-updater')
    const update = await check()
    if (!update) return
    await update.downloadAndInstall()
    toast(`新版本 ${update.version} 已就绪`, 'info', {
      label: '重启更新',
      onClick: () => {
        void (async () => {
          const { relaunch } = await import('@tauri-apps/plugin-process')
          await relaunch()
        })()
      },
    })
  } catch {
    // 更新检查失败（离线/ manifest 未发布）不打断使用——下次轮询再试
  }
}
