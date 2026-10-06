// 轻量 i18n 框架（英文化第一批：框架 + 应用骨架）。
// 设计：模块级可变语言状态——React 组件（useLang）与非 React 调用方（toast、lib 函数）
// 共用同一份当前语言；字典内嵌本文件（扁平点分 key，按域分节），后续批次只追加 key。
// 查找顺序：dict[lang][key] ?? dict.zh[key] ?? key 本身（永不空白）；缺失 key console.warn 一次。
import { useSyncExternalStore } from 'react'

export type Lang = 'zh' | 'en'

const STORAGE_KEY = 'easyvibe.lang'

// node（vitest）环境无 localStorage——可空访问，初始检测降级为 navigator 判定。
const storage = () => (typeof localStorage === 'undefined' ? null : localStorage)

const detect = (): Lang => {
  const saved = storage()?.getItem(STORAGE_KEY)
  if (saved === 'zh' || saved === 'en') return saved
  const nav = typeof navigator === 'undefined' ? '' : navigator.language
  return nav.toLowerCase().startsWith('zh') ? 'zh' : 'en'
}

let current: Lang = detect()
const listeners = new Set<(lang: Lang) => void>()
const warned = new Set<string>()

// ---------------- 字典（zh 为基准；en 以 Record<keyof zh> 强约束 key 对齐） ----------------

const zh = {
  // shell.nav：左侧导航（分组头 + 条目 + 折叠把手）
  'shell.nav.group.explore': '探索',
  'shell.nav.group.workspace': '工作区',
  'shell.nav.group.kb': '知识库',
  'shell.nav.map': '架构地图',
  'shell.nav.modules': '模块目录',
  'shell.nav.deps': '依赖关系',
  'shell.nav.drift': '漂移洞察',
  'shell.nav.health': '健康看板',
  'shell.nav.workbench': '任务对话',
  'shell.nav.tasks': '任务',
  'shell.nav.runs': '运行',
  'shell.nav.usage': '用量',
  'shell.nav.changes': '变更记录',
  'shell.nav.git': 'Git',
  'shell.nav.kb-docs': '文档中心',
  'shell.nav.kb-decisions': '决策记录',
  'shell.nav.kb-apis': '接口目录',
  'shell.nav.settings': '设置',
  'shell.nav.collapse': '收起',
  'shell.nav.collapseTip': '收起导航',
  'shell.nav.expandTip': '展开导航',
  // shell.topbar：顶栏全局动作
  'shell.topbar.views': '视图',
  'shell.topbar.viewsTip': '我的视图（对话沉淀的图资产）',
  'shell.topbar.suggest': '优化建议',
  'shell.topbar.suggestTip': 'AI 主动发现优化建议，逐条可发起修复',
  'shell.topbar.patrol': '巡检',
  'shell.topbar.patrolling': '巡检中…',
  'shell.topbar.patrolTip': '巡检：Supervisor 直调 LLM（带健康基线），产出新地图并落健康历史',
  'shell.topbar.patrolDisabledTip': '未检测到执行 agent——先安装或在设置中配置',
  'shell.topbar.export': '导出',
  'shell.topbar.exportTip': '导出架构健康报告（Markdown，零 token 成本）',
  'shell.topbar.wsReconnect': '重连中',
  'shell.topbar.wsReconnectTip': '与后端的实时连接已断开，正在自动重连；页面数据走 HTTP 轮询兜底（实时性降级）',
  'shell.topbar.welcomeTip': '新手引导（欢迎页 + 上手指引）——再点关闭',
  'shell.topbar.settingsTip': '设置（LLM 服务 / 槽位绑定 / 高级）',
  'shell.topbar.toLight': '切换到亮色模式',
  'shell.topbar.toDark': '切换到暗黑模式',
  // shell.bubble：全局运行会话指示器（顶栏正中状态丸 + 悬停卡）
  'shell.bubble.viewRunsTip': '查看 agent 流水（跨仓库；点击定位到当前会话）',
  'shell.bubble.failed': '{label} · 失败',
  'shell.bubble.manyRunning': '{count} 个会话进行中',
  'shell.bubble.running': '{label} · 进行中',
  'shell.bubble.queued': '排队中：{label}',
  'shell.bubble.elapsed': '已运行 {elapsed}',
  'shell.bubble.runningShort': '进行中…',
  'shell.bubble.sessionEnded': '会话已结束',
  'shell.bubble.autoStart': '结束后自动开始',
  'shell.bubble.queuedChip': '排队:{label}',
  'shell.bubble.cancelQueueTip': '取消排队',
  'shell.bubble.tasksQueued': '任务排队×{count}',
  'shell.bubble.tasksQueuedTip': '有任务在排队等待执行槽位（并发满自动开始）——点击查看任务',
  'shell.bubble.cardTitle': '运行中的会话（{count}）',
  'shell.bubble.cancelSession': '取消该会话',
  'shell.bubble.cancelSessionTip': '取消该会话（二次确认）',
  'shell.bubble.startedAt': '启动时间',
  'shell.bubble.sessionFailed': '「{label}」执行失败',
  'shell.bubble.killConfirm': '确定取消「{label}」（{repo}）？该操作不可撤销。',
  'shell.bubble.killOk': '已取消「{label}」',
  'shell.bubble.killErr': '取消失败（请确认后端在线后重试）。',
  'shell.bubble.queueNone': '没有排队任务',
  'shell.bubble.queueCancelFailed': '取消排队失败（HTTP {status}）',
  'shell.bubble.queueCancelled': '已取消排队',
  'shell.bubble.queueCancelErr': '取消排队失败（请确认后端在线后重试）。',
  // settings.*：设置面板外壳（分区标题 + 通用控件；分区内部细节文案属后续批）
  'settings.section.agent': '执行 agent',
  'settings.section.agentHint': 'CLI agent 命令与参数',
  'settings.section.services': '模型服务',
  'settings.section.servicesHint': 'LLM 服务与槽位绑定',
  'settings.section.harness': 'Harness',
  'settings.section.harnessHint': '自定义补充：追加团队规则',
  'settings.section.advanced': '高级参数',
  'settings.section.advancedHint': '上下文预算与自动巡检',
  'settings.section.about': '关于',
  'settings.section.aboutHint': '版本与运行环境',
  'settings.shell.unsaved': '未保存',
  'settings.shell.discard': '放弃更改',
  'settings.shell.loading': '加载配置…',
  'settings.shell.save': '保存配置',
  'settings.shell.saving': '保存中…',
  'settings.shell.saved': '已是最新',
  'settings.shell.backendOffline': '需要本地后端在线',
  'settings.shell.keyEncrypted': 'API Key 加密存储',
  'settings.shell.keyLocalOnly': '仅本机可解密',
  // common.*：跨域通用
  'common.language': '语言',
} as const

