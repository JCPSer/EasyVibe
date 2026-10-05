// 域 client：任务（列表/创建/决策/审批/diff/管理动作/开发文档）。
// 消费者：task-ui（TaskWorkflowPage/TaskBoardPage/ChangesPage/DocCard）。
import { apiFetch, repoBase, jsonInit } from './core'

export const listTasks = (repo: string, query = '', init?: RequestInit) =>
  apiFetch(`${repoBase(repo)}/tasks${query}`, init)
export const createTask = (repo: string, body: unknown) =>
  apiFetch(`${repoBase(repo)}/tasks`, jsonInit('POST', body))
export const taskDiff = (repo: string, tid: string) =>
  apiFetch(`${repoBase(repo)}/tasks/${encodeURIComponent(tid)}/diff`)
export const taskApprovals = (repo: string, tid: string) =>
  apiFetch(`${repoBase(repo)}/tasks/${encodeURIComponent(tid)}/approvals`)
export const decideTask = (repo: string, tid: string, body: unknown) =>
  apiFetch(`${repoBase(repo)}/tasks/${encodeURIComponent(tid)}/decide`, jsonInit('POST', body))
/** kill/retry/remediate/rewind/review 等管理动作（init 缺省 POST） */
export const taskAction = (repo: string, tid: string, action: string, init?: RequestInit) =>
  apiFetch(`${repoBase(repo)}/tasks/${encodeURIComponent(tid)}/${action}`, init ?? { method: 'POST' })
export const deleteTask = (repo: string, tid: string) =>
  apiFetch(`${repoBase(repo)}/tasks/${encodeURIComponent(tid)}`, { method: 'DELETE' })
export const devDocs = (repo: string, taskId: string) =>
  apiFetch(`${repoBase(repo)}/dev-docs?taskId=${encodeURIComponent(taskId)}`)
export const devDoc = (repo: string, path: string, init?: RequestInit) =>
  apiFetch(`${repoBase(repo)}/dev-doc?path=${encodeURIComponent(path)}`, init)
