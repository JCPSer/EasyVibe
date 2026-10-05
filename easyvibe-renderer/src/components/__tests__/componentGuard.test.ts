// 防回胀守卫（前端 R13）：顶层业务组件 + map-canvas 面板的体量与方向门禁。
// 范式同 repoLayout.test.ts：只读源码文本（不 import 被测符号），毫秒级进常规 vitest run。
//
// 口径（方案 §R1/R2/R3/R7）：
//   ① 归属快照：`src/components/*.tsx`（单层）必须显式登记为 console-ui 或 map-canvas——
//      新增顶层组件不登记即失败，杜绝"新增文件自动逃逸守卫"。
//   ② 体量上限：默认 ≤600；拆分前存量越线者进 LEGACY_FROZEN 锁现值（只降不升、禁止新增），
//      拆分完成后从表内移除并回落 600。
//   ③ 方向：顶层组件不得反向 import App / pages/routes；map-canvas 面板不得 import console-ui 的 *Page。
//   ④ 子目录守卫（R13-B1 补齐）：本次 god 组件拆分产物落点 `components/chat|settings|taskworkflow/**`
//      曾整体逃逸旧守卫（旧断言只扫单层）——拆后薄壳（≤600）背后的实现体可无提示回胀。
//      故对三目录**递归**做「文件集快照双向全等 + LOC≤600 + 反向 import」，并以 KNOWN_SUBDIRS
//      登记表锁死 `src/components/` 顶层子目录集合：新增目录不登记即失败（同① 的目录级版本）。
//
// 口径：LOC = split('\n').length（文件以换行结尾时 = wc -l + 1，比 wc 口径大 1，勿混淆）；
//       阈值 600 为全仓「职责域」统一上限（与后端 crate 子模块守卫一致）。
// @ts-expect-error vitest 运行时支持 node:fs（tsconfig 无 node 类型）
import { readFileSync, readdirSync } from 'node:fs'
// @ts-expect-error vitest 运行时支持 node:path
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

// @ts-expect-error vitest 运行时提供
const cwd: string = process.cwd()

const read = (rel: string) => readFileSync(resolve(cwd, rel), 'utf-8')
const loc = (rel: string) => read(rel).split('\n').length
const lsTopComponents = () =>
  readdirSync(resolve(cwd, 'src/components'))
    .filter((f: string) => f.endsWith('.tsx'))
    .map((f: string) => f.replace(/\.tsx$/, ''))

