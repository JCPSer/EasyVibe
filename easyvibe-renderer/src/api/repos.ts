// 域 client：仓库（注册/注销/健康）+ 仓库级聚合（用量/看板/巡检历史/运行总览）。
// 消费者：console-ui 装配壳（WelcomePage/UsagePage/HealthPage/DriftPage/RunsPage/SessionBubble）。
import { apiFetch, repoBase, jsonInit } from './core'

export const listRepos = () => apiFetch('/repos')
export const addRepo = (path: string) => apiFetch('/repos', jsonInit('POST', { path }))
export const removeRepo = (id: string, wipe = false) =>
  apiFetch(`${repoBase(id)}${wipe ? '?wipe=true' : ''}`, { method: 'DELETE' })
export const repoMap = (repo: string) => apiFetch(`${repoBase(repo)}/map`)
export const usage = (repo: string, days = 30) => apiFetch(`${repoBase(repo)}/usage?days=${days}`)
export const healthDashboard = (repo: string) => apiFetch(`${repoBase(repo)}/health-dashboard`)
export const agentSessions = (repo: string) => apiFetch(`${repoBase(repo)}/agent-sessions`)
export const patrolRuns = (repo: string, keep?: number) =>
  apiFetch(`${repoBase(repo)}/patrol-runs${keep != null ? `?keep=${keep}` : ''}`)
export const prunePatrolRuns = (repo: string, keep: number) =>
  apiFetch(`${repoBase(repo)}/patrol-runs?keep=${keep}`, { method: 'DELETE' })
export const sessionsOverview = () => apiFetch('/sessions/overview')
export const eventsSummary = (repo: string) => apiFetch(`${repoBase(repo)}/events/summary`)
export const ingestEvent = (repo: string, body: unknown) =>
  apiFetch(`${repoBase(repo)}/events`, jsonInit('POST', body))
