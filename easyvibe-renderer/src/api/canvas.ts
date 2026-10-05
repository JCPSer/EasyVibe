// 域 client：地图域（map/growth/progress/freshness/子图/归纳/巡检/健康历史）。
// 消费者：map-canvas（Canvas/DetailPanel/IssuesList/GitPage 图表）与 gate 引导。
import { apiFetch, repoBase } from './core'

export const mapProgress = (repo: string) => apiFetch(`${repoBase(repo)}/progress`)
export const freshness = (repo: string) => apiFetch(`${repoBase(repo)}/freshness`)
export const reinduce = (repo: string) => apiFetch(`${repoBase(repo)}/reinduce`, { method: 'POST' })
export const growth = (repo: string) => apiFetch(`${repoBase(repo)}/growth`)
export const submap = (repo: string, moduleId: string) =>
  apiFetch(`${repoBase(repo)}/modules/${encodeURIComponent(moduleId)}`)
export const analyzeSubmap = (repo: string, moduleId: string) =>
  apiFetch(`${repoBase(repo)}/modules/${encodeURIComponent(moduleId)}/analyze-submap`, { method: 'POST' })
export const patrol = (repo: string) => apiFetch(`${repoBase(repo)}/patrol`, { method: 'POST' })
export const healthHistory = (repo: string, moduleId: string) =>
  apiFetch(`${repoBase(repo)}/modules/${encodeURIComponent(moduleId)}/health-history`)
