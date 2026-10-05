// 前端 API client · 唯一 fetch 出口（c-arch-1 双枢纽收敛）。
//
// 设计口径（行为零改动）：
//  - 只把「`/api` 前缀 + fetch 调用」收口到本文件；调用点的 `.then/.ok/.json`、
//    错误文案、AbortSignal、状态码分支**一律保持原样**——client 只换传输层。
//  - 域模块（repos/git/canvas/chat/task/settings/system）各自持有本域 REST 路径，
//    业务文件（components/pages/hooks）只 import 域模块，不得再出现 `fetch(` 或 `/api/` 字面量
//    （由 `src/__tests__/archGuard.test.ts` 断言组 5 强制）。
//  - 分层：components/pages/hooks ──► src/api/**（leaf，只依赖外部库/自身相对路径）；
//    src/api/** 不得反向 import 业务层（components/pages/hooks/App）。

/** `{success, data?, message?}` 统一响应信封（对应后端 easyvibe-common::ApiResponse） */
export type ApiEnvelope<T> = { success: boolean; data?: T; message?: string }

/** 同源部署：Tauri 生产 WebView 与浏览器（dev/preview）都直连 127.0.0.1:PORT，相对路径即同源。 */
export function apiBase(): string {
  return ''
}

export function apiUrl(path: string): string {
  const p = path.startsWith('/') ? path : `/${path}`
  return path.startsWith('/api') ? `${apiBase()}${path}` : `${apiBase()}/api${p}`
}

/** 唯一 fetch 出口：语义等价 `fetch('/api' + path, init)`（非 JSON 兜底、状态码、signal 全部透传）。 */
export function apiFetch(path: string, init?: RequestInit): Promise<Response> {
  return fetch(apiUrl(path), init)
}

/** 非 REST 静态资源回落（离线 demo 数据 `/data/*.json|log`）——仍收口网络层，避免业务文件裸 fetch。 */
export function fetchStatic(url: string, init?: RequestInit): Promise<Response> {
  return fetch(url, init)
}

/** 仓库资源基路径（统一 `encodeURIComponent`，与既有调用点保持一致口径）。 */
export function repoBase(repo: string): string {
  return `/repos/${encodeURIComponent(repo)}`
}

/** JSON 请求体 init 帮手。 */
export function jsonInit(method: string, body: unknown): RequestInit {
  return { method, headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }
}

/** 解包 `{success, data}` 信封（迁移期调用点可继续自解；新代码用此）。 */
export async function unwrap<T>(res: Response): Promise<T | undefined> {
  const body = (await res.json()) as ApiEnvelope<T>
  return body.data
}
