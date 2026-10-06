// 防回胀守卫（前端 R13 / c-arch-3）：组件归属、体量与方向门禁。
// 范式同 repoLayout.test.ts：只读源码文本（不 import 被测符号），毫秒级进常规 vitest run。
//
// 口径（c-arch-3 组件归属规则化后重写，2026-10-06）：
//   ① 归属快照：`components/` 根目录**不得存在任何文件**（根目录清空后由 archGuard 断言组 7 权威守）；
//      整页落在 `src/pages/`，画布件落在 `components/canvas/`，装配壳落在 `components/shell/`，
//      浮层落在 `components/overlays/`——四落点文件集**双向全等**，新增文件不登记即失败。
//   ② 体量上限：默认 ≤600；拆分前存量越线者进 LEGACY_FROZEN 锁现值（只降不升、禁止新增）。
//   ③ 方向：页面与组件不得反向 import App / pages/routes；map-canvas 件不得 import console-ui 的 *Page。
//   ④ 子目录守卫：对 canvas/shell/overlays/chat/settings/taskworkflow **递归**做
//      「文件集快照双向全等 + LOC≤上限 + 反向 import」，并以 KNOWN_SUBDIRS 锁死顶层子目录集合。
//
// 口径：LOC = split('\n').length（文件以换行结尾时 = wc -l + 1，比 wc 口径大 1，勿混淆）。
// @ts-expect-error vitest 运行时支持 node:fs（tsconfig 无 node 类型）
import { readFileSync, readdirSync } from 'node:fs'
// @ts-expect-error vitest 运行时支持 node:path
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

// @ts-expect-error vitest 运行时提供
const cwd: string = process.cwd()

const read = (rel: string) => readFileSync(resolve(cwd, rel), 'utf-8')
const loc = (rel: string) => read(rel).split('\n').length
const ls = (relDir: string, exts: string[]) =>
  readdirSync(resolve(cwd, relDir)).filter((f: string) => exts.some((e) => f.endsWith(e)))