const en: Record<keyof typeof zh, string> = {
  'shell.nav.group.explore': 'Explore',
  'shell.nav.group.workspace': 'Workspace',
  'shell.nav.group.kb': 'Knowledge Base',
  'shell.nav.map': 'Architecture Map',
  'shell.nav.modules': 'Modules',
  'shell.nav.deps': 'Dependencies',
  'shell.nav.drift': 'Drift',
  'shell.nav.health': 'Health',
  'shell.nav.workbench': 'Chat',
  'shell.nav.tasks': 'Tasks',
  'shell.nav.runs': 'Runs',
  'shell.nav.usage': 'Usage',
  'shell.nav.changes': 'Changes',
  'shell.nav.git': 'Git',
  'shell.nav.kb-docs': 'Docs',
  'shell.nav.kb-decisions': 'Decisions',
  'shell.nav.kb-apis': 'APIs',
  'shell.nav.settings': 'Settings',
  'shell.nav.collapse': 'Collapse',
  'shell.nav.collapseTip': 'Collapse sidebar',
  'shell.nav.expandTip': 'Expand sidebar',
  'shell.topbar.views': 'Views',
  'shell.topbar.viewsTip': 'Your views — graph assets distilled from conversations',
  'shell.topbar.suggest': 'Suggestions',
  'shell.topbar.suggestTip': 'AI proactively surfaces optimization suggestions — launch a fix from each one',
  'shell.topbar.patrol': 'Patrol',
  'shell.topbar.patrolling': 'Patrolling…',
  'shell.topbar.patrolTip': 'Patrol: the supervisor calls the LLM directly (with a health baseline), produces a fresh map and records health history',
  'shell.topbar.patrolDisabledTip': 'No execution agent detected — install one or configure it in Settings',
  'shell.topbar.export': 'Export',
  'shell.topbar.exportTip': 'Export the architecture health report (Markdown, zero token cost)',
  'shell.topbar.wsReconnect': 'Reconnecting',
  'shell.topbar.wsReconnectTip': 'Realtime connection to the backend is down — reconnecting automatically. Pages fall back to HTTP polling (less fresh).',
  'shell.topbar.welcomeTip': 'Onboarding (welcome page + quick-start guide) — click again to close',
  'shell.topbar.settingsTip': 'Settings (LLM services / slot bindings / advanced)',
  'shell.topbar.toLight': 'Switch to light mode',
  'shell.topbar.toDark': 'Switch to dark mode',
  'shell.bubble.viewRunsTip': 'View agent activity (across repos; click to locate this session)',
  'shell.bubble.failed': '{label} · failed',
  'shell.bubble.manyRunning': '{count} sessions running',
  'shell.bubble.running': '{label} · running',
  'shell.bubble.queued': 'Queued: {label}',
  'shell.bubble.elapsed': '{elapsed} elapsed',
  'shell.bubble.runningShort': 'Running…',
  'shell.bubble.sessionEnded': 'Session ended',
  'shell.bubble.autoStart': 'Auto-starts when the current one ends',
  'shell.bubble.queuedChip': 'Queue: {label}',
  'shell.bubble.cancelQueueTip': 'Cancel queue',
  'shell.bubble.tasksQueued': '{count} tasks queued',
  'shell.bubble.tasksQueuedTip': 'Tasks waiting for a free execution slot (they start automatically) — click to view tasks',
  'shell.bubble.cardTitle': '{count} sessions running',
  'shell.bubble.cancelSession': 'Cancel session',
  'shell.bubble.cancelSessionTip': 'Cancel this session (asks for confirmation)',
  'shell.bubble.startedAt': 'Started',
  'shell.bubble.sessionFailed': '"{label}" failed',
  'shell.bubble.killConfirm': 'Cancel "{label}" ({repo})? This cannot be undone.',
  'shell.bubble.killOk': 'Cancelled "{label}"',
  'shell.bubble.killErr': 'Cancel failed — make sure the backend is online and retry.',
  'shell.bubble.queueNone': 'Nothing in the queue',
  'shell.bubble.queueCancelFailed': 'Failed to cancel the queue (HTTP {status})',
  'shell.bubble.queueCancelled': 'Removed from queue',
  'shell.bubble.queueCancelErr': 'Failed to cancel the queue — make sure the backend is online and retry.',
  'settings.section.agent': 'Agent',
  'settings.section.agentHint': 'CLI agent command & arguments',
  'settings.section.services': 'Model Services',
  'settings.section.servicesHint': 'LLM services & slot bindings',
  'settings.section.harness': 'Harness',
  'settings.section.harnessHint': 'Custom add-ons: append team rules',
  'settings.section.advanced': 'Advanced',
  'settings.section.advancedHint': 'Context budget & auto patrol',
  'settings.section.about': 'About',
  'settings.section.aboutHint': 'Version & runtime',
  'settings.shell.unsaved': 'Unsaved',
  'settings.shell.discard': 'Discard changes',
  'settings.shell.loading': 'Loading settings…',
  'settings.shell.save': 'Save changes',
  'settings.shell.saving': 'Saving…',
  'settings.shell.saved': 'Up to date',
  'settings.shell.backendOffline': 'Local backend must be online',
  'settings.shell.keyEncrypted': 'API Keys stored encrypted',
  'settings.shell.keyLocalOnly': 'decryptable only on this machine',
  'common.language': 'Language',
}

