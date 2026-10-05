// 模块边界守卫（R5，对应打回意见 c-arch-1）：renderer-runtime / renderer-shared 切环 + 三域拆分的
// 防复现机制。范式同 componentGuard/repoLayout：只读源码文本（不 import 被测符号），毫秒级进 vitest run。
//
// 守卫的四条硬结论（方案 §3.2 方向规则）：
//   ① runtime / shared 是 leaf：不得 import 任何上层业务（components/pages/hooks/App/lib）。
//   ② 环回归：chat-ui ⊥ map-canvas、task-ui ⊥ chat-ui；map-canvas 文件集不得再出现 @/lib|pages|hooks|App
//      返回边（切环判据）；map-canvas → chat-ui 仅保留 DetailPanel→PanelChat 单向白名单。
//   ③ 装配方向：域文件不得反向 import App / pages/routes；仅 App.tsx / pages/routes.tsx 可 import 各域页面。
//   ④ 文件集快照：runtime / shared / 三域 与磁盘实况双向全等（增删必须同步改表，防"换个地址复活"）。
//
// 契约冻结（PageId 17 / WS 10 / Canvas props 20）由 repoLayout.test.ts 覆盖，此处不重复。
// @ts-expect-error vitest 运行时支持 node:fs（tsconfig 无 node 类型）
import { readFileSync, readdirSync } from 'node:fs'
// @ts-expect-error vitest 运行时支持 node:path
import { resolve, dirname, join, normalize } from 'node:path'
import { describe, expect, it } from 'vitest'

// @ts-expect-error vitest 运行时提供
const cwd: string = process.cwd()
const read = (rel: string) => readFileSync(resolve(cwd, rel), 'utf-8')
const sorted = (a: string[]) => [...a].sort()

const walk = (relDir: string): string[] => {
  const out: string[] = []
  const rec = (rel: string) => {
    for (const e of readdirSync(resolve(cwd, rel), { withFileTypes: true })) {
      const child = `${rel}/${e.name}`
      if (e.isDirectory()) rec(child)
      else if (/\.tsx?$/.test(e.name)) out.push(child)
    }
  }
  rec(relDir)
  return out
}

