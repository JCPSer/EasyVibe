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
  // 展开模块的容器盒与子节点相对坐标（未展开则无此项）
  blocks: Map<string, { x: number; y: number; width: number; height: number; childPositions: Map<string, { x: number; y: number }>; childOrder: string[] }>
}

// 子模块卡片尺寸与展开容器参数
export const SUB_W = 226
export const SUB_H = 96
const BLOCK_PAD = 20
const BLOCK_HEADER = 48
const BLOCK_COLS = 3

export interface ExpandedMeta {
  ids: string[] // 子模块 id；加载中时为骨架占位 id
  loading: boolean
}

export function layoutMap(map: CodeMap, expanded?: Map<string, ExpandedMeta>): LayoutResult {
  const layers = [...map.layers].sort((a, b) => a.order - b.order)
  const positions = new Map<string, { x: number; y: number }>()
  const bands: BandBox[] = []
  const blocks: LayoutResult['blocks'] = new Map()

  const blockSize = (count: number) => {
    const rows = Math.max(1, Math.ceil(Math.max(count, 1) / BLOCK_COLS))
    return {
      width: BLOCK_COLS * SUB_W + (BLOCK_COLS - 1) * GAP_X + BLOCK_PAD * 2,
      height: BLOCK_HEADER + rows * SUB_H + (rows - 1) * GAP_X + BLOCK_PAD,
      rows,
    }
  }

  let y = 0
  for (const layer of layers) {
    const mods = map.modules.filter((m) => m.layer === layer.id)
    // 打包：展开的模块独占一行，普通模块按行填充（每行最多 5 列）
    const rows: { items: { mod: Module; expanded: boolean }[] }[] = []
    let cur: { mod: Module; expanded: boolean }[] = []
    let curCols = 0
    for (const mod of mods) {
      const isExp = expanded?.has(mod.id) ?? false
      if (isExp) {
        if (cur.length) { rows.push({ items: cur }); cur = [] }
        rows.push({ items: [{ mod, expanded: true }] })
        curCols = 0
      } else {
        if (curCols >= 5) { rows.push({ items: cur }); cur = []; curCols = 0 }
        cur.push({ mod, expanded: false })
        curCols += 1
      }
    }
    if (cur.length) rows.push({ items: cur })

    let bandH = BAND_PAD_TOP + BAND_PAD_BOTTOM
    let innerY = y + BAND_PAD_TOP
    for (const row of rows) {
      const exp = row.items[0].expanded ? row.items[0] : null
      if (exp) {
        const meta = expanded!.get(exp.mod.id)!
        const { width, height } = blockSize(meta.ids.length)
        positions.set(exp.mod.id, { x: LABEL_W + BAND_PAD_X, y: innerY })
        const childPositions = new Map<string, { x: number; y: number }>()
        meta.ids.forEach((sid, i) => {
          const r = Math.floor(i / BLOCK_COLS)
          const c = i % BLOCK_COLS
          childPositions.set(sid, { x: BLOCK_PAD + c * (SUB_W + GAP_X), y: BLOCK_HEADER + r * (SUB_H + GAP_X) })
        })
        blocks.set(exp.mod.id, { x: LABEL_W + BAND_PAD_X, y: innerY, width, height, childPositions, childOrder: meta.ids })
        innerY += height + GAP_X
        bandH += height + GAP_X
      } else {
        const n = row.items.length
        const gridW = n * NODE_W + (n - 1) * GAP_X
        const startX = LABEL_W + BAND_PAD_X + Math.max(0, (CANVAS_W - LABEL_W - BAND_PAD_X * 2 - gridW) / 2)
        row.items.forEach(({ mod }, i) => {
          positions.set(mod.id, { x: startX + i * (NODE_W + GAP_X), y: innerY })
        })
        innerY += NODE_H + GAP_X
        bandH += NODE_H + GAP_X
      }
    }
    bandH -= GAP_X

    bands.push({ layerId: layer.id, x: 0, y, width: CANVAS_W, height: bandH })
    y += bandH + BAND_GAP
  }

  return { positions, bands, canvasHeight: y - BAND_GAP, blocks }
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
