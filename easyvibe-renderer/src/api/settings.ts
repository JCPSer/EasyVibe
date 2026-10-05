// 域 client：设置/密钥/harness 自定义层/诊断。
// 消费者：settings-ui（SettingsPanel/HarnessSection/AgentSection）与 runtime/useUiPrefs。
import { apiFetch, jsonInit } from './core'

export const listSettings = (scope: string) => apiFetch(`/settings?scope=${encodeURIComponent(scope)}`)
export const putSetting = (body: unknown) => apiFetch('/settings/set', jsonInit('PUT', body))
export const deleteSetting = (scope: string, key: string) =>
  apiFetch(`/settings/${encodeURIComponent(scope)}/${encodeURIComponent(key)}`, { method: 'DELETE' })
export const harness = () => apiFetch('/harness')
export const customFiles = () => apiFetch('/harness/custom/files')
export const customFile = (path: string, init?: RequestInit) =>
  apiFetch(`/harness/custom/file?path=${encodeURIComponent(path)}`, init)
export const putCustomFile = (body: unknown) => apiFetch('/harness/custom/file', jsonInit('PUT', body))
export const toggleCustomFile = (body: unknown) => apiFetch('/harness/custom/toggle', jsonInit('PUT', body))
export const generateCustom = (body: unknown) => apiFetch('/harness/custom/generate', jsonInit('POST', body))
export const diagnostics = () => apiFetch('/diagnostics')
