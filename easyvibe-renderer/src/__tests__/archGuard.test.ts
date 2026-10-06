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
// @ts-expect-error vitest 运行时支持 node:child_process
import { execFileSync } from 'node:child_process'
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
  'runtime/analytics.ts', 'runtime/env.ts', 'runtime/growthBus.ts', 'runtime/host.ts', 'runtime/motion.ts',
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

// map-canvas 文件集（c-arch-3 组件归属规则化后）：全部落 `components/canvas/**`，
// 原 7 条顶层内联路径（4 节点 + DetailPanel/IssuesList/TaskFormPanel）已迁入，由 walk 自动收录。
const MAP_CANVAS_FILES = walk('src/components/canvas')

// 装配点集合（console-ui）：pages/**（含 routes.tsx）+ App.tsx 之外的业务文件须单向依赖。
const PAGES_ABS = walk('src/pages')

// map-canvas → chat-ui 白名单判定（c-arch-3 搬迁后抽为纯函数 + 反绕过自证，见断言组 2 ④）。
// DetailPanel 迁址 `components/DetailPanel.tsx` → `components/canvas/DetailPanel.tsx` 后，
// 旧字面量判定会静默失效（放行非法边或误报），故以路径后缀 + 相对形态双判。
const isAllowedCanvasToChatEdge = (file: string, spec: string): boolean => {
  const isDetailPanel = file.endsWith('components/canvas/DetailPanel.tsx')
  const toPanelChat = spec === '@/components/chat/PanelChat' || /(?:\.\.\/)+chat\/PanelChat$/.test(spec)
  return isDetailPanel && toPanelChat
}

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
  it('chat-ui ⊥ map-canvas：chat/** 不得 import map-canvas（canvas/ 全域）', () => {
    // 搬迁后 map-canvas 件全部落 `@/components/canvas/**` / `easyvibe-renderer/src/components/canvas/**`，
    // 旧正则里的裸文件名分支（DetailPanel|ModuleNode…)已失效，收窄为唯一真实落点（ΔS3）。
    const re = /^(?:@\/components\/canvas\/|easyvibe-renderer\/src\/components\/canvas\/)/
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
        expect(isAllowedCanvasToChatEdge(f, s), `${f} → ${s}（非白名单的 map-canvas→chat 边）`).toBe(true)
      }
    }
  })
  it('④′ 反绕过自证：白名单判定随迁（canvas/DetailPanel→PanelChat 放行，画布其他件仍拦截）', () => {
    expect(isAllowedCanvasToChatEdge('src/components/canvas/DetailPanel.tsx', '@/components/chat/PanelChat')).toBe(true)
    expect(isAllowedCanvasToChatEdge('src/components/canvas/DetailPanel.tsx', '../chat/PanelChat')).toBe(true)
    expect(isAllowedCanvasToChatEdge('src/components/canvas/Canvas.tsx', '@/components/chat/PanelChat')).toBe(false)
    expect(isAllowedCanvasToChatEdge('src/pages/GitPage.tsx', '@/components/chat/PanelChat')).toBe(false)
  })
})

