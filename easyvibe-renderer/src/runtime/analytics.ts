// R3 D1：使用证据埋点（门控的秤）——fire-and-forget POST /events，失败静默（不打扰用户）。
// 事件名 dot.case：ui.* 前端交互 / task.* patrol.* 服务端（总线持久化任务代写）。
// 读数：GET /api/repos/{id}/events/summary（L3 三道门、Harness 验证指标的判定数据源）。
// c-arch-9：REST 取数经 @/api 唯一 fetch 出口（不再直连）。
import { ingestEvent } from '@/api/repos'

export function track(repo: string | null | undefined, name: string, payload?: Record<string, unknown>) {
  if (!repo) return
  ingestEvent(repo, { name, payload: payload ?? {} }).catch(() => {})
}