// 抽取一个文件里所有 import/export-from 的模块说明符（别名与相对路径都收）。
const SPEC_RE = /(?:from\s+|import\s*\(\s*)(['"])([^'"]+)\1/g
const specs = (rel: string): string[] => {
  const src = read(rel)
  return [...src.matchAll(SPEC_RE)].map((m) => m[2])
}

// 把相对说明符解析成仓库相对路径候选（不带扩展名），用于"别名+相对双堵"。
const resolveRel = (fileRel: string, spec: string): string | null => {
  if (spec.startsWith('@/')) return 'easyvibe-renderer/src/' + spec.slice(2)
  if (spec.startsWith('./') || spec.startsWith('../')) {
    const base = dirname(fileRel)
    return normalize(join(base, spec)).replace(/\\/g, '/')
  }
  return null
}

// ---------------- 文件集定义（与磁盘实况双向全等，见断言组 4） ----------------
const RUNTIME_FILES = [
  'runtime/__tests__/terminalBuffer.test.ts',
  'runtime/analytics.ts', 'runtime/env.ts', 'runtime/growthBus.ts', 'runtime/motion.ts',
  'runtime/notify.ts', 'runtime/sessionQueue.ts', 'runtime/terminalBuffer.ts', 'runtime/toast.tsx',
  'runtime/useRepoActivity.ts', 'runtime/ws.ts',
]
const SHARED_FILES = [
  'shared/contract/chat.ts', 'shared/contract/selection.ts',
  'shared/logic/depsAnalysis.ts', 'shared/logic/diffStat.ts', 'shared/logic/growthMerge.ts',
  'shared/logic/inductionProgress.ts', 'shared/logic/layout.ts', 'shared/logic/onboardingCopy.ts',
  'shared/logic/taskContext.ts',
  'shared/primitives/MarkdownMessage.tsx',
]
const CHAT_FILES = [
  'chat/AnswerCards.tsx', 'chat/ChatPanel.tsx', 'chat/ComposerDock.tsx',
  'chat/ConversationSwitcher.tsx', 'chat/HeaderActions.tsx', 'chat/MessageStream.tsx',
  'chat/PanelChat.tsx', 'chat/QuickAsk.tsx', 'chat/QuickAskComposer.tsx', 'chat/QuickAskStream.tsx',
  'chat/SuggestPanel.tsx', 'chat/WorkbenchPage.tsx', 'chat/chatUpgrade.ts', 'chat/types.ts',
  'chat/useConversations.ts',
]
const SETTINGS_FILES = [
  'settings/AboutSection.tsx', 'settings/AdvancedSection.tsx', 'settings/AgentSection.tsx',
  'settings/HarnessSection.tsx', 'settings/ServicesSection.tsx', 'settings/SettingsPanel.tsx',
  'settings/common.ts', 'settings/controls.tsx',
]
const TASK_FILES = [
  'taskworkflow/DocCard.tsx', 'taskworkflow/PhaseDocReview.tsx', 'taskworkflow/StageLookback.tsx',
  'taskworkflow/StagePipeline.tsx', 'taskworkflow/TaskAdminButtons.tsx', 'taskworkflow/TaskBoardPage.tsx',
  'taskworkflow/TaskGovernancePage.tsx', 'taskworkflow/TaskPage.tsx', 'taskworkflow/TaskWorkflowPage.tsx',
  'taskworkflow/diffParse.ts', 'taskworkflow/taskAdmin.ts', 'taskworkflow/taskStage.ts',
  'taskworkflow/types.ts',
  'taskworkflow/stages/AnalysisStage.tsx', 'taskworkflow/stages/DiffStage.tsx',
  'taskworkflow/stages/DoneStage.tsx', 'taskworkflow/stages/ErrorStage.tsx',
  'taskworkflow/stages/ReportStage.tsx', 'taskworkflow/stages/TerminalStage.tsx',
]

const DOMAIN_DIRS = ['chat', 'settings', 'taskworkflow'] as const
const DOMAIN_SETS: Record<string, string[]> = { chat: CHAT_FILES, settings: SETTINGS_FILES, taskworkflow: TASK_FILES }
const DOMAIN_ABS: Record<string, string[]> = {
  chat: CHAT_FILES.map((f) => `src/components/${f}`),
  settings: SETTINGS_FILES.map((f) => `src/components/${f}`),
  taskworkflow: TASK_FILES.map((f) => `src/components/${f}`),
}
const RUNTIME_ABS = RUNTIME_FILES.map((f) => `src/${f}`)
const SHARED_ABS = SHARED_FILES.map((f) => `src/${f}`)

// map-canvas 文件集（方案 §3.3）：canvas/** + 7 个顶层（节点 4 + DetailPanel/IssuesList/TaskFormPanel）
const MAP_CANVAS_FILES = [
  ...walk('src/components/canvas'),
  'src/components/ModuleNode.tsx', 'src/components/BandNode.tsx', 'src/components/SubmoduleNode.tsx',
  'src/components/ExpandedModuleNode.tsx', 'src/components/DetailPanel.tsx',
  'src/components/IssuesList.tsx', 'src/components/TaskFormPanel.tsx',
]

describe('archGuard · 断言组 1：leaf（runtime / shared 不得依赖业务层）', () => {
  it('src/runtime/** 只许 types/react/外部库（禁 components/pages/hooks/shared/lib/App）', () => {
    const forbidden = /^(?:@\/(?:components|pages|hooks|shared|lib|App)|easyvibe-renderer\/src\/(?:components|pages|hooks|shared|lib|App))/
    for (const f of RUNTIME_ABS) {
      for (const s of specs(f)) {
        const r = resolveRel(f, s) ?? s
        expect(forbidden.test(s) || forbidden.test(r), `${f} → ${s}`).toBe(false)
      }
    }
  })
  it('src/shared/** 只许 types/react/外部库（禁 components/pages/hooks/runtime/lib/App）', () => {
    const forbidden = /^(?:@\/(?:components|pages|hooks|runtime|lib|App)|easyvibe-renderer\/src\/(?:components|pages|hooks|runtime|lib|App))/
    for (const f of SHARED_ABS) {
      for (const s of specs(f)) {
        const r = resolveRel(f, s) ?? s
        expect(forbidden.test(s) || forbidden.test(r), `${f} → ${s}`).toBe(false)
      }
    }
  })
})

describe('archGuard · 断言组 2：环回归（打回意见 c-arch-1）', () => {
  it('chat-ui ⊥ map-canvas：chat/** 不得 import map-canvas（DetailPanel/IssuesList/canvas/节点）', () => {
    const re = /^(?:@\/components\/(?:DetailPanel|IssuesList|canvas\/|ModuleNode|BandNode|SubmoduleNode|ExpandedModuleNode)|easyvibe-renderer\/src\/components\/(?:DetailPanel|IssuesList|canvas\/|ModuleNode|BandNode|SubmoduleNode|ExpandedModuleNode))/
    for (const f of DOMAIN_ABS.chat) {
      for (const s of specs(f)) {
        const r = resolveRel(f, s) ?? s
        expect(re.test(s) || re.test(r), `${f} → ${s}`).toBe(false)
      }
    }
  })
  it('task-ui ⊥ chat-ui：taskworkflow/** 不得 import chat 组件或 MarkdownMessage', () => {
    const re = /^(?:@\/components\/(?:chat\/|ChatPanel|QuickAsk|PanelChat|AnswerCards|SuggestPanel|WorkbenchPage|MarkdownMessage)|easyvibe-renderer\/src\/components\/(?:chat\/|ChatPanel|QuickAsk|PanelChat|AnswerCards|SuggestPanel|WorkbenchPage|MarkdownMessage))/
    for (const f of DOMAIN_ABS.taskworkflow) {
      for (const s of specs(f)) {
        const r = resolveRel(f, s) ?? s
        expect(re.test(s) || re.test(r), `${f} → ${s}`).toBe(false)
      }
    }
  })
  it('map-canvas → console-ui 返回边归零：不得 import @/lib、@/pages、@/hooks、@/App', () => {
    const re = /^(?:@\/(?:lib|pages|hooks|App)|easyvibe-renderer\/src\/(?:lib|pages|hooks|App))/
    for (const f of MAP_CANVAS_FILES) {
      for (const s of specs(f)) {
        const r = resolveRel(f, s) ?? s
        expect(re.test(s) || re.test(r), `${f} → ${s}（返回边未切断）`).toBe(false)
      }
    }
  })
  it('map-canvas → chat-ui 仅白名单 DetailPanel → PanelChat（单向）', () => {
    const re = /^(?:@\/components\/(?:chat\/|ChatPanel|QuickAsk|PanelChat|AnswerCards|SuggestPanel|WorkbenchPage)|easyvibe-renderer\/src\/components\/(?:chat\/|ChatPanel|QuickAsk|PanelChat|AnswerCards|SuggestPanel|WorkbenchPage))/
    for (const f of MAP_CANVAS_FILES) {
      for (const s of specs(f)) {
        const r = resolveRel(f, s) ?? s
        if (!re.test(s) && !re.test(r)) continue
        const allowed = f === 'src/components/DetailPanel.tsx' && s === '@/components/chat/PanelChat'
        expect(allowed, `${f} → ${s}（非白名单的 map-canvas→chat 边）`).toBe(true)
      }
    }
  })
})

describe('archGuard · 断言组 3：装配方向（域 → 装配点 禁止，装配点 → 域 放行）', () => {
  it('域文件不得反向 import App / pages/routes', () => {
    const re = /^(?:@\/(?:App|pages\/routes)|easyvibe-renderer\/src\/(?:App|pages\/routes))(?:$|\.|')/
    const all = [...RUNTIME_ABS, ...SHARED_ABS, ...Object.values(DOMAIN_ABS).flat(), ...MAP_CANVAS_FILES]
    for (const f of all) {
      for (const s of specs(f)) {
        const r = resolveRel(f, s) ?? s
        expect(re.test(r), `${f} → ${s}`).toBe(false)
      }
    }
  })
  it('仅 App.tsx / pages/routes.tsx 可 import 域页面（装配点唯一性）', () => {
    // 仅覆盖"路由级域入口页"：WorkbenchPage / SettingsPanel / TaskPage / Canvas。
    // 域内页面互引（如 TaskPage → TaskBoardPage）与面板组合（DetailPanel → PanelChat 白名单、
    // SuggestDrawer → SuggestPanel）不属于"域 → 装配点"反例，另行在白名单/方向规则中约束。
    const domainPage = /@\/components\/(?:chat\/WorkbenchPage|settings\/SettingsPanel|taskworkflow\/TaskPage|canvas\/Canvas)/
    const allowedFiles = new Set(['src/App.tsx', 'src/pages/routes.tsx'])
    for (const f of walk('src')) {
      if (allowedFiles.has(f)) continue
      for (const s of specs(f)) {
        expect(domainPage.test(s), `${f} → ${s}（域页面只能由装配点 import）`).toBe(false)
      }
    }
  })
})

describe('archGuard · 断言组 4：文件集快照（双向全等）', () => {
  it('src/runtime/** 集合 == RUNTIME_FILES', () => {
    expect(sorted(walk('src/runtime').map((f) => f.replace(/^src\//, '')))).toEqual(sorted(RUNTIME_FILES))
  })
  it('src/shared/** 集合 == SHARED_FILES', () => {
    expect(sorted(walk('src/shared').map((f) => f.replace(/^src\//, '')))).toEqual(sorted(SHARED_FILES))
  })
  it('三域目录集合 == 快照（chat/settings/taskworkflow 文件已登记）', () => {
    for (const dir of DOMAIN_DIRS) {
      const actual = walk(`src/components/${dir}`).map((f) => f.replace(/^src\/components\//, ''))
      expect(sorted(actual), `${dir}/**`).toEqual(sorted(DOMAIN_SETS[dir]))
    }
  })
})

// ---------------- 断言组 5：api client 层与「禁业务文件直连 REST」（c-arch-1 收敛） ----------------
//
// R6/R7/R8：server-api 的 REST 契约面收敛为「每域一个 api client」，业务文件不得再直连。
//  ① src/api/** 文件集快照（双向全等，照 RUNTIME_FILES 范式）；
//  ② 业务文件禁直连：源码（**去注释后**）不得出现裸 `fetch(`、`'/api/…'` 字面量、`new WebSocket`；
//  ③ src/api/** 依赖白名单：只许相对路径与 @/runtime、@/types（不得反向 import 业务层）；
//  ④ LEGACY 棘轮：批 C 四域未迁完，登记现值只降不升、禁新增（照 componentGuard 范式）；
//  ⑤ 反绕过：`fetch` 与 `/api/` 两条同时扫（`const u='/api/x';fetch(u)` 逃逸被堵）。
const API_FILES = [
  'api/canvas.ts', 'api/chat.ts', 'api/core.ts', 'api/git.ts', 'api/index.ts',
  'api/repos.ts', 'api/settings.ts', 'api/system.ts', 'api/task.ts',
]
const API_ABS = API_FILES.map((f) => `src/${f}`)

/** 网络层 leaf / 测试 / 生成物豁免（见方案 §3.3.2）。runtime 是网络层且为 leaf（不得 import @/api）。 */
const DIRECT_REST_WHITELIST = /^(src\/api\/|src\/runtime\/|src\/types\/generated)|(__tests__\/|\.test\.tsx?$)/
/** 批 C 四域存量（本轮未迁；棘轮只降不升——清干净须摘牌，新增文件不得蒙混）。 */
const DIRECT_REST_LEGACY: ReadonlyArray<readonly [string, number]> = [
  ['src/components/canvas/Canvas.tsx', 4],
  ['src/components/canvas/InductionOverlay.tsx', 6],
  ['src/components/canvas/useGrowthPlayback.ts', 4],
  ['src/components/canvas/useSubmaps.ts', 4],
  ['src/components/chat/ChatPanel.tsx', 8],
  ['src/components/chat/MessageStream.tsx', 4],
  ['src/components/chat/QuickAsk.tsx', 10],
  ['src/components/chat/QuickAskStream.tsx', 4],
  ['src/components/chat/SuggestPanel.tsx', 2],
  ['src/components/chat/WorkbenchPage.tsx', 16],
  ['src/components/chat/useConversations.ts', 10],
  ['src/components/settings/AgentSection.tsx', 10],
  ['src/components/settings/HarnessSection.tsx', 14],
  ['src/components/taskworkflow/DocCard.tsx', 4],
  ['src/components/taskworkflow/PhaseDocReview.tsx', 8],
  ['src/components/taskworkflow/TaskBoardPage.tsx', 4],
  ['src/components/taskworkflow/TaskGovernancePage.tsx', 2],
  ['src/components/taskworkflow/TaskWorkflowPage.tsx', 10],
  ['src/components/taskworkflow/stages/DoneStage.tsx', 2],
  ['src/components/taskworkflow/stages/ErrorStage.tsx', 2],
  ['src/components/taskworkflow/stages/TerminalStage.tsx', 2],
  ['src/components/taskworkflow/taskAdmin.ts', 7],
]

/** 去注释（保留字符串字面量——`/api/` 字面量正是要抓的）：`/* *\/` 与行注释，行注释避开 `https://`。 */
const stripComments = (s: string): string =>
  s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:])\/\/.*$/gm, '$1')
const RE_FETCH = /(?<![\w.])fetch\s*\(/g
const RE_APIPATH = /['"`]\/api\//g
const RE_WS = /new\s+WebSocket\b/g
const directRestHits = (rel: string): number => {
  const src = stripComments(read(rel))
  return (src.match(RE_FETCH)?.length ?? 0) + (src.match(RE_APIPATH)?.length ?? 0) + (src.match(RE_WS)?.length ?? 0)
}
const isBusinessFile = (f: string) => !DIRECT_REST_WHITELIST.test(f)

describe('archGuard · 断言组 5：api client 层 + 禁业务文件直连 REST', () => {
  it('① src/api/** 文件集 == API_FILES（双向全等）', () => {
    expect(sorted(walk('src/api').map((f) => f.replace(/^src\//, '')))).toEqual(sorted(API_FILES))
  })
  it('② 业务文件（去注释后）不得直连 REST：fetch( / /api/ 字面量 / new WebSocket，LEGACY 棘轮只降不升', () => {
    const violating: Record<string, number> = {}
    for (const f of walk('src')) {
      if (!isBusinessFile(f)) continue
      const n = directRestHits(f)
      if (n > 0) violating[f] = n
    }
    const legacy = Object.fromEntries(DIRECT_REST_LEGACY.map(([f, n]) => [f, n]))
    // 键集全等：清干净须摘牌；新增违规文件不得蒙混
    expect(sorted(Object.keys(violating)), '直连 REST 的业务文件集').toEqual(sorted(Object.keys(legacy)))
    for (const [f, budget] of DIRECT_REST_LEGACY) {
      expect(violating[f] ?? 0, `${f} 直连 REST 处数`).toBeLessThanOrEqual(budget)
    }
  })
  it('③ src/api/** 依赖白名单：只许相对路径 + @/runtime + @/types（不得反向 import 业务层）', () => {
    const forbidden = /^(?:@\/(?:components|pages|hooks|lib|App|shared|api)|easyvibe-renderer\/src\/(?:components|pages|hooks|lib|App|shared|api))/
    for (const f of API_ABS) {
      for (const s of specs(f)) {
        const r = resolveRel(f, s) ?? s
        const isRelative = s.startsWith('./') || s.startsWith('../')
        const allowedAlias = /^@\/(?:runtime|types)\//.test(s)
        expect(forbidden.test(s) || forbidden.test(r), `${f} → ${s}`).toBe(false)
        expect(isRelative || allowedAlias, `${f} → ${s}（api 层只许相对路径 / @/runtime / @/types）`).toBe(true)
      }
    }
  })
  it('④ 反绕过：fetch 与 /api/ 字面量两条同时扫（const u="/api/x"; fetch(u) 逃逸被堵）', () => {
    // 直接对内联样例断言扫描器行为（守卫自身可执行性，R12）
    const sample = stripComments(`const u = '/api/repos'\nfetch(u)\nnew WebSocket('/ws')`)
    const hits = (sample.match(RE_FETCH)?.length ?? 0) + (sample.match(RE_APIPATH)?.length ?? 0) + (sample.match(RE_WS)?.length ?? 0)
    expect(hits).toBe(3)
    // 注释中的 /api/ 与 fetch 不误报
    expect(directRestHits('src/api/core.ts')).toBeGreaterThan(0) // 唯一出口自身持有 fetch（在 src/api 内合法）
  })
})