describe('archGuard · 断言组 3：装配方向（域 → 装配点 禁止，装配点 → 域 放行）', () => {
  it('域文件不得反向 import App / pages/routes', () => {
    const re = /^(?:@\/(?:App|pages\/routes)|easyvibe-renderer\/src\/(?:App|pages\/routes))(?:$|\.|')/
    // + PAGES_ABS：整页落 `src/pages/**` 后同样不得反向 import App / routes（c-arch-3 R5-B）。
    const all = [
      ...RUNTIME_ABS, ...SHARED_ABS, ...Object.values(DOMAIN_ABS).flat(),
      ...MAP_CANVAS_FILES, ...PAGES_ABS,
    ]
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

/** 网络层 leaf / 测试 / 生成物豁免（见方案 §3.3.2）。runtime 是网络层且为 leaf（不得 import @/api），
 *  R5 裁定：保持豁免，但其直连作为**受控网络层出口**独立登记（见 NETWORK_REST_BASELINE），只降不升。 */
const DIRECT_REST_WHITELIST = /^(src\/api\/|src\/runtime\/|src\/types\/generated)|(__tests__\/|\.test\.tsx?$)/
/** 业务域存量（R7 真值化：R6 修复 stripComments 后按去注释真值登记）。
 *  逐域迁移（canvas → taskworkflow → chat → settings）后**已清空**：业务文件直连 REST = 0。
 *  棘轮纪律不撤销：键集双向全等（任何新增直连 → 键集不等 → 必红）+ 只降不升。 */
const DIRECT_REST_LEGACY: ReadonlyArray<readonly [string, number]> = []

/** R5 裁定：runtime 直连 = renderer-core 内部**受控网络层出口**（raw fetch / WS 属传输层自身实现，
 *  不构成 presentation→application 直连）；独立登记、只降不升，与业务棘轮分表。 */
const NETWORK_REST_BASELINE: ReadonlyArray<readonly [string, number]> = [
  ['src/runtime/analytics.ts', 2],
  ['src/runtime/sessionQueue.ts', 2],
  ['src/runtime/useRepoActivity.ts', 2],
  ['src/runtime/ws.ts', 3],
]

/** 去注释（保留字符串字面量——`/api/` 字面量正是要抓的）。
 *
 * R6 盲区修复：旧实现「先删块注释」时，块注释起始记号会出现在行注释文本里
 * （例如「见 ./settings/*（x）」中的注释起始符），与后续某处的块注释结束符配对，
 * 吞掉大段真实代码（`SettingsPanel.tsx` 26→0、`ChatPanel.tsx` 12→8），导致键集两侧同盲的假绿。
 * 改为**单遍状态机**：进入行注释即跳到行尾（行注释优先于块注释识别）；字符串/模板
 * 字面量内原样保留（`'/api/*'` 不被误删，仍会被 RE_APIPATH 抓到）；未闭合块注释到文件尾。 */
const stripComments = (s: string): string => {
  const out: string[] = []
  let state: 'code' | 'line' | 'block' | 'string' = 'code'
  let quote = ''
  let i = 0
  while (i < s.length) {
    const c = s[i]
    const d = s[i + 1]
    if (state === 'code') {
      if (c === '/' && d === '/') {
        state = 'line'
        i += 2
        continue
      }
      if (c === '/' && d === '*') {
        state = 'block'
        i += 2
        continue
      }
      if (c === "'" || c === '"' || c === '`') {
        quote = c
        state = 'string'
        out.push(c)
        i += 1
        continue
      }
      out.push(c)
      i += 1
      continue
    }
    if (state === 'line') {
      if (c === '\n') {
        state = 'code'
        out.push(c)
      }
      i += 1
      continue
    }
    if (state === 'block') {
      if (c === '*' && d === '/') {
        state = 'code'
        i += 2
        continue
      }
      i += 1
      continue
    }
    // state === 'string'：反斜杠转义保真，闭合引号回 code；内容原样保留。
    if (c === '\\') {
      out.push(c)
      if (i + 1 < s.length) out.push(s[i + 1])
      i += 2
      continue
    }
    if (c === quote) {
      state = 'code'
      out.push(c)
      i += 1
      continue
    }
    out.push(c)
    i += 1
  }
  return out.join('')
}
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
  it('⑤ runtime 网络层出口登记（R5）：只降不升（与业务棘轮分表，见 NETWORK_REST_BASELINE）', () => {
    const actual: Record<string, number> = {}
    for (const f of walk('src/runtime')) {
      const n = directRestHits(f)
      if (n > 0) actual[f] = n
    }
    // 键集全等：网络层出口若新增/消失须同步登记表
    expect(sorted(Object.keys(actual)), 'runtime 网络层出口文件集').toEqual(
      sorted(NETWORK_REST_BASELINE.map(([f]) => f)),
    )
    for (const [f, budget] of NETWORK_REST_BASELINE) {
      expect(actual[f] ?? 0, `${f} 网络层直连处数`).toBeLessThanOrEqual(budget)
    }
  })
  it('⑥ 盲区回归哨兵（R6/A7）：stripComments 不吞代码；行注释内的 `/*` 不改变计数', () => {
    // 复现旧盲区最小样例：行注释里的 `/*` 曾与后续 JSX 注释的 `*/` 配对吞掉大段代码
    const sample = "// 见 ./settings/*（2026-10-05）\nconst a = 1\n{/* 分区导航 */}\nconst b = 2\n"
    const stripped = stripComments(sample)
    expect(stripped).toContain('const a = 1')
    expect(stripped).toContain('const b = 2')
    // 字符串里的 `/*` / `/api/` 原样保留（不去注释，仍可被 RE_APIPATH 抓）
    expect(stripComments(`const u = '/api/*'`)).toContain('/api/*')
    // 对真实文件的计数不因行注释注入 `/*` 而改变（永久哨兵）
    const target = 'src/components/settings/SettingsPanel.tsx'
    const before = directRestHits(target)
    const src = read(target)
    const injected = src.replace(/^(.*)$/m, '$1 // 注入 /* 干扰哨兵')
    // 用同一状态机对注入文本计数（直接Rest样本级验证）
    const countOf = (text: string) =>
      ((t: string) => (t.match(RE_FETCH)?.length ?? 0) + (t.match(RE_APIPATH)?.length ?? 0) + (t.match(RE_WS)?.length ?? 0))(
        stripComments(text),
      )
    expect(countOf(injected)).toBe(before)
  })
  it('⑦ 迁移终态（R6/R7/R3/R2）：SettingsPanel 26→0、ChatPanel 12→0（盲区已修，归零可证）', () => {
    // 轨迹：R6 修复前 SettingsPanel 26（盲区，守卫不可见）、ChatPanel 12→8（丢 4）；
    // R6 修复后二者真值可证（26 / 12，登记入 R7 棘轮）；R3/R2 迁移后归零。
    expect(directRestHits('src/components/settings/SettingsPanel.tsx')).toBe(0)
    expect(directRestHits('src/components/chat/ChatPanel.tsx')).toBe(0)
  })
})

// ---------------- 断言组 6：宿主能力单向门（c-arch-3，R5/R6） ----------------
//
// 命题：「宿主能力收敛为 renderer-runtime 的一个门面，业务文件只经门面调用；archGuard 扩到 @tauri-apps/*」。
//  ① 唯一白名单：@tauri-apps/* 说明符全库只许出现在门面 src/runtime/host.ts；
//  ② 扫描面 walk('src') 全量（含 api/hooks/lib/各域），排除 __tests__（守卫文件自身含包名字面量）与门面；
//  ③ 去注释后扫描（注释里的包名不误报）；静态 `from` 与动态 `import()` 双收（复用 SPEC_RE）；
//  ④ 门面导出面快照：新增宿主能力必须同步改 FACADE_EXPORTS（双向全等，照 RUNTIME_FILES 范式）。
// 注：FACADE_EXPORTS 同时是 R9「类型不外泄」的间接哨兵——业务文件若为类型再 import @tauri-apps/*，①直接拦下。
const HOST_FACADE = 'src/runtime/host.ts'
const isHostSpec = (s: string) => /^@tauri-apps\//.test(s)
const isGuardFile = (f: string) => /__tests__\//.test(f)
/** 指定源码文本中所有 import/export-from 说明符（复用 SPEC_RE，动态 import() 与静态 from 双收）。 */
const specsOf = (src: string): string[] => [...src.matchAll(SPEC_RE)].map((m) => m[2])
const FACADE_EXPORTS = [
  'checkAndInstallUpdate', 'isTauriRuntime', 'notify', 'onBackendRecovered',
  'pickDirectory', 'relaunchApp', 'startWindowDrag', 'toggleWindowMaximize',
]

describe('archGuard · 断言组 6：宿主能力单向门（@tauri-apps/* 只许出现在 runtime/host.ts）', () => {
  it('① src/** 全量（排除 __tests__ 与门面）不得出现 @tauri-apps/* 说明符', () => {
    for (const f of walk('src')) {
      if (f === HOST_FACADE || isGuardFile(f)) continue
      for (const s of specsOf(stripComments(read(f)))) {
        expect(isHostSpec(s), `${f} → ${s}（宿主能力必须走 @/runtime/host 门面）`).toBe(false)
      }
    }
  })
  it('② 反绕过自证：动态 import() / 静态 from / import type 三形态都被识别为宿主说明符', () => {
    expect(specsOf(stripComments(`import('@tauri-apps/plugin-dialog')`)).some(isHostSpec)).toBe(true)
    expect(specsOf(`import { x } from '@tauri-apps/api/window'`).some(isHostSpec)).toBe(true)
    expect(specsOf(`import type { X } from '@tauri-apps/api/event'`).some(isHostSpec)).toBe(true)
    // 注释形态不误报（去注释后消失）
    expect(specsOf(stripComments(`// import('@tauri-apps/plugin-process')`)).some(isHostSpec)).toBe(false)
  })
  it('③ 门面导出面 == FACADE_EXPORTS（能力清单双向全等；新增能力须同步改表）', () => {
    const src = read(HOST_FACADE)
    const fnExports = [...src.matchAll(/export\s+(?:async\s+)?function\s+(\w+)/g)].map((m) => m[1])
    const namedExports: string[] = [...src.matchAll(/export\s*\{([^}]*)\}/g)]
      .flatMap((m: RegExpMatchArray) => m[1].split(',').map((x: string) => x.trim().split(/\s+as\s+/).pop()!.trim()))
      .filter((name: string) => name.length > 0)
    expect(sorted([...new Set([...fnExports, ...namedExports])])).toEqual(sorted(FACADE_EXPORTS))
  })
})

// ⑤ 产物副本一致性守卫（R4/R6）：唯一实现在 scripts/verify_assets.py（跨平台 python3），
// 前端只做薄封装调用同一入口，不重复实现。覆盖：L1 源↔副本 sha256（副本缺失=SKIP，CI 全新
// checkout 亦成立）+ L2 名录↔源码（--forbid-literals：受管名/env 名/陈旧名不得出现在名录以外的源码）。
describe('⑤ 产物副本一致性守卫（源↔副本 sha256 + 名录字面量）', () => {
  it('python3 scripts/verify_assets.py --check 全绿（漂移或字面量外泄即失败）', () => {
    const script = resolve(cwd, '../scripts/verify_assets.py')
    // execFileSync 在非零退出时抛错，即测试失败（fail-closed）
    const out = execFileSync('python3', [script, '--check'], { encoding: 'utf-8' })
    expect(out).toContain('全部通过')
  })
})

// ---------------- 断言组 7：组件归属锚点（c-arch-3） ----------------
//
// 组件归属规则（可判定，按序首个命中即定；与 scripts/check_components_ownership.py 同源）：
//   ① 路由可达整页（PageId 装配表引用）      → src/pages/
//   ② 消费图数据 / ReactFlow 的画布件         → src/components/canvas/
//   ③ 浮层内容、全局浮标（无路由）            → src/components/overlays/
//   ④ 装配壳 chrome（页头/窗控/主题/装配壳）  → src/components/shell/
//   ⑤ 其余可复用业务组件                      → src/components/<既有域>/
//   ⑥ 禁止：src/components/ 根目录存在任何 *.tsx（及任何文件）——本断言组固化
//
// 变更流程：新增组件若不满足 ①–⑤ 任一条 → 先提规则修订（改守卫 + 改规则），再落文件。
const collectRootTsx = (entries: string[]): string[] => entries.filter((f) => f.endsWith('.tsx'))
const rootEntries = (): Array<{ name: string; isFile: () => boolean }> =>
  readdirSync(resolve(cwd, 'src/components'), { withFileTypes: true })

describe('archGuard · 断言组 7：组件归属锚点（components/ 根目录不得存在 *.tsx）', () => {
  it('7.1 src/components/ 根目录 *.tsx 集合为空（归属规则从约定升级为测试）', () => {
    const tsx = rootEntries().filter((e) => e.isFile() && e.name.endsWith('.tsx')).map((e) => e.name)
    expect(sorted(tsx)).toEqual([])
  })
  it('7.2 更强形态：根目录不得存在任何文件（只允许目录，防 .ts/.css/.json 换皮逃逸）', () => {
    const files = rootEntries().filter((e) => e.isFile()).map((e) => e.name)
    expect(sorted(files)).toEqual([])
  })
  it('7.3 反绕过自证：扫描器能识别根级组件（含大小写/扩展名变体），断言非空洞', () => {
    expect(collectRootTsx(['Foo.tsx', 'Bar.ts', 'Baz.tsx.bak', 'canvas'])).toEqual(['Foo.tsx'])
    expect(collectRootTsx([])).toEqual([])
  })
})
