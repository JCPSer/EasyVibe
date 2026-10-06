// 宿主能力端口（host capability port）——renderer-runtime（presentation-support / order 2）。
//
// c-arch-5 结构性消除：宿主能力从「门面 + 受控逆边」改为「契约 + 运行时注册」。
//   · 本文件**不含任何宿主插件（@tauri-apps 包）依赖**，只定义契约、注册表与默认实现；
//   · 真实现由 app-entry 层的适配模块 src/host-adapter/** 在引导期注入（register.ts）；
//   · 默认实现 = 全 no-op / null：浏览器（dev/preview）与「壳未注入」时同语义，调用方零改动；
//   · 依赖方向恒为下行：host-adapter(0) → renderer-runtime(2)（适配器实现端口），
//     全库不再存在任何指向 app-entry 的逆向能力通道（direction_violation = 0）。
//
// 设计要点（与原门面的三条约束逐条等价，见方案 R10）：
//   · 探测实现唯一：复用 runtime/env.ts::isTauriRuntime，这里只 re-export（避免两份实现漂移）。
//   · 动态 import 保留在适配器内：tauri 插件不进主 chunk（code-split 语义不变）。
//   · 懒加载 + 单例缓存 + 失败回滚：语义由适配器实现，端口只做转发。
//   · 浏览器（dev/preview）下所有能力静默降级为 no-op / null。
//
// 转发器一律用 `export function` 声明（archGuard 断言组 6 的导出面快照只识别函数声明与命名导出；
// 用 `export const` 承载能力名会让快照静默漏项）。

import { isTauriRuntime } from '@/runtime/env'

export { isTauriRuntime }

export interface PickDirectoryOptions {
  title?: string
}

export interface UpdateReady {
  version: string
}

/** 宿主能力契约：适配模块（app-entry）负责提供真实现，端口只暴露契约。 */
export interface HostCapabilities {
  notify(title: string, body: string): Promise<void>
  startWindowDrag(): void
  toggleWindowMaximize(): void
  pickDirectory(opts?: PickDirectoryOptions): Promise<string | null>
  checkAndInstallUpdate(): Promise<UpdateReady | null>
  relaunchApp(): Promise<void>
  onBackendRecovered(cb: () => void): Promise<() => void>
}

const noop = () => {}

/** 默认实现：无宿主注入时全静默降级（浏览器与「壳未注入」同语义）。 */
const NOOP_CAPABILITIES: HostCapabilities = {
  notify: async () => {},
  startWindowDrag: noop,
  toggleWindowMaximize: noop,
  pickDirectory: async () => null,
  checkAndInstallUpdate: async () => null,
  relaunchApp: async () => {},
  onBackendRecovered: async () => noop,
}

let table: HostCapabilities = NOOP_CAPABILITIES

/** 供 app-entry 适配模块在引导期注入能力表；传 null/undefined 视为复位为 no-op。 */
export function setHostCapabilities(cap: HostCapabilities | null | undefined): void {
  table = cap ?? NOOP_CAPABILITIES
}

/** 取当前能力表（默认为全 no-op）。 */
export function getHostCapabilities(): HostCapabilities {
  return table
}

// —— 能力 1：系统通知 ——
export async function notify(title: string, body: string): Promise<void> {
  return getHostCapabilities().notify(title, body)
}

// —— 能力 2/3：窗口拖拽 / 最大化 ——
export function startWindowDrag(): void {
  return getHostCapabilities().startWindowDrag()
}

export function toggleWindowMaximize(): void {
  return getHostCapabilities().toggleWindowMaximize()
}

// —— 能力 4：目录选择（非宿主返回 null）——
export async function pickDirectory(opts: PickDirectoryOptions = {}): Promise<string | null> {
  return getHostCapabilities().pickDirectory(opts)
}

// —— 能力 5：更新（检查 + 安装并返回版本）——
export async function checkAndInstallUpdate(): Promise<UpdateReady | null> {
  return getHostCapabilities().checkAndInstallUpdate()
}

export async function relaunchApp(): Promise<void> {
  return getHostCapabilities().relaunchApp()
}

// —— 能力 6：宿主事件订阅（返回可注销函数）——
export async function onBackendRecovered(cb: () => void): Promise<() => void> {
  return getHostCapabilities().onBackendRecovered(cb)
}