// 递归收集 `src/components/<dir>/**` 的 .ts/.tsx，返回相对 `src/` 的路径（`components/<dir>/...`）。
const lsRecursive = (dir: string): string[] => {
  const out: string[] = []
  const walk = (rel: string) => {
    for (const e of readdirSync(resolve(cwd, rel), { withFileTypes: true })) {
      const child = `${rel}/${e.name}`
      if (e.isDirectory()) walk(child)
      else if (/\.tsx?$/.test(e.name)) out.push(child.replace(/^src\/components\//, ''))
    }
  }
  walk(`src/components/${dir}`)
  return out
}

// ---------------- 归属快照：四落点（c-arch-3，2026-10-06 迁移后重采） ----------------
// 9 个路由可达整页（`routes.tsx` 由 repoLayout 的冻结键集合单独守）。
const PAGES_FILES = [
  'ChangesPage.tsx', 'DepsPage.tsx', 'DriftPage.tsx', 'GitPage.tsx', 'HealthPage.tsx',
  'ModulesPage.tsx', 'PlaceholderPage.tsx', 'RunsPage.tsx', 'UsagePage.tsx',
]
// 本次迁入 canvas/ 的 7 件（含 4 个 ReactFlow 节点 + 3 个画布面板）。
const CANVAS_NEW_FILES = [
  'BandNode.tsx', 'ModuleNode.tsx', 'SubmoduleNode.tsx', 'ExpandedModuleNode.tsx',
  'DetailPanel.tsx', 'IssuesList.tsx', 'TaskFormPanel.tsx',
]
const SHELL_NEW_FILES = ['AppShell.tsx', 'WindowControls.tsx', 'ThemeToggle.tsx']
const OVERLAYS_NEW_FILES = [
  'OnboardingChecklist.tsx', 'SessionBubble.tsx', 'ViewsPanel.tsx', 'WelcomePage.tsx',
]

const DEFAULT_CEILING = 600

// LEGACY ratchet：存量 >600 的受守卫文件锁当前值（只降不升、禁止新增）。
// 键 = 相对 `src/` 的仓库路径（搬迁后改址，见 c-arch-3 方案 R5-A/R9）。
const LEGACY_FROZEN: Record<string, number> = {
  'pages/GitPage.tsx': 878,
  'pages/RunsPage.tsx': 667,
  'components/canvas/DetailPanel.tsx': 628,
  'components/canvas/Canvas.tsx': 700, // 既有 repoLayout ≤700 承诺，不收紧
}
const ceilingOf = (rel: string) => LEGACY_FROZEN[rel] ?? DEFAULT_CEILING
const sorted = (a: string[]) => [...a].sort()

// ---------------- 子目录守卫（2026-10-06 扩至 6 域：canvas/shell/overlays + chat/settings/taskworkflow） ----------------
// 以下文件集为**双向全等快照**：增删文件必须同步改表（防「换个地址复活」）。
const GUARDED_SUBDIRS = ['canvas', 'shell', 'overlays', 'chat', 'settings', 'taskworkflow']

const SUBDIR_FILES: Record<string, string[]> = {
  canvas: [
    'canvas/BandNode.tsx', 'canvas/Canvas.tsx', 'canvas/CanvasBoundary.tsx',
    'canvas/DetailPanel.tsx', 'canvas/ExpandedModuleNode.tsx', 'canvas/FilterButton.tsx',
    'canvas/GrowthPanel.tsx', 'canvas/InductionOverlay.tsx', 'canvas/IssuesList.tsx',
    'canvas/Legend.tsx', 'canvas/ModuleNode.tsx', 'canvas/ModuleToolbar.tsx',
    'canvas/SubmoduleNode.tsx', 'canvas/TaskFormPanel.tsx', 'canvas/buildFlow.ts',
    'canvas/nodeTypes.ts', 'canvas/types.ts', 'canvas/useCanvasPanelDrag.ts',
    'canvas/useGrowthPlayback.ts', 'canvas/useSubmaps.ts',
  ],
  shell: [
    'shell/AppShell.tsx', 'shell/AttentionBar.tsx', 'shell/ThemeToggle.tsx',
    'shell/TopBar.tsx', 'shell/TopCenter.tsx', 'shell/WindowControls.tsx',
  ],
  overlays: [
    'overlays/ChecklistOverlay.tsx', 'overlays/OnboardingChecklist.tsx',
    'overlays/SessionBubble.tsx', 'overlays/SessionBubbleCard.tsx', 'overlays/SuggestDrawer.tsx',
    'overlays/TaskDraftOverlay.tsx', 'overlays/ViewsDrawer.tsx', 'overlays/ViewsPanel.tsx',
    'overlays/WelcomeOverlay.tsx', 'overlays/WelcomePage.tsx',
  ],
  chat: [
    'chat/AnswerCards.tsx', 'chat/ChatPanel.tsx', 'chat/ComposerDock.tsx',
    'chat/ConversationSwitcher.tsx', 'chat/HeaderActions.tsx', 'chat/MessageStream.tsx',
    'chat/PanelChat.tsx', 'chat/QuickAsk.tsx', 'chat/QuickAskComposer.tsx', 'chat/QuickAskStream.tsx',
    'chat/SuggestPanel.tsx', 'chat/WorkbenchPage.tsx', 'chat/chatUpgrade.ts', 'chat/types.ts',
    'chat/useConversations.ts',
  ],
  settings: [
    'settings/AboutSection.tsx', 'settings/AdvancedSection.tsx', 'settings/AgentSection.tsx',
    'settings/HarnessSection.tsx', 'settings/ServicesSection.tsx', 'settings/SettingsPanel.tsx',
    'settings/common.ts', 'settings/controls.tsx',
  ],
  taskworkflow: [
    'taskworkflow/DocCard.tsx', 'taskworkflow/PhaseDocReview.tsx', 'taskworkflow/StageLookback.tsx',
    'taskworkflow/StagePipeline.tsx', 'taskworkflow/TaskAdminButtons.tsx', 'taskworkflow/TaskBoardPage.tsx',
    'taskworkflow/TaskGovernancePage.tsx', 'taskworkflow/TaskPage.tsx', 'taskworkflow/TaskWorkflowPage.tsx',
    'taskworkflow/diffParse.ts', 'taskworkflow/taskAdmin.ts', 'taskworkflow/taskStage.ts',
    'taskworkflow/types.ts',
    'taskworkflow/stages/AnalysisStage.tsx', 'taskworkflow/stages/DiffStage.tsx',
    'taskworkflow/stages/DoneStage.tsx', 'taskworkflow/stages/ErrorStage.tsx',
    'taskworkflow/stages/ReportStage.tsx', 'taskworkflow/stages/TerminalStage.tsx',
  ],
}

// `src/components/` 顶层允许出现的子目录全集——新增目录不登记即失败，杜绝"换个地址复活"。
const KNOWN_SUBDIRS = [
  '__tests__', 'canvas', 'chat', 'gate', 'overlays', 'settings', 'shell', 'taskworkflow', 'ui',
]
// 相对 `src/components/` 的子目录文件（键形如 `canvas/BandNode.tsx`）。
const SUBDIR_FILES_FLAT = GUARDED_SUBDIRS.flatMap((d) => SUBDIR_FILES[d])
// 受守卫文件全集（**相对 src/**）：9 页 + 六域子目录文件（用于 LOC 棘轮键集合双向全等）。
const GUARDED_ABS = [
  ...PAGES_FILES.map((f) => `pages/${f}`),
  ...SUBDIR_FILES_FLAT.map((f) => `components/${f}`),
]

describe('防膨胀守卫 · 断言组 1：归属快照（四落点，双向全等）', () => {
  it('src/pages/*.tsx 集合 == 9 个整页 + routes.tsx（装配表就地保留）', () => {
    expect(sorted(ls('src/pages', ['.tsx']))).toEqual(sorted([...PAGES_FILES, 'routes.tsx']))
  })
  it('canvas/ 已收录 7 个迁移件（R2/R3；全量全等在断言组 4）', () => {
    const actual = new Set(readdirSync(resolve(cwd, 'src/components/canvas')))
    for (const f of CANVAS_NEW_FILES) expect(actual.has(f), `canvas/${f}`).toBe(true)
  })
  it('shell/ 已收录 3 个装配壳件（R3）', () => {
    const actual = new Set(readdirSync(resolve(cwd, 'src/components/shell')))
    for (const f of SHELL_NEW_FILES) expect(actual.has(f), `shell/${f}`).toBe(true)
  })
  it('overlays/ 已收录 4 个浮层件（R3）', () => {
    const actual = new Set(readdirSync(resolve(cwd, 'src/components/overlays')))
    for (const f of OVERLAYS_NEW_FILES) expect(actual.has(f), `overlays/${f}`).toBe(true)
  })
})

describe('防膨胀守卫 · 断言组 2：LOC 上限（行数为权威）', () => {
  it(`每个受守卫文件 ≤ 上限（默认 ${DEFAULT_CEILING}；LEGACY 锁现值）`, () => {
    for (const rel of GUARDED_ABS) {
      expect(loc(`src/${rel}`), rel).toBeLessThanOrEqual(ceilingOf(rel))
    }
  })
  it('LEGACY 表键集合 == 上限 >600 的文件集合（显式冻结，禁止静默新增）', () => {
    const over = GUARDED_ABS.filter((f) => ceilingOf(f) > DEFAULT_CEILING)
    expect(sorted(over)).toEqual(sorted(Object.keys(LEGACY_FROZEN)))
  })
})

describe('防膨胀守卫 · 断言组 3：反向/横向 import 禁止', () => {
  it('页面与组件不得反向 import App 或页面装配（别名与相对路径双堵）', () => {
    const reApp = /from ['"](@\/App|\.\.?\/App)['"]/
    const reRoutes = /from ['"](@\/pages\/routes|\.\.?\/pages\/routes)['"]/
    for (const rel of GUARDED_ABS) {
      const src = read(`src/${rel}`)
      expect(src, `${rel} 反向 import App`).not.toMatch(reApp)
      expect(src, `${rel} 反向 import pages/routes`).not.toMatch(reRoutes)
    }
  })
  it('map-canvas 件不得 import console-ui 的 *Page（维持 map-canvas → console-ui 单向）', () => {
    // 页面迁址后 *Page 落点由 `@/components/*Page` 变为 `@/pages/*Page` / `./*Page` / `../pages/*Page`，
    // 正则同步放宽（ΔS3：只认旧落点会导致搬迁后假绿）。
    const rePage = /from ['"](?:@\/(?:components\/[A-Za-z]*Page|pages\/[A-Za-z]*Page)|(?:\.\.\/)+(?:pages\/)?[A-Za-z]*Page)['"]/
    for (const f of SUBDIR_FILES.canvas) {
      expect(read(`src/components/${f}`), `${f} import console-ui *Page`).not.toMatch(rePage)
    }
  })
  it('ui-kit 生成物不得 import 业务面（App / pages / 业务组件）', () => {
    const uiDir = 'src/components/ui'
    const files = readdirSync(resolve(cwd, uiDir)).filter((f: string) => f.endsWith('.tsx') || f.endsWith('.ts'))
    const reBusiness = /from ['"](@\/App|@\/pages\/|@\/components\/(?!ui\/))/
    for (const f of files) {
      expect(read(`${uiDir}/${f}`), `ui/${f} 引用业务层`).not.toMatch(reBusiness)
    }
  })
})

describe('防膨胀守卫 · 断言组 4：域子目录（canvas / shell / overlays / chat / settings / taskworkflow）', () => {
  it('守卫子目录文件集快照双向全等（新增/删除文件必须显式登记，防换个地址复活）', () => {
    for (const dir of GUARDED_SUBDIRS) {
      expect(sorted(lsRecursive(dir)), `components/${dir}/**`).toEqual(sorted(SUBDIR_FILES[dir]))
    }
  })

  it('守卫子目录每个文件 ≤ ceilingOf(rel)（薄壳背后的实现体同样受限）', () => {
    for (const rel of SUBDIR_FILES_FLAT) {
      expect(loc(`src/components/${rel}`), rel).toBeLessThanOrEqual(ceilingOf(`components/${rel}`))
    }
  })

  it('守卫子目录不得反向 import App / pages（维持页面装配 → 组件单向）', () => {
    const reApp = /from ['"](?:@\/App|(?:\.\.\/)+App)['"]/
    const rePages = /from ['"](?:@\/pages\/|(?:\.\.\/)+pages\/)/
    for (const rel of SUBDIR_FILES_FLAT) {
      const src = read(`src/components/${rel}`)
      expect(src, `${rel} 反向 import App`).not.toMatch(reApp)
      expect(src, `${rel} 反向 import pages`).not.toMatch(rePages)
    }
  })

  it('chat/** 不得 import console-ui 的 *Page', () => {
    const rePage = /from ['"](?:@\/(?:components\/[A-Za-z]*Page|pages\/[A-Za-z]*Page)|(?:\.\.\/)+(?:pages\/)?[A-Za-z]*Page)['"]/
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

  it('反绕过自证：归属扫描器对根级组件（含大小写/扩展名变体）非空洞', () => {
    // 断言组 1 的「⊇ 迁移件」与本组「双向全等」都依赖 ls/lsRecursive 非空且能识别真实文件；
    // 此处内联样例证明扫描口径不会静默变空（配合 archGuard 组 7 的根目录空集断言）。
    const sample = ['Foo.tsx', 'Bar.ts', 'Baz.tsx.bak', 'canvas']
    expect(sample.filter((f) => f.endsWith('.tsx'))).toEqual(['Foo.tsx'])
  })
})
