import { useCallback, useEffect, useState } from 'react'
import { putSetting, listSettings } from '@/api/settings'

/**
 * 每项目 UI 偏好：右栏开合/宽度随项目持久化（M4-1 状态持久化，防刷新丢位置）+ 通用 ui.* 写入。
 * 页签不恢复（2026-10-03 用户裁定）：切仓库固定落架构地图。
 */
export function useUiPrefs(backendRepo: string | null, resetToMap: () => void) {
  const [panelOpen, setPanelOpen] = useState(true)
  const [panelWidth, setPanelWidth] = useState(340)

  const saveUiPref = useCallback(
    (key: string, value: unknown) => {
      if (!backendRepo) return
      putSetting({ scope: backendRepo, key, value }).catch(() => {})
    },
    [backendRepo],
  )

  useEffect(() => {
    if (!backendRepo) return
    resetToMap() // 切仓库先回架构地图，数据到达前不闪旧页
    let stale = false
    listSettings(backendRepo)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { key: string; value: unknown }[] } | null) => {
        if (stale || !d?.data) return
        const get = (k: string) => d.data!.find((i) => i.key === k)?.value
        const w = get('ui.panelWidth')
        if (typeof w === 'number' && Number.isFinite(w)) setPanelWidth(Math.min(560, Math.max(340, w)))
        const po = get('ui.panelOpen')
        if (typeof po === 'boolean') setPanelOpen(po)
      })
      .catch(() => {})
    return () => {
      stale = true
    }
  }, [backendRepo, resetToMap])

  const handlePanelOpenChange = useCallback(
    (open: boolean) => {
      setPanelOpen(open)
      saveUiPref('ui.panelOpen', open)
    },
    [saveUiPref],
  )

  const handlePanelWidthChange = useCallback(
    (w: number) => {
      setPanelWidth(w)
      saveUiPref('ui.panelWidth', w)
    },
    [saveUiPref],
  )

  return { saveUiPref, panelOpen, panelWidth, handlePanelOpenChange, handlePanelWidthChange }
}