// 递归收集 `src/components/<subdir>/**` 的 .ts/.tsx，返回相对 `src/components/` 的路径。
const lsRecursive = (subdir: string): string[] => {
  const out: string[] = []
  const walk = (rel: string) => {
    for (const e of readdirSync(resolve(cwd, rel), { withFileTypes: true })) {
      const child = `${rel}/${e.name}`
      if (e.isDirectory()) walk(child)
      else if (/\.tsx?$/.test(e.name)) out.push(child.replace(/^src\/components\//, ''))
    }
  }
  walk(`src/components/${subdir}`)
  return out
}

// ---------------- 归属快照（来源：.easyvibe/map/map.json modules.*.files，2026-10-05） ----------------
const CONSOLE_UI_COMPONENTS = [
  'AppShell', 'ChangesPage', 'DepsPage', 'DriftPage', 'GitPage', 'HealthPage', 'ModulesPage',
  'OnboardingChecklist', 'PlaceholderPage', 'RunsPage', 'SessionBubble', 'SettingsPanel',
  'StagePipeline', 'SuggestPanel', 'TaskAdminButtons', 'TaskBoardPage', 'TaskGovernancePage',
  'TaskPage', 'TaskWorkflowPage', 'ThemeToggle', 'UsagePage', 'ViewsPanel', 'WelcomePage',
  'WindowControls', 'WorkbenchPage',
]
const MAP_CANVAS_COMPONENTS = [
  'ModuleNode', 'BandNode', 'SubmoduleNode', 'ExpandedModuleNode', 'DetailPanel', 'PanelChat',
  'QuickAsk', 'AnswerCards', 'IssuesList', 'TaskFormPanel', 'ChatPanel', 'MarkdownMessage',
]

const DEFAULT_CEILING = 600

// LEGACY ratchet：拆分前存量 >600 的顶层组件锁当前值（只降不升、禁止新增）。
// god 组件（TaskWorkflowPage/ChatPanel/SettingsPanel/QuickAsk）拆到 ≤600 后必须从本表删除。
const LEGACY_FROZEN: Record<string, number> = {
  GitPage: 878,
  RunsPage: 667,
  DetailPanel: 628,
}

const ALL_COMPONENTS = [...CONSOLE_UI_COMPONENTS, ...MAP_CANVAS_COMPONENTS]
const ceilingOf = (name: string) => LEGACY_FROZEN[name] ?? DEFAULT_CEILING
const sorted = (a: string[]) => [...a].sort()

// ---------------- 子目录守卫（B1 补齐，来源：本轮 god 组件拆分产物，2026-10-05） ----------------
// 三目录合计 25 文件 / 3518 行：chat（map-canvas 对话壳）、settings / taskworkflow（console-ui）。
// 旧守卫（本文件单层断言 + repoLayout R8 的 canvas/gate/shell/overlays）均不覆盖它们，
// 导致拆后薄壳符合阈值、实现体却零保护。以下文件集为**双向全等快照**：增删文件必须同步改表。
const GUARDED_SUBDIRS = ['chat', 'settings', 'taskworkflow']

const SUBDIR_FILES: Record<string, string[]> = {
  chat: [
    'chat/ComposerDock.tsx', 'chat/ConversationSwitcher.tsx', 'chat/HeaderActions.tsx',
    'chat/MessageStream.tsx', 'chat/QuickAskComposer.tsx', 'chat/QuickAskStream.tsx',
    'chat/types.ts', 'chat/useConversations.ts',
  ],
  settings: [
    'settings/AboutSection.tsx', 'settings/AdvancedSection.tsx', 'settings/AgentSection.tsx',
    'settings/HarnessSection.tsx', 'settings/ServicesSection.tsx',
    'settings/common.ts', 'settings/controls.tsx',
  ],
  taskworkflow: [
    'taskworkflow/DocCard.tsx', 'taskworkflow/PhaseDocReview.tsx', 'taskworkflow/StageLookback.tsx',
    'taskworkflow/diffParse.ts', 'taskworkflow/types.ts',
    'taskworkflow/stages/AnalysisStage.tsx', 'taskworkflow/stages/DiffStage.tsx',
    'taskworkflow/stages/DoneStage.tsx', 'taskworkflow/stages/ErrorStage.tsx',
    'taskworkflow/stages/ReportStage.tsx', 'taskworkflow/stages/TerminalStage.tsx',
  ],
}

// `src/components/` 顶层允许出现的子目录全集——新增目录不登记即失败，杜绝"换个地址复活"。
const KNOWN_SUBDIRS = [
  '__tests__', 'canvas', 'chat', 'gate', 'overlays', 'settings', 'shell', 'taskworkflow', 'ui',
]
const GUARDED_FILES = GUARDED_SUBDIRS.flatMap((d) => SUBDIR_FILES[d])

describe('防膨胀守卫 · 断言组 1：归属快照（双向全等）', () => {
  it('src/components/*.tsx（单层）集合 == console-ui ∪ map-canvas（新增组件必须显式登记归属）', () => {
    expect(sorted(lsTopComponents())).toEqual(sorted(ALL_COMPONENTS))
  })
})

describe('防膨胀守卫 · 断言组 2：LOC 上限（行数为权威）', () => {
  it(`每个顶层组件 ≤ 上限（默认 ${DEFAULT_CEILING}；LEGACY 锁现值）`, () => {
    for (const f of ALL_COMPONENTS) {
      expect(loc(`src/components/${f}.tsx`), `${f}.tsx`).toBeLessThanOrEqual(ceilingOf(f))
    }
  })
  it('LEGACY 表键集合 == 上限 >600 的组件集合（显式冻结，禁止静默新增）', () => {
    const over = ALL_COMPONENTS.filter((f) => ceilingOf(f) > DEFAULT_CEILING)
    expect(sorted(over)).toEqual(sorted(Object.keys(LEGACY_FROZEN)))
  })
})

describe('防膨胀守卫 · 断言组 3：反向/横向 import 禁止', () => {
  it('顶层组件不得反向 import App 或页面装配（别名与相对路径双堵）', () => {
    const reApp = /from ['"](@\/App|\.\.?\/App)['"]/
    const reRoutes = /from ['"](@\/pages\/routes|\.\.?\/pages\/routes)['"]/
    for (const f of ALL_COMPONENTS) {
      const src = read(`src/components/${f}.tsx`)
      expect(src, `${f}.tsx 反向 import App`).not.toMatch(reApp)
      expect(src, `${f}.tsx 反向 import pages/routes`).not.toMatch(reRoutes)
    }
  })
  it('map-canvas 面板不得 import console-ui 的 *Page（维持 map-canvas → console-ui 单向）', () => {
    const rePage = /from ['"](@\/components\/[A-Za-z]*Page|\.\.?\/[A-Za-z]*Page)['"]/
    for (const f of MAP_CANVAS_COMPONENTS) {
      expect(read(`src/components/${f}.tsx`), `${f}.tsx import console-ui *Page`).not.toMatch(rePage)
    }
  })
  it('ui-kit 生成物不得 import 业务面（App / pages / 顶层业务组件）', () => {
    const uiDir = 'src/components/ui'
    const files = readdirSync(resolve(cwd, uiDir)).filter((f: string) => f.endsWith('.tsx') || f.endsWith('.ts'))
    const reBusiness = /from ['"](@\/App|@\/pages\/|@\/components\/(?!ui\/))/
    for (const f of files) {
      expect(read(`${uiDir}/${f}`), `ui/${f} 引用业务层`).not.toMatch(reBusiness)
    }
  })
})

describe('防膨胀守卫 · 断言组 4：拆分产物子目录（chat / settings / taskworkflow，B1 补齐）', () => {
  it('守卫子目录文件集快照双向全等（新增/删除文件必须显式登记，防换个地址复活）', () => {
    for (const dir of GUARDED_SUBDIRS) {
      expect(sorted(lsRecursive(dir)), `${dir}/**`).toEqual(sorted(SUBDIR_FILES[dir]))
    }
  })

  it(`守卫子目录每个文件 ≤ ${DEFAULT_CEILING} 行（薄壳背后的实现体同样受限）`, () => {
    for (const rel of GUARDED_FILES) {
      expect(loc(`src/components/${rel}`), rel).toBeLessThanOrEqual(DEFAULT_CEILING)
    }
  })

  it('守卫子目录不得反向 import App / pages（维持页面装配 → 组件单向）', () => {
    const reApp = /from ['"](?:@\/App|(?:\.\.\/)+App)['"]/
    const rePages = /from ['"](?:@\/pages\/|(?:\.\.\/)+pages\/)/
    for (const rel of GUARDED_FILES) {
      const src = read(`src/components/${rel}`)
      expect(src, `${rel} 反向 import App`).not.toMatch(reApp)
      expect(src, `${rel} 反向 import pages`).not.toMatch(rePages)
    }
  })

  it('chat/**（map-canvas 对话壳）不得 import console-ui 的 *Page', () => {
    const rePage = /from ['"](?:@\/components\/[A-Za-z]*Page|(?:\.\.\/)+[A-Za-z]*Page)['"]/
    for (const rel of SUBDIR_FILES.chat) {
      expect(read(`src/components/${rel}`), `${rel} import console-ui *Page`).not.toMatch(rePage)
    }
  })

  it('src/components 顶层子目录集合 == KNOWN_SUBDIRS（禁止未登记目录逃逸守卫）', () => {
    const entries: Array<{ name: string; isDirectory: () => boolean }> =
      readdirSync(resolve(cwd, 'src/components'), { withFileTypes: true })
    const dirs = entries.filter((e) => e.isDirectory()).map((e) => e.name)
    expect(sorted(dirs)).toEqual(sorted(KNOWN_SUBDIRS))
  })
})
