// i18n 框架单测（node 环境）：
//  ① zh/en key 集合全等（防半边翻译，锁死）；② 缺失 key 回退中文/自身且不抛错 + warn 一次；
//  ③ 模板变量替换；④ setLang 持久化与订阅通知；⑤ 初始语言检测（已存值 ?? navigator）。
import { beforeEach, describe, expect, it, vi } from 'vitest'

// node 环境无 localStorage——提供内存桩（模块初始检测与 setLang 持久化都消费它）。
const mem = new Map<string, string>()
vi.stubGlobal('localStorage', {
  getItem: (k: string) => mem.get(k) ?? null,
  setItem: (k: string, v: string) => {
    mem.set(k, v)
  },
  removeItem: (k: string) => {
    mem.delete(k)
  },
})

const { t, setLang, getLang, onLangChange, zhDict, enDict } = await import('@/runtime/i18n')

describe('i18n · 字典完整性', () => {
  it('zh/en key 集合全等', () => {
    expect(Object.keys(enDict).sort()).toEqual(Object.keys(zhDict).sort())
  })
  it('每个 key 在两种语言下都解析到非空、非 key 本身的文案', () => {
    for (const lang of ['zh', 'en'] as const) {
      setLang(lang)
      for (const key of Object.keys(zhDict)) {
        const s = t(key)
        expect(s.length, `${lang}:${key}`).toBeGreaterThan(0)
        expect(s, `${lang}:${key}`).not.toBe(key)
      }
    }
  })
})

describe('i18n · 查找回退', () => {
  beforeEach(() => setLang('zh'))
  it('缺失 key 回退 key 本身，不抛错、不空白', () => {
    expect(t('no.such.key')).toBe('no.such.key')
  })
  it('缺失 key console.warn 只报一次', () => {
    const spy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      t('warn.once.key')
      t('warn.once.key')
      expect(spy).toHaveBeenCalledTimes(1)
      expect(spy.mock.calls[0]?.[0]).toContain('warn.once.key')
    } finally {
      spy.mockRestore()
    }
  })
})

describe('i18n · 模板变量', () => {
  it('中文复数形态：{count} 个会话进行中', () => {
    setLang('zh')
    expect(t('shell.bubble.manyRunning', { count: 3 })).toBe('3 个会话进行中')
  })
  it('英文复数形态：{count} sessions running', () => {
    setLang('en')
    expect(t('shell.bubble.manyRunning', { count: 3 })).toBe('3 sessions running')
  })
  it('多变量替换（label + repo）', () => {
    setLang('en')
    expect(t('shell.bubble.killConfirm', { label: 'Patrol', repo: 'demo' })).toBe(
      'Cancel "Patrol" (demo)? This cannot be undone.',
    )
  })
  it('未提供的占位符原样保留（永不空白）', () => {
    setLang('en')
    expect(t('shell.bubble.running', { label: 'Patrol' })).toBe('Patrol · running')
  })
})

describe('i18n · setLang 持久化与订阅', () => {
  it('写 localStorage 并通知订阅者，退订后不再通知', () => {
    const seen: string[] = []
    const off = onLangChange((l) => seen.push(l))
    setLang('en')
    expect(mem.get('easyvibe.lang')).toBe('en')
    expect(seen).toEqual(['en'])
    off()
    setLang('zh')
    expect(mem.get('easyvibe.lang')).toBe('zh')
    expect(seen).toEqual(['en'])
  })
})

describe('i18n · 初始语言检测（已存值 ?? navigator）', () => {
  it('已存值优先', async () => {
    vi.resetModules()
    mem.set('easyvibe.lang', 'en')
    const m = await import('@/runtime/i18n')
    expect(m.getLang()).toBe('en')
  })
  it('无已存值时按 navigator.language 判定', async () => {
    vi.resetModules()
    mem.clear()
    vi.stubGlobal('navigator', { language: 'zh-CN' })
    expect((await import('@/runtime/i18n')).getLang()).toBe('zh')
    vi.resetModules()
    vi.stubGlobal('navigator', { language: 'en-US' })
    expect((await import('@/runtime/i18n')).getLang()).toBe('en')
  })
  it('默认导出实例与全局状态一致', () => {
    expect(getLang()).toBe('zh')
  })
})
