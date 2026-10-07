// 设置面板拆分产物：共享类型 / 分区表 / 样式常量（纯 .ts，无组件导出）。
// 拆自 SettingsPanel.tsx（2026-10-05 防膨胀）。组件型共享件见 ./controls——
// 二者分开是为了满足 react-refresh/only-export-components（审查非阻断 #3）。
// i18n 第三批：用户可见的槽位/范围文案不在此存中文字面量，只存字典 key（渲染期经 t 解析）。
import { Bot, Info, ShieldCheck, SlidersHorizontal, Terminal } from 'lucide-react'

export interface Service {
  id: string
  name: string
  baseUrl: string
  model: string
  apiKey: string
}

/** 功能槽位 → 字典 key（label/desc 渲染期 t(`settings.slot.*`) 解析）。 */
export const SLOTS: [string, string, string][] = [
  ['induction', 'settings.slot.induction', 'settings.slot.inductionHint'],
  ['patrol', 'settings.slot.patrol', 'settings.slot.patrolHint'],
  ['chat', 'settings.slot.chat', 'settings.slot.chatHint'],
]

export const SECTIONS = [
  { id: 'agent', label: '执行 agent', icon: Terminal, hint: 'CLI agent 命令与参数' },
  { id: 'services', label: '模型服务', icon: Bot, hint: 'LLM 服务与槽位绑定' },
  { id: 'harness', label: 'Harness', icon: ShieldCheck, hint: '自定义补充：追加团队规则' },
  { id: 'advanced', label: '高级参数', icon: SlidersHorizontal, hint: '上下文预算与自动巡检' },
  { id: 'about', label: '关于', icon: Info, hint: '版本与运行环境' },
] as const

export type SectionId = (typeof SECTIONS)[number]['id']

/** harness 规则生效范围 → 字典 key（label/desc 渲染期 t(`settings.scope.*`) 解析）。 */
export const SCOPES: { key: string; file: string; labelKey: string; descKey: string }[] = [
  { key: 'global', file: 'global.md', labelKey: 'settings.scope.global', descKey: 'settings.scope.globalHint' },
  { key: 'analysis', file: 'rule_analysis.md', labelKey: 'settings.scope.analysis', descKey: 'settings.scope.analysisHint' },
  { key: 'design', file: 'rule_design.md', labelKey: 'settings.scope.design', descKey: 'settings.scope.designHint' },
  { key: 'implement', file: 'rule_implement.md', labelKey: 'settings.scope.implement', descKey: 'settings.scope.implementHint' },
  { key: 'review', file: 'rule_review.md', labelKey: 'settings.scope.review', descKey: 'settings.scope.reviewHint' },
]

export interface CustomSlot {
  path: string
  slot: string
  exists: boolean
  size: number
  mtimeMs: number
  enabled: boolean
}

export const field =
  'w-full rounded-md border bg-slate-50 dark:bg-slate-950/70 px-2.5 py-1.5 text-[12px] text-slate-700 dark:text-slate-200 outline-none transition-colors focus:border-blue-300 focus:bg-white'
