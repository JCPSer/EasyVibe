// 域 client：git 状态/日志/提交/同步/撤销。
// 消费者：GitPage / ChangesPage。
import { apiFetch, repoBase, jsonInit } from './core'

export const gitStatus = (repo: string) => apiFetch(`${repoBase(repo)}/git/status`)
export const gitLog = (repo: string, limit = 30) => apiFetch(`${repoBase(repo)}/git/log?limit=${limit}`)
export const gitCommit = (repo: string, hash: string) =>
  apiFetch(`${repoBase(repo)}/git/commit?hash=${encodeURIComponent(hash)}`)
export const commitMessage = (repo: string, body: unknown) =>
  apiFetch(`${repoBase(repo)}/git/commit-message`, jsonInit('POST', body))
export const postCommit = (repo: string, body: unknown) =>
  apiFetch(`${repoBase(repo)}/git/commit`, jsonInit('POST', body))
export const discard = (repo: string, body: unknown) =>
  apiFetch(`${repoBase(repo)}/git/discard`, jsonInit('POST', body))
export const gitSync = (repo: string, kind: 'pull' | 'push') =>
  apiFetch(`${repoBase(repo)}/git/${kind}`, { method: 'POST' })
