// 任务管理三操作（2026-10-03 现状重审 P0：任务"只进不出"的消解）。
// 后端契约（easyvibe-app main.rs）：
//   POST   {tid}/kill   终止活动会话（running/awaiting_approval）；无会话 400
//   POST   {tid}/retry  failed/interrupted → pending 重新入队；其它状态 409
//   DELETE {tid}        running 先 best-effort 杀会话再删；级联 approvals + 任务归档
// 三个函数统一：!ok 时抛带服务端 message 的 Error，调用方 toast 呈现。

async function call(url: string, method: string, body?: string, contentType?: string): Promise<void> {
  const r = await fetch(url, { method, body, headers: contentType ? { 'content-type': contentType } : undefined })
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

/** 修改并复审：子 agent 审查打回的任务，注入审查意见直达实施阶段重跑（完成后自动复审） */
export function remediateTask(repo: string, taskId: string): Promise<void> {
  return call(`/api/repos/${encodeURIComponent(repo)}/tasks/${encodeURIComponent(taskId)}/remediate`, 'POST')
}

/**
 * 管道回看·节点重开（2026-10-05 方案 §3.1）：把任务放回目标评审关
 * （analysis/solution）。后续动作复用 decide——「通过」推进、「打回」带意见重跑本阶段。
 * 源状态：待审/失败/中断/审查打回/已归档；running 须先终止，auto 信任不支持。
 */
export function rewindTask(repo: string, taskId: string, gate: 'analysis' | 'solution'): Promise<void> {
  return call(`/api/repos/${encodeURIComponent(repo)}/tasks/${encodeURIComponent(taskId)}/rewind`, 'POST', JSON.stringify({ gate }), 'application/json')
}

/** 人工触发子 agent 复审（代码审查节点）：仅 Diff 关可发起，202 异步——
 *  结论经 result.review（at 时间戳）+ 留痕 + 事件送达，不迁移任务状态 */
export function reviewTask(repo: string, taskId: string): Promise<void> {
  return call(`/api/repos/${encodeURIComponent(repo)}/tasks/${encodeURIComponent(taskId)}/review`, 'POST')
}

export function deleteTask(repo: string, taskId: string): Promise<void> {
  return call(`/api/repos/${encodeURIComponent(repo)}/tasks/${encodeURIComponent(taskId)}`, 'DELETE')
}
