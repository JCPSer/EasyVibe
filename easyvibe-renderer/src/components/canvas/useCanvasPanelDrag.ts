import { useCallback, useState } from 'react'
import type { PanelTab } from '@/components/DetailPanel'

/**
 * 右栏页签与拖拽调宽（画布私有编排）。
 * 从 Canvas 抽出：仅移动 state/回调声明位置，依赖数组与行为逐字保持。
 */
export function useCanvasPanelDrag(panelWidth: number, onPanelWidthChange: (w: number) => void) {
  const [tab, setTab] = useState<PanelTab>('detail')
  // 改进#4：右栏可调宽（默认 340–560；对话页签放宽到 720，D1-C）
  const handleTabChange = useCallback(
    (t: PanelTab) => {
      setTab(t)
      // 离开对话页签时若宽度超出默认上限，收回（避免宽栏压窄其他页签内容）
      if (t !== 'chat') onPanelWidthChange(Math.min(panelWidth, 560))
    },
    [onPanelWidthChange, panelWidth],
  )
  const startPanelDrag = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault()
      const startX = e.clientX
      const startW = panelWidth
      const maxW = tab === 'chat' ? 720 : 560
      const onMove = (ev: MouseEvent) => onPanelWidthChange(Math.min(maxW, Math.max(340, startW + (startX - ev.clientX))))
      const onUp = () => {
        window.removeEventListener('mousemove', onMove)
        window.removeEventListener('mouseup', onUp)
      }
      window.addEventListener('mousemove', onMove)
      window.addEventListener('mouseup', onUp)
    },
    [panelWidth, tab, onPanelWidthChange],
  )
  return { tab, setTab, handleTabChange, startPanelDrag }
}
