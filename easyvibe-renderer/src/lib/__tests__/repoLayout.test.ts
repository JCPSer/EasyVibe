// 防回胀守卫（R8）：照抄后端 module_size_guard.rs / arch_guard.rs 范式——
// 只读源码文本（不 import 被测符号），毫秒级进常规 vitest run。
// 断言组：① LOC 上限 ② 反向/横向 import 禁止 ③ 清单快照双向全等 ④ App hooks 序。
// @ts-expect-error vitest 运行时支持 node:fs（tsconfig 无 node 类型）
import { readFileSync, readdirSync } from 'node:fs'
// @ts-expect-error vitest 运行时支持 node:path
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

// tsconfig.app.json 未含 node 类型（types: [vite/client]），此处最小声明
// @ts-expect-error vitest 运行时提供
const cwd: string = process.cwd()

const read = (rel: string) => readFileSync(resolve(cwd, rel), 'utf-8')
const loc = (rel: string) => read(rel).split('\n').length
const ls = (relDir: string, exts: string[]) =>
  readdirSync(resolve(cwd, relDir)).filter((f: string) => exts.some((e) => f.endsWith(e)))

// ---------------- 冻结快照（值取自方案 §0.3，源码重采于 2026-10-05） ----------------
const FROZEN_PAGE_IDS = [
  'map', 'modules', 'deps', 'drift', 'health', 'workbench', 'tasks', 'runs', 'usage',
  'todo', 'review', 'changes', 'git', 'kb-docs', 'kb-decisions', 'kb-apis', 'settings',
]
const FROZEN_WS_EVENTS = [
  'map.changed', 'growth.event', 'session.statusChanged', 'queue.changed', 'session.output',
  'patrol.finished', 'freshness.changed', 'task.contractAlert', 'task.contractViolated', 'task.statusChanged',
]
const FROZEN_CANVAS_PROPS = [
  'map', 'backendRepo', 'onPatrollingChange', 'panelOpen', 'onPanelOpenChange', 'panelWidth',
  'onPanelWidthChange', 'viewRequest', 'onViewRequestConsumed', 'guide', 'onTaskCreated', 'agentReady',
  'onChatAbout', 'onGoWorkbench', 'onInspectEdge', 'onOpenRuns', 'onOpenDeps', 'lensRequest',
  'onLensRequestConsumed', 'dark',
]

// ---------------- 采集器（可复现，仅读源码） ----------------
function pageIdKeys(): string[] {
  const src = read('src/components/AppShell.tsx')
  const m = src.match(/export type PageId =([\s\S]*?)\n\n/)
  if (!m) throw new Error('AppShell.tsx 未找到 PageId 联合类型')
  return [...m[1].matchAll(/'([^']+)'/g)].map((x) => x[1])
}

function routeKeys(): string[] {
  const src = read('src/pages/routes.tsx')
  const start = src.indexOf('export function buildPages(')
  const ret = src.indexOf('return {', start)
  const seg = src.slice(ret, src.indexOf('\n  }\n}', ret))
  return [...seg.matchAll(/^ {4}'?([A-Za-z0-9-]+)'?:\s/gm)].map((x) => x[1])
}

