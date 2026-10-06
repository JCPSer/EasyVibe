// 宿主能力适配器（host adapter）——app-entry（order 0），与 desktop-shell 同层。
//
// c-arch-5：本文件是**全库唯一** `@tauri-apps/*` 落点（原 src/runtime/host.ts 的实现整体搬到这里）。
//   · 依赖方向恒为下行：host-adapter(0) → renderer-runtime(2)（实现端口契约），
//     故不再产生任何「renderer-runtime(2) → desktop-shell(0)」的逆边，direction_violation = 0；
//   · 新增 Tauri 插件（第 7 个）只改本模块，**不需要**动 presentation / presentation-support 任何模块，
//     结构上明确允许「第二个落点」（见 check_host_boundary.py B1）。
//   · 模块求值时自注册进端口（boot.tsx 以静态 import 保证先于 App 装载求值，无竞态）。
//
// 语义逐条与原门面等价：
//   · 探测唯一化：复用 runtime/env.ts::isTauriRuntime（不裸判 '__TAURI_INTERNALS__'）。
//   · 非 Tauri（浏览器 dev/preview）：全部能力静默 no-op / null，且不加载任何 tauri 插件。
//   · 动态 import：6 个 tauri 包不进主 chunk（code-split 语义保持）。
//   · 懒加载单例缓存 + 失败回滚不粘滞（插件一时不可得时下次可重试）。

import { setHostCapabilities, type HostCapabilities, type PickDirectoryOptions, type UpdateReady } from '@/runtime/host'
import { isTauriRuntime } from '@/runtime/env'

// ---------------- 内部：懒加载 + 单例缓存（失败不粘滞） ----------------
const cache = new Map<string, Promise<unknown>>()

function load<T>(key: string, loader: () => Promise<T>): Promise<T> {
  const hit = cache.get(key) as Promise<T> | undefined
  if (hit) return hit
  const p = loader().catch((e: unknown) => {
    cache.delete(key) // 关键：失败回滚，插件一时加载失败不永久粘滞
    throw e
  })
  cache.set(key, p)
  return p
}

const loadNotify = () => load('notify', () => import('@tauri-apps/plugin-notification'))
const loadWindow = () => load('window', () => import('@tauri-apps/api/window'))
const loadDialog = () => load('dialog', () => import('@tauri-apps/plugin-dialog'))
const loadUpdater = () => load('updater', () => import('@tauri-apps/plugin-updater'))
const loadProcess = () => load('process', () => import('@tauri-apps/plugin-process'))
const loadEvent = () => load('event', () => import('@tauri-apps/api/event'))

const noop = () => {}

// —— 能力 1：系统通知（首次请求权限一次 + 失败静默）——
let asked = false

const impl: HostCapabilities = {
  async notify(title: string, body: string): Promise<void> {
    if (!isTauriRuntime()) return
    try {
      const m = await loadNotify()
      if (!asked) {
        asked = true
        if (!(await m.isPermissionGranted())) await m.requestPermission()
      }
      if (await m.isPermissionGranted()) await m.sendNotification({ title, body })
    } catch {
      // 插件缺失/权限拒绝——系统通知不可用时静默（界面内 toast 仍是底线）
    }
  },

  // —— 能力 2/3：窗口拖拽 / 最大化（非 Tauri no-op）——
  startWindowDrag(): void {
    if (!isTauriRuntime()) return
    loadWindow()
      .then((m) => m.getCurrentWindow().startDragging())
      .catch(noop)
  },

  toggleWindowMaximize(): void {
    if (!isTauriRuntime()) return
    loadWindow()
      .then((m) => m.getCurrentWindow().toggleMaximize())
      .catch(noop)
  },

  // —— 能力 4：目录选择（非 Tauri 返回 null；宿主侧异常上抛，由调用方决定降级文案）——
  async pickDirectory(opts: PickDirectoryOptions = {}): Promise<string | null> {
    if (!isTauriRuntime()) return null
    const m = await loadDialog()
    const sel = await m.open({ directory: true, title: opts.title })
    return typeof sel === 'string' ? sel : null
  },

  // —— 能力 5：更新（检查 + 安装并返回版本；重启）——
  async checkAndInstallUpdate(): Promise<UpdateReady | null> {
    if (!isTauriRuntime()) return null
    const m = await loadUpdater()
    const update = await m.check()
    if (!update) return null
    await update.downloadAndInstall()
    return { version: update.version }
  },

  async relaunchApp(): Promise<void> {
    if (!isTauriRuntime()) return
    const m = await loadProcess()
    await m.relaunch()
  },

  // —— 能力 6：宿主事件订阅（返回可注销函数；非 Tauri / 订阅失败返回 no-op）——
  async onBackendRecovered(cb: () => void): Promise<() => void> {
    if (!isTauriRuntime()) return noop
    try {
      const m = await loadEvent()
      return await m.listen('backend-recovered', cb)
    } catch {
      return noop
    }
  },
}

/** 显式注册入口（供引导脚本/测试调用；模块求值时也会自动注册一次）。 */
export function registerHostCapabilities(): void {
  setHostCapabilities(impl)
}

// 模块求值即注册：boot.tsx 以静态 import 保证先于 @/main 求值（无竞态）。
registerHostCapabilities()
