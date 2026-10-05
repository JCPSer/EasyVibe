// 设置面板拆分产物：共享类型 / 分区表 / 样式常量（纯 .ts，无组件导出）。
// 拆自 SettingsPanel.tsx（2026-10-05 防膨胀）。组件型共享件见 ./controls——
// 二者分开是为了满足 react-refresh/only-export-components（审查非阻断 #3）。
import { Bot, Info, ShieldCheck, SlidersHorizontal, Terminal } from 'lucide-react'

export interface Service {
  id: string
  name: string
  baseUrl: string
  model: string
  apiKey: string
}

export const SLOTS: [string, string, string][] = [
  ['induction', '地图归纳', '首次归纳与重新归纳'],
  ['patrol', '巡检', '健康巡检与健康写回'],
  ['chat', '对话', '入口对话与智能建议'],
]

export const SECTIONS = [
  { id: 'agent', label: '执行 agent', icon: Terminal, hint: 'CLI agent 命令与参数' },
  { id: 'services', label: '模型服务', icon: Bot, hint: 'LLM 服务与槽位绑定' },
  { id: 'harness', label: 'Harness', icon: ShieldCheck, hint: '自定义补充：追加团队规则' },
  { id: 'advanced', label: '高级参数', icon: SlidersHorizontal, hint: '上下文预算与自动巡检' },
  { id: 'about', label: '关于', icon: Info, hint: '版本与运行环境' },
] as const

export type SectionId = (typeof SECTIONS)[number]['id']

export const SCOPES: { key: string; label: string; file: string; desc: string }[] = [
  { key: 'global', label: '通用', file: 'global.md', desc: '全部 agent 上下文（任务流水线、归纳、巡检、入口对话）' },
  { key: 'analysis', label: '需求分析', file: 'rule_analysis.md', desc: '仅需求分析阶段（阶段 1 需求矩阵 agent）' },
  { key: 'design', label: '方案设计', file: 'rule_design.md', desc: '仅方案设计阶段（阶段 2 方案设计 agent）' },
  { key: 'implement', label: '代码开发', file: 'rule_implement.md', desc: '仅代码开发阶段（阶段 3 实施 agent）' },
  { key: 'review', label: '代码审查', file: 'rule_review.md', desc: '仅代码审查（独立审查 agent 与阶段产物初审）' },
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