function wsEvents(): string[] {
  const src = read('src/lib/ws.ts')
  return [...new Set([...src.matchAll(/msg\.name === '([^']+)'/g)].map((x) => x[1]))]
}

function canvasProps(): string[] {
  const src = read('src/components/canvas/Canvas.tsx')
  const m = src.match(/export function Canvas\(\{([\s\S]*?)\}: \{/)
  if (!m) throw new Error('Canvas.tsx 未找到 props 解构')
  return m[1]
    .split(',')
    .map((s: string) => s.trim())
    .filter(Boolean)
    .map((s: string) => s.split(/[\s=:]/)[0])
    .filter(Boolean)
}

const sorted = (a: string[]) => [...a].sort()

describe('R8 防回胀守卫 · 断言组 1：LOC 上限（行数为权威）', () => {
  it('App.tsx ≤ 400（只留页面路由 + 数据源装配）', () => {
    expect(loc('src/App.tsx')).toBeLessThanOrEqual(400)
  })
  it('Canvas.tsx ≤ 700', () => {
    expect(loc('src/components/canvas/Canvas.tsx')).toBeLessThanOrEqual(700)
  })
  it('components/canvas 其余文件 ≤ 500', () => {
    for (const f of ls('src/components/canvas', ['.ts', '.tsx'])) {
      if (f === 'Canvas.tsx') continue
      expect(loc(`src/components/canvas/${f}`), f).toBeLessThanOrEqual(500)
    }
  })
  it('gate / shell / overlays ≤ 300；hooks ≤ 300；ws.ts ≤ 300；routes.tsx ≤ 400', () => {
    for (const dir of ['src/components/gate', 'src/components/shell', 'src/components/overlays']) {
      for (const f of ls(dir, ['.tsx'])) expect(loc(`${dir}/${f}`), `${dir}/${f}`).toBeLessThanOrEqual(300)
    }
    for (const f of ls('src/hooks', ['.ts'])) expect(loc(`src/hooks/${f}`), f).toBeLessThanOrEqual(300)
    expect(loc('src/lib/ws.ts')).toBeLessThanOrEqual(300)
    expect(loc('src/pages/routes.tsx')).toBeLessThanOrEqual(400)
  })
})

describe('R8 防回胀守卫 · 断言组 2：反向/横向 import 禁止', () => {
  const pageImport = /from '@\/components\/[A-Za-z]*Page'/
  const appImport = /from '@\/App'/

  it('components/canvas/** 不得 import App 或页面组件', () => {
    for (const f of ls('src/components/canvas', ['.ts', '.tsx'])) {
      const src = read(`src/components/canvas/${f}`)
      expect(src, `${f} 反向 import App`).not.toMatch(appImport)
      expect(src, `${f} 反向 import 页面组件`).not.toMatch(pageImport)
      expect(src, `${f} 横向 import gate`).not.toMatch(/from '@\/components\/gate/)
    }
  })
  it('components/gate/** 不得 import App / canvas / 页面组件', () => {
    for (const f of ls('src/components/gate', ['.tsx'])) {
      const src = read(`src/components/gate/${f}`)
      expect(src, `${f}`).not.toMatch(appImport)
      expect(src, `${f}`).not.toMatch(/from '@\/components\/canvas/)
      expect(src, `${f}`).not.toMatch(pageImport)
    }
  })
  it('新增 hooks / lib/ws.ts 不得 import App 或页面组件', () => {
    const targets = [
      'src/hooks/useAgentState.ts', 'src/hooks/useOnboarding.ts', 'src/hooks/useBackendConnection.ts',
      'src/hooks/useTheme.ts', 'src/hooks/useAttention.ts', 'src/hooks/usePatrol.ts',
      'src/hooks/useUiPrefs.ts', 'src/hooks/useSystemNotifications.ts', 'src/lib/ws.ts',
    ]
    for (const t of targets) {
      const src = read(t)
      expect(src, `${t} import App`).not.toMatch(appImport)
      expect(src, `${t} import 页面组件`).not.toMatch(pageImport)
    }
  })
})

describe('R8 防回胀守卫 · 断言组 3：清单快照（双向全等）', () => {
  it('PageId 键集合 == 17（AppShell ∪ routes 装配表，少一页=白屏/404，多一页=意外暴露）', () => {
    expect(sorted(pageIdKeys())).toEqual(sorted(FROZEN_PAGE_IDS))
    expect(sorted(routeKeys())).toEqual(sorted(FROZEN_PAGE_IDS))
    expect(pageIdKeys().length).toBe(17)
  })
  it('lib/ws.ts 的 msg.name 字面量集合 == 10', () => {
    expect(sorted(wsEvents())).toEqual(sorted(FROZEN_WS_EVENTS))
    expect(wsEvents().length).toBe(10)
  })
  it('Canvas props 名集合 == 20', () => {
    expect(sorted(canvasProps())).toEqual(sorted(FROZEN_CANVAS_PROPS))
    expect(canvasProps().length).toBe(20)
  })
})

describe('R8 防回胀守卫 · 断言组 4：App hooks 序（历史 P0 白屏）', () => {
  it('所有 hook 调用先于任一提前 return', () => {
    const lines = read('src/App.tsx').split('\n')
    const hookRe = /\buse[A-Z][A-Za-z0-9]*\s*\(/
    const hookLines = lines
      .map((l: string, i: number) => (hookRe.test(l) && !l.trimStart().startsWith('//') ? i + 1 : 0))
      .filter(Boolean)
    const earlyRes = [
      /^\s{2}if \(error && !backendRepo\) \{/,
      /^\s{2}if \(backendRepo && \(error \|\| !map\)\) \{/,
      /^\s{2}if \(!map\) \{/,
    ]
    const earlyLines = lines.map((l: string, i: number) => (earlyRes.some((r) => r.test(l)) ? i + 1 : 0)).filter(Boolean)
    expect(earlyLines.length).toBe(3)
    expect(Math.max(...hookLines)).toBeLessThan(Math.min(...earlyLines))
  })
})
