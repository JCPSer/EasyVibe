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
