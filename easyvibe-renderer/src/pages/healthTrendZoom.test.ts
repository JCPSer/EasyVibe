// 视窗逻辑单测：缩放锚点不动点、平移钳制不越界、档位切换。
import { describe, expect, it } from 'vitest'
import {
  clampWindow, initialWindow, isPresetActive, MIN_POINTS, panWindow, presetWindow, yDomainOf, zoomWindow,
} from './healthTrendZoom'

describe('healthTrendZoom · 初始视窗', () => {
  it('总数 100 取最近 30 条（[70, 30)）', () => {
    expect(initialWindow(100)).toEqual({ start: 70, count: 30 })
  })
  it('总数不足 30 则全部（start=0）', () => {
    expect(initialWindow(12)).toEqual({ start: 0, count: 12 })
  })
  it('总数 0 不炸（start=0, count=0）', () => {
    expect(initialWindow(0)).toEqual({ start: 0, count: 0 })
  })
})

describe('healthTrendZoom · 钳制', () => {
  it('count 下限 MIN_POINTS；start 已在界内则保持', () => {
    expect(clampWindow({ start: 90, count: 2 }, 100)).toEqual({ start: 90, count: MIN_POINTS })
  })
  it('count 上限 total，start 不越左界', () => {
    expect(clampWindow({ start: -10, count: 500 }, 60)).toEqual({ start: 0, count: 60 })
  })
  it('total 小于 MIN_POINTS 时下限随 total（不能再放大）', () => {
    expect(clampWindow({ start: 0, count: 1 }, 3)).toEqual({ start: 0, count: 3 })
  })
})

describe('healthTrendZoom · 缩放锚点', () => {
  // 视窗 [40, 20)：数据索引 40..59；anchor=0.5 的锚点索引 = 40 + 0.5*20 = 50
  it('锚点索引保持不动：factor 0.5 → 视窗 [45, 10)，50 仍在正中', () => {
    const w = zoomWindow({ start: 40, count: 20 }, 100, 0.5, 0.5)
    expect(w).toEqual({ start: 45, count: 10 })
    expect(w.start + 0.5 * w.count).toBe(50)
  })
  it('左缘锚点（anchor=0）：视窗向右扩展，起点不动', () => {
    const w = zoomWindow({ start: 40, count: 20 }, 100, 0, 2)
    expect(w.start).toBe(40)
    expect(w.count).toBe(40)
  })
  it('右缘锚点（anchor=1）：视窗向左扩展，终点不动', () => {
    const w = zoomWindow({ start: 40, count: 20 }, 100, 1, 2)
    expect(w.start + w.count).toBe(60)
    expect(w.count).toBe(40)
  })
  it('放大到触顶：count 被钳到 total，start 归零', () => {
    const w = zoomWindow({ start: 10, count: 20 }, 30, 0.5, 10)
    expect(w).toEqual({ start: 0, count: 30 })
  })
  it('缩到下限：count 钳到 MIN_POINTS，锚点尽量保持（±1 内）', () => {
    const w = zoomWindow({ start: 40, count: 20 }, 100, 0.5, 0.01)
    expect(w.count).toBe(MIN_POINTS)
    expect(Math.abs(w.start + 0.5 * w.count - 50)).toBeLessThanOrEqual(1)
  })
  it('总数 ≤1 或非法 factor 原样钳制返回', () => {
    expect(zoomWindow({ start: 0, count: 1 }, 1, 0.5, 2)).toEqual({ start: 0, count: 1 })
    expect(zoomWindow({ start: 40, count: 20 }, 100, 0.5, 0)).toEqual({ start: 40, count: 20 })
  })
})

describe('healthTrendZoom · 平移', () => {
  it('正向平移受右界钳制', () => {
    expect(panWindow({ start: 80, count: 20 }, 100, 10)).toEqual({ start: 80, count: 20 })
  })
  it('负向平移受左界钳制', () => {
    expect(panWindow({ start: 5, count: 20 }, 100, -10)).toEqual({ start: 0, count: 20 })
  })
  it('正常平移原样生效', () => {
    expect(panWindow({ start: 40, count: 20 }, 100, -5)).toEqual({ start: 35, count: 20 })
  })
})

describe('healthTrendZoom · 档位', () => {
  it('最近 20 / 最近 50：贴右缘', () => {
    expect(presetWindow(100, 20)).toEqual({ start: 80, count: 20 })
    expect(presetWindow(100, 50)).toEqual({ start: 50, count: 50 })
  })
  it('不足档位数的总数 → 全部', () => {
    expect(presetWindow(30, 50)).toEqual({ start: 0, count: 30 })
  })
  it('全部档位 = start 0 + count total', () => {
    expect(presetWindow(77, 'all')).toEqual({ start: 0, count: 77 })
  })
  it('isPresetActive 只有精确落在档位上才高亮', () => {
    expect(isPresetActive({ start: 80, count: 20 }, 100, 20)).toBe(true)
    expect(isPresetActive({ start: 70, count: 30 }, 100, 20)).toBe(false)
    expect(isPresetActive({ start: 0, count: 100 }, 100, 'all')).toBe(true)
  })
})

describe('healthTrendZoom · 动态 Y 轴', () => {
  it('无数据回落 0–100 固定五格', () => {
    expect(yDomainOf([])).toEqual({ min: 0, max: 100, ticks: [0, 25, 50, 75, 100] })
    expect(yDomainOf([null, undefined, NaN])).toEqual({ min: 0, max: 100, ticks: [0, 25, 50, 75, 100] })
  })
  it('集中分区间的自适应：70–90 → 域收紧且含数据端点余量', () => {
    const d = yDomainOf([74, 78, 89, 82, null])
    expect(d.min).toBeLessThanOrEqual(74 - 2)
    expect(d.max).toBeGreaterThanOrEqual(89 + 2)
    expect(d.max - d.min).toBeLessThan(40) // 不再是 0–100 大片空白
    expect(d.ticks[0]).toBeGreaterThanOrEqual(d.min)
    expect(d.ticks[d.ticks.length - 1]).toBeLessThanOrEqual(d.max)
    expect(d.ticks.length).toBeGreaterThanOrEqual(3)
  })
  it('域钳制在 0–100 内', () => {
    const d = yDomainOf([1, 3, 5])
    expect(d.min).toBeGreaterThanOrEqual(0)
    expect(d.max).toBeLessThanOrEqual(100)
    const hi = yDomainOf([95, 98, 100])
    expect(hi.max).toBeLessThanOrEqual(100)
  })
  it('水平数据（同分）至少保 10 分量程', () => {
    const d = yDomainOf([80, 80, 80])
    expect(d.max - d.min).toBeGreaterThanOrEqual(10)
  })
  it('刻度步长好看（整除域端点附近）', () => {
    const d = yDomainOf([70, 90])
    const steps = d.ticks.slice(1).map((v, i) => v - d.ticks[i])
    expect(new Set(steps).size).toBe(1) // 等距
    expect([1, 2, 5, 10, 20, 25, 50]).toContain(steps[0])
  })
})
