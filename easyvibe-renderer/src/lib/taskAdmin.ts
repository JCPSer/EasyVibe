// 任务管理三操作（2026-10-03 现状重审 P0：任务"只进不出"的消解）。
// 后端契约（easyvibe-app main.rs）：
//   POST   {tid}/kill   终止活动会话（running/awaiting_approval）；无会话 400
//   POST   {tid}/retry  failed/interrupted → pending 重新入队；其它状态 409
//   DELETE {tid}        running 先 best-effort 杀会话再删；级联 approvals + 任务归档
// 三个函数统一：!ok 时抛带服务端 message 的 Error，调用方 toast 呈现。

async function call(url: string, method: string): Promise<void> {
  const r = await fetch(url, { method })
  if (!r.ok) {
    const d = await r.json().catch(() => null)
    throw new Error(d?.error ?? `请求失败（${r.status}）`)
  }
}

export function killTask(repo: string, taskId: string): Promise<void> {
  return call(`/api/repos/${encodeURIComponent(repo)}/tasks/${encodeURIComponent(taskId)}/kill`, 'POST')
}

export function retryTask(repo: string, taskId: string): Promise<void> {
  return call(`/api/repos/${encodeURIComponent(repo)}/tasks/${encodeURIComponent(taskId)}/retry`, 'POST')
}

export function deleteTask(repo: string, taskId: string): Promise<void> {
  return call(`/api/repos/${encodeURIComponent(repo)}/tasks/${encodeURIComponent(taskId)}`, 'DELETE')
}
