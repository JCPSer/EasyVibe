// 域 client：对话/会话/压缩/重置/视图/建议。
// 消费者：chat-ui（ChatPanel/QuickAsk/WorkbenchPage/MessageStream/SuggestPanel）。
import { apiFetch, repoBase, jsonInit } from './core'

/** 对话历史：query 为调用点自建的查询串（保留 conv/before/limit 拼接细节） */
export const chatHistory = (repo: string, query = '', init?: RequestInit) =>
  apiFetch(`${repoBase(repo)}/chat${query}`, init)
export const sendChat = (repo: string, body: unknown) =>
  apiFetch(`${repoBase(repo)}/chat`, jsonInit('POST', body))
export const compactChat = (repo: string, query = '') =>
  apiFetch(`${repoBase(repo)}/chat/compact${query}`, { method: 'POST' })
export const resetChat = (repo: string, query = '') =>
  apiFetch(`${repoBase(repo)}/chat/reset${query}`, { method: 'POST' })
export const conversations = (repo: string, init?: RequestInit) =>
  apiFetch(`${repoBase(repo)}/conversations`, init)
export const conversation = (repo: string, cid: string, init?: RequestInit) =>
  apiFetch(`${repoBase(repo)}/conversations/${encodeURIComponent(cid)}`, init)
export const views = (repo: string, query = '', init?: RequestInit) =>
  apiFetch(`${repoBase(repo)}/views${query}`, init)
export const view = (repo: string, slug: string, init?: RequestInit) =>
  apiFetch(`${repoBase(repo)}/views/${encodeURIComponent(slug)}`, init)
export const saveView = (repo: string, body: unknown) =>
  apiFetch(`${repoBase(repo)}/views`, jsonInit('POST', body))
export const renameView = (repo: string, slug: string, name: string) =>
  apiFetch(`${repoBase(repo)}/views/${encodeURIComponent(slug)}`, jsonInit('PUT', { name }))
export const deleteView = (repo: string, slug: string) =>
  apiFetch(`${repoBase(repo)}/views/${encodeURIComponent(slug)}`, { method: 'DELETE' })
export const suggest = (repo: string, init?: RequestInit) =>
  apiFetch(`${repoBase(repo)}/suggest`, init ?? { method: 'POST' })
