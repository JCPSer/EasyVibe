// 健康看板趋势图视窗逻辑（纯函数）：缩放锚点 / 平移钳制 / 档位切换 / 轴刻度格式化。
// 独立成文件是为了让 vitest 直接覆盖（HealthPage 有 componentGuard 600 行 LOC 上限，
// 视窗与图辅助逻辑全部外置，图内只留渲染与事件接线）。
//
// 约定：数据数组恒为旧→新；视窗 {start, count} 为半开区间 [start, start+count)。
import { toMs } from '@/shared/logic/diffStat'

export interface TrendWindow {
  start: number
  count: number
}

/** 缩放下限：少于该点数不再放大（再密 x 轴刻度与悬停热区就挤没了）。总数不足时随总数。 */
export const MIN_POINTS = 5
/** 初始视窗：最近 INITIAL_COUNT 条（数据不足则全部）。 */
export const INITIAL_COUNT = 30

/** 初始视窗 = 最近 INITIAL_COUNT 条，不足则全部。 */
export const initialWindow = (total: number): TrendWindow => {
  const n = Math.max(0, Math.floor(total))
  const count = Math.min(INITIAL_COUNT, n)
  return { start: n - count, count }
}

/** 钳制：count ∈ [min(MIN_POINTS, total), total]，start ∈ [0, total - count]。total<0 按 0 处理。 */
export const clampWindow = (w: TrendWindow, total: number): TrendWindow => {
  const n = Math.max(0, Math.floor(total))
  const minCount = Math.min(MIN_POINTS, n)
  const count = Math.min(Math.max(Math.floor(w.count), minCount), n)
  const start = Math.min(Math.max(Math.floor(w.start), 0), n - count)
  return { start, count }
}

/** 缩放：anchor 为光标在视窗内的相对位置（0..1，0 = 视窗最左/最旧），
 *  factor > 1 放大视窗（点数变多），< 1 缩小（点数变少）——锚点处的数据索引保持不动。 */
export const zoomWindow = (w: TrendWindow, total: number, anchor: number, factor: number): TrendWindow => {
  const n = Math.max(0, Math.floor(total))
  if (n <= 1 || factor <= 0) return clampWindow(w, n)
  const a = Math.min(Math.max(anchor, 0), 1)
  // 先钳 count（含下限），再用钳后的 count 反推 start——锚点在下限附近偏差最小
  const count = clampWindow({ start: w.start, count: Math.round(w.count * factor) }, n).count
  const start = Math.round(w.start + a * w.count - a * count)
  return clampWindow({ start, count }, n)
}

/** 平移：delta 为索引位移（正 = 视窗向旧数据方向移），越界即钳制。 */
export const panWindow = (w: TrendWindow, total: number, delta: number): TrendWindow =>
  clampWindow({ start: w.start + delta, count: w.count }, total)

/** 档位切换：preset = 最近 N 条；'all' = 全部。 */
export const presetWindow = (total: number, preset: number | 'all'): TrendWindow => {
  const n = Math.max(0, Math.floor(total))
  const count = preset === 'all' ? n : Math.min(Math.floor(preset), n)
  return { start: n - count, count }
}

/** 当前视窗是否正落在某档位上（快捷按钮高亮用）。 */
export const isPresetActive = (w: TrendWindow, total: number, preset: number | 'all'): boolean => {
  const p = presetWindow(total, preset)
  return w.start === p.start && w.count === p.count
}

/** x 轴刻度格式化（M/D）：startedAt 可能是 epoch 毫秒串——先经 toMs 归一，切串会出乱码（ui-test P2）。 */
export const fmtDay = (iso: string): string => {
  const t = toMs(iso)
  if (t === null) return '—'
  const d = new Date(t)
  return `${d.getMonth() + 1}/${d.getDate()}`
}

export interface YDomain {
  min: number
  max: number
  ticks: number[]
}

/** 动态 Y 轴：按视窗内数据的 min/max 自适应（分数集中 70–90 时 0–100 固定刻度会把趋势压平）。
 *  边距取量程的 15%（至少 ±4 分），钳制在 [0, 100]；无数据回落 0–100。
 *  刻度取「好看步长」（1/2/5/10/20/25/50）生成 4–6 格。 */
export function yDomainOf(values: (number | null | undefined)[]): YDomain {
  const vs = values.filter((v): v is number => typeof v === 'number' && Number.isFinite(v))
  if (!vs.length) return { min: 0, max: 100, ticks: [0, 25, 50, 75, 100] }
  const lo = Math.min(...vs)
  const hi = Math.max(...vs)
  const pad = Math.max(4, Math.ceil((hi - lo) * 0.15))
  let min = Math.max(0, Math.floor(lo - pad))
  let max = Math.min(100, Math.ceil(hi + pad))
  if (max - min < 10) {
    // 数据几乎水平（单点/同分）：保证至少 10 分量程，趋势线不顶满
    const mid = Math.round((min + max) / 2)
    min = Math.max(0, mid - 5)
    max = Math.min(100, min + 10)
  }
  const span = max - min
  const step = [1, 2, 5, 10, 20, 25, 50].find((s) => Math.ceil(span / s) <= 5) ?? 50
  const ticks: number[] = []
  for (let v = Math.ceil(min / step) * step; v <= max; v += step) ticks.push(v)
  return { min, max, ticks }
}
