// 宿主能力门面（host facade）——全库唯一允许 import '@tauri-apps/*' 的文件（R1，c-arch-3）。
// 归属 renderer-runtime（presentation-support）：业务层（console-ui 等）只经本门面调用宿主能力，
// 不再直接接触 tauri 包，也不再裸判 '__TAURI_INTERNALS__'。纪律由 archGuard 断言组 6 在测试期固化。
//
// 设计要点：
//   · 探测实现唯一：复用 runtime/env.ts::isTauriRuntime，这里只 re-export（避免两份实现漂移）。
//   · 动态 import 保留：tauri 插件不进主 chunk（code-split 语义不变）。
//   · 懒加载 + 单例缓存：失败回滚缓存，插件一时不可得时下次可重试（更新按小时轮询）。
//   · 浏览器（dev/preview）下所有能力静默降级为 no-op / null，调用方降级语义逐条不变。

import { isTauriRuntime } from '@/runtime/env'

export { isTauriRuntime }

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

export async function notify(title: string, body: string): Promise<void> {
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
}

// —— 能力 2/3：窗口拖拽 / 最大化（非 Tauri no-op）——
export function startWindowDrag(): void {
  if (!isTauriRuntime()) return
  loadWindow()
    .then((m) => m.getCurrentWindow().startDragging())
    .catch(noop)
}

export function toggleWindowMaximize(): void {
  if (!isTauriRuntime()) return
  loadWindow()
    .then((m) => m.getCurrentWindow().toggleMaximize())
    .catch(noop)
}

// —— 能力 4：目录选择（非 Tauri 返回 null；宿主侧异常上抛，由调用方决定降级文案）——
export interface PickDirectoryOptions {
  title?: string
}

export async function pickDirectory(opts: PickDirectoryOptions = {}): Promise<string | null> {
  if (!isTauriRuntime()) return null
  const m = await loadDialog()
  const sel = await m.open({ directory: true, title: opts.title })
  return typeof sel === 'string' ? sel : null
}

// —— 能力 5：更新（检查 + 安装并返回版本；重启）——
export interface UpdateReady {
  version: string
}

export async function checkAndInstallUpdate(): Promise<UpdateReady | null> {
  if (!isTauriRuntime()) return null
  const m = await loadUpdater()
  const update = await m.check()
  if (!update) return null
  await update.downloadAndInstall()
  return { version: update.version }
}

export async function relaunchApp(): Promise<void> {
  if (!isTauriRuntime()) return
  const m = await loadProcess()
  await m.relaunch()
}

// —— 能力 6：宿主事件订阅（返回可注销函数；非 Tauri / 订阅失败返回 no-op）——
export async function onBackendRecovered(cb: () => void): Promise<() => void> {
  if (!isTauriRuntime()) return noop
  try {
    const m = await loadEvent()
    return await m.listen('backend-recovered', cb)
  } catch {
    return noop
  }
}
