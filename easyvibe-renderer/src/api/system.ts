// 域 client：系统/运行时（health/agent/llm + 会话输出·终止·队列）。
// 消费者：useBackendConnection（版本锚/探测）、settings-ui（agent 区）、RunsPage/SessionBubble。
import { apiFetch, repoBase, jsonInit } from './core'

export const health = () => apiFetch('/health')
export const agentStatus = () => apiFetch('/agent/status')
export const agentTest = () => apiFetch('/agent/test', { method: 'POST' })
export const agentDetect = () => apiFetch('/agent/detect', { method: 'POST' })
export const llmTest = (body: unknown) => apiFetch('/llm/test', jsonInit('POST', body))
export const sessionOutput = (repo: string, sessionId: string, afterSeq: number, limit = 5000) =>
  apiFetch(`${repoBase(repo)}/sessions/${encodeURIComponent(sessionId)}/output?afterSeq=${afterSeq}&limit=${limit}`)
export const killSession = (repo: string, sessionId: string) =>
  apiFetch(`${repoBase(repo)}/sessions/${encodeURIComponent(sessionId)}/kill`, { method: 'POST' })
export const cancelSessionQueue = (repo: string) =>
  apiFetch(`${repoBase(repo)}/session-queue`, { method: 'DELETE' })