/** 字典导出（测试用：zh/en key 集合全等断言锁死半边翻译）。 */
export const zhDict: Readonly<Record<string, string>> = zh
export const enDict: Readonly<Record<string, string>> = en

const dict: Record<Lang, Readonly<Record<string, string>>> = { zh, en }

// ---------------- API ----------------

export function getLang(): Lang {
  return current
}

export function setLang(lang: Lang): void {
  current = lang
  storage()?.setItem(STORAGE_KEY, lang)
  for (const fn of listeners) fn(lang)
}

export function onLangChange(listener: (lang: Lang) => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function t(key: string, vars?: Record<string, string | number>): string {
  let s = dict[current][key] ?? zhDict[key]
  if (s === undefined) {
    if (!warned.has(key)) {
      warned.add(key)
      console.warn(`[i18n] missing key: ${key}`)
    }
    s = key
  }
  if (vars) {
    for (const [name, value] of Object.entries(vars)) {
      s = s.replaceAll(`{${name}}`, String(value))
    }
  }
  return s
}

/** React 订阅入口：语言切换即时全界面生效（useSyncExternalStore 快照去重，同值不触发渲染）。 */
export function useLang(): { lang: Lang; setLang: typeof setLang; t: typeof t } {
  // 第三个参数 getServerSnapshot：SSR（renderToStaticMarkup 测试）下不抛错，取当前值即可
  const lang = useSyncExternalStore(onLangChange, getLang, getLang)
  return { lang, setLang, t }
}
