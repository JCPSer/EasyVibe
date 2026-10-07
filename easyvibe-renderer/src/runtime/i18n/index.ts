// 轻量 i18n 框架 · **取词契约**（词表数据已按域分片：见 ./dict/*.ts）。
// 设计：模块级可变语言状态——React 组件（useLang）与非 React 调用方（toast、lib 函数）
// 共用同一份当前语言；本文件只含契约（74 行逻辑 + 分片聚合 import），**不随翻译批次增长**。
// 词表分片由 ./dict/<domain>.ts 承载（zh/en 同片，按顶层命名空间切分）。
// 查找顺序：dict[lang][key] ?? zhDict[key] ?? key 本身（永不空白）；缺失 key console.warn 一次。
import { useSyncExternalStore } from 'react'

import { zh as zhPages, en as enPages } from './dict/pages'
import { zh as zhCanvas, en as enCanvas } from './dict/canvas'
import { zh as zhTask, en as enTask } from './dict/task'
import { zh as zhSettings, en as enSettings } from './dict/settings'
import { zh as zhChat, en as enChat } from './dict/chat'
import { zh as zhShell, en as enShell } from './dict/shell'
import { zh as zhOnboarding, en as enOnboarding } from './dict/onboarding'
import { zh as zhViews, en as enViews } from './dict/views'
import { zh as zhReport, en as enReport } from './dict/report'
import { zh as zhHooks, en as enHooks } from './dict/hooks'
import { zh as zhCommon, en as enCommon } from './dict/common'
import { zh as zhTop, en as enTop } from './dict/top'
import { zh as zhDeps, en as enDeps } from './dict/deps'
import { zh as zhGit, en as enGit } from './dict/git'
import { zh as zhAttention, en as enAttention } from './dict/attention'
import { zh as zhMisc, en as enMisc } from './dict/misc'

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

// ---------------- 分片词表聚合（每片 zh/en 同片；Object.assign 一次并表） ----------------

const zhAll = Object.assign({}, zhPages, zhCanvas, zhTask, zhSettings, zhChat, zhShell, zhOnboarding, zhViews, zhReport, zhHooks, zhCommon, zhTop, zhDeps, zhGit, zhAttention, zhMisc)
const enAll = Object.assign({}, enPages, enCanvas, enTask, enSettings, enChat, enShell, enOnboarding, enViews, enReport, enHooks, enCommon, enTop, enDeps, enGit, enAttention, enMisc)

/** 字典导出（测试用：zh/en key 集合全等断言锁死半边翻译）。 */
export const zhDict: Readonly<Record<string, string>> = zhAll
export const enDict: Readonly<Record<string, string>> = enAll

const dict: Record<Lang, Readonly<Record<string, string>>> = { zh: zhDict, en: enDict }

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
