import type { CodeMap, Module } from '@/types/map'

// 矩形分层布局：每层一条等宽横带（左侧层标签列 + 右侧模块区），层自上而下按 order 排列
export const LABEL_W = 200
export const NODE_W = 240
export const NODE_H = 100
const GAP_X = 44
const BAND_PAD_X = 48
const BAND_PAD_TOP = 28
const BAND_PAD_BOTTOM = 56 // 给层间连线留出弧度空间
const BAND_GAP = 64
export const CANVAS_W = 1560

export interface BandBox {
  layerId: string
  x: number
  y: number
  width: number
  height: number
}

export interface LayoutResult {
  positions: Map<string, { x: number; y: number }>
  bands: BandBox[]
  canvasHeight: number
}

function columnsFor(count: number): number {
  if (count <= 5) return count
  return Math.ceil(count / Math.ceil(count / 5)) // 超过 5 个时折行，每行最多 5 个
}

export function layoutMap(map: CodeMap): LayoutResult {
  const layers = [...map.layers].sort((a, b) => a.order - b.order)
  const positions = new Map<string, { x: number; y: number }>()
  const bands: BandBox[] = []

  let y = 0
  for (const layer of layers) {
    const mods = map.modules.filter((m) => m.layer === layer.id)
    const cols = Math.max(1, columnsFor(mods.length))
    const rows = Math.ceil(mods.length / cols)

    const gridW = cols * NODE_W + (cols - 1) * GAP_X
    const bandH = BAND_PAD_TOP + rows * NODE_H + (rows - 1) * GAP_X + BAND_PAD_BOTTOM

    // 模块在横带内整体居中
    const startX = LABEL_W + BAND_PAD_X + Math.max(0, (CANVAS_W - LABEL_W - BAND_PAD_X * 2 - gridW) / 2)

    mods.forEach((mod, i) => {
      const row = Math.floor(i / cols)
      const col = i % cols
      positions.set(mod.id, {
        x: startX + col * (NODE_W + GAP_X),
        y: y + BAND_PAD_TOP + row * (NODE_H + GAP_X),
      })
    })

    bands.push({ layerId: layer.id, x: 0, y, width: CANVAS_W, height: bandH })
    y += bandH + BAND_GAP
  }

  return { positions, bands, canvasHeight: y - BAND_GAP }
}

export function healthColor(score: number): string {
  if (score >= 75) return '#10b981' // Healthy 绿
  if (score >= 60) return '#f59e0b' // Warning 琥珀
  return '#ef4444' // Error 红
}

export function healthLabel(score: number): string {
  if (score >= 75) return 'Healthy'
  if (score >= 60) return 'Warning'
  return 'Error'
}

export function dependentsOf(map: CodeMap, mod: Module): string[] {
  return map.edges.filter((e) => e.to === mod.id).map((e) => e.from)
}
