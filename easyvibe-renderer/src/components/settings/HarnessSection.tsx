// Harness 分区：团队规则补充（总开关 + AI/手动生成 + 导出/清空/生效范围）。
// 拆自 SettingsPanel.tsx（2026-10-05 防膨胀）。
import { useCallback, useEffect, useState } from 'react'
import { Loader2, Pencil, Save, Sparkles } from 'lucide-react'
import { toast } from '@/runtime/toast'
import { SCOPES, type CustomSlot } from './common'
import { IosToggle } from './controls'

/// Harness 分区（v4 黑盒 2026-10-05 用户裁定）：规则本体完全不展示——
/// 出厂 harness 是只读圣物（任何方式不动原规则）；用户只有「增」的权限：
/// 五个 harness 类型（global/analysis/design/implement/review）各加各的规则。
/// 界面 = 一个总开关 + 一个 AI 生成按钮 + 排查兜底微链接（导出/清空/生效范围）。
export function HarnessSection({ about, onVersionChange }: { about: { backend: string; harness: string } | null; onVersionChange: (v: string) => void }) {
  const [slots, setSlots] = useState<CustomSlot[] | null>(null)
  const [ai, setAi] = useState<{ scope: string; description: string; mode: 'ai' | 'manual'; manual: string; generating: boolean } | null>(null)
  const [showScopes, setShowScopes] = useState(false)
  const [confirmClear, setConfirmClear] = useState(false)

  const load = useCallback(() => {
    fetch('/api/harness/custom/files')
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { slots?: CustomSlot[] } } | null) => setSlots(d?.data?.slots ?? []))
      .catch(() => setSlots([]))
    fetch('/api/harness')
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { manifest?: { version?: string } } } | null) => {
        const v = d?.data?.manifest?.version
        if (v) onVersionChange(v)
      })
      .catch(() => null)
  }, [onVersionChange])
  useEffect(load, [load])

  const slotOf = (key: string) => slots?.find((s) => s.slot === key)
  // 总开关 = 五个类型的槽全部启用（legacy development 也随总开关）
  const allKeys = [...SCOPES.map((s) => s.key), 'development']
  const masterOn = slots !== null && allKeys.every((k) => (slots.find((s) => s.slot === k)?.enabled ?? true))

  const toggleMaster = (on: boolean) => {
    Promise.all(
      allKeys.map((k) =>
        fetch('/api/harness/custom/toggle', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ path: slotFile(k), enabled: on }),
        })
      )
    )
      .then(() => {
        toast(on ? '团队规则已启用——立即注入对应 agent 上下文' : '团队规则已停用（规则保留，随时恢复）', 'info')
        load()
      })
      .catch(() => toast('开关切换失败', 'error'))
  }

  const slotFile = (key: string) => SCOPES.find((s) => s.key === key)?.file ?? 'rule_development.md'

  const activeRules = (slots ?? []).filter((s) => s.exists && s.slot !== 'development')
  const lastGen = activeRules.reduce((m, s) => Math.max(m, s.mtimeMs), 0)

  const openAi = (scope: string) => setAi({ scope, description: '', mode: 'ai', manual: '', generating: false })

  const saveRule = (scopeKey: string, content: string, label: string) =>
    fetch('/api/harness/custom/file', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: slotFile(scopeKey), content }),
    })
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then(() => {
        toast(`「${label}」规则已生效——${SCOPES.find((s) => s.key === scopeKey)?.desc ?? ''}`, 'info')
        setAi(null)
        load()
      })

  const runAi = () => {
    if (!ai || !ai.description.trim()) return
    setAi({ ...ai, generating: true })
    const scope = SCOPES.find((s) => s.key === ai.scope) ?? SCOPES[0]
    fetch('/api/harness/custom/generate', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ slot: scope.key, description: ai.description.trim() }),
    })
      .then((r) => (r.ok ? r.json() : r.json().then((e: { error?: string }) => Promise.reject(new Error(e?.error ?? String(r.status))))))
      .then((d: { data?: { content?: string } }) => saveRule(scope.key, d?.data?.content ?? '', scope.label))
      .catch((e: Error) => toast(`生成失败：${e.message}`, 'error'))
      .finally(() => setAi((a) => (a ? { ...a, generating: false } : null)))
  }

  const saveManual = () => {
    if (!ai || !ai.manual.trim()) return
    const scope = SCOPES.find((s) => s.key === ai.scope) ?? SCOPES[0]
    setAi({ ...ai, generating: true })
    saveRule(scope.key, ai.manual.trim(), scope.label)
      .catch((e: Error) => toast(`保存失败：${e.message}`, 'error'))
      .finally(() => setAi((a) => (a ? { ...a, generating: false } : null)))
  }

  const exportRules = () => {
    const existing = (slots ?? []).filter((s) => s.exists)
    if (existing.length === 0) {
      toast('还没有任何自定义规则', 'info')
      return
    }
    Promise.all(
      existing.map((s) =>
        fetch(`/api/harness/custom/file?path=${encodeURIComponent(s.path)}`)
          .then((r) => (r.ok ? r.json() : null))
          .then((d: { data?: { content?: string } } | null) => `## ${s.path}\n\n${d?.data?.content ?? ''}`)
      )
    )
      .then((sections) => {
        const blob = new Blob([`# EasyVibe 团队自定义规则导出\n\n${sections.join('\n\n---\n\n')}\n`], { type: 'text/markdown' })
        const a = document.createElement('a')
        a.href = URL.createObjectURL(blob)
        a.download = 'easyvibe-custom-rules.md'
        a.click()
        URL.revokeObjectURL(a.href)
        toast('已导出当前自定义规则', 'info')
      })
      .catch(() => toast('导出失败', 'error'))
  }

  const clearRules = () => {
    const existing = (slots ?? []).filter((s) => s.exists)
    Promise.all(existing.map((s) => fetch(`/api/harness/custom/file?path=${encodeURIComponent(s.path)}`, { method: 'DELETE' })))
      .then(() => {
        toast('已清空全部自定义规则——agent 上下文恢复为纯出厂规则', 'info')
        setConfirmClear(false)
        load()
      })
      .catch(() => toast('清空失败', 'error'))
      .finally(() => setConfirmClear(false))
  }

  return (
    <div className="space-y-3">
      {/* 头部：标题 + 副文案 + 出厂版本（规则本体一律不展示） */}
      <div className="flex items-start justify-between">
        <div>
          <h3 className="text-[15px] font-bold text-slate-800 dark:text-slate-100">Harness 团队规则</h3>
          <p className="text-cap mt-0.5 leading-4 text-slate-400 dark:text-slate-500">
            出厂工作流护栏随软件版本自动更新、不可见不可改；
            <br />
            你可以用 AI 按环节生成团队规则补充，追加到对应 agent 上下文
          </p>
        </div>
        <span className="shrink-0 rounded-full bg-blue-50 dark:bg-blue-950/40 px-2.5 py-1 text-micro font-bold text-blue-600">
          出厂版本 v{about?.harness ?? '…'}
        </span>
      </div>

      {/* 开关卡：总开关（停用≠删除） */}
      <div className="flex items-center gap-3 rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-4 py-3.5">
        <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-emerald-50 dark:bg-emerald-950/40 text-[16px]">🛡</span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-[13px] font-bold text-slate-700 dark:text-slate-200">
            启用团队规则补充
            {slots !== null && (
              <span className={`rounded-full px-2 py-0.5 text-micro font-bold ${masterOn ? 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-600' : 'bg-slate-100 dark:bg-slate-800 text-slate-400'}`}>
                {masterOn ? '● 生效中' : '已停用'}
              </span>
            )}
          </div>
          <p className="text-cap mt-1 leading-4 text-slate-400 dark:text-slate-500">
            开启后，你的补充规则追加在出厂规则之后，对应环节的 agent 同时看到两者，冲突时以补充为准。随时可关闭——规则保留，重开立即恢复。
          </p>
        </div>
        <IosToggle on={masterOn} onChange={toggleMaster} />
      </div>

      {/* AI 生成卡 */}
      <div className="flex items-center gap-3 rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-4 py-3.5">
        <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-gradient-to-br from-violet-500/15 to-indigo-500/15 text-[16px]">✨</span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-[13px] font-bold text-slate-700 dark:text-slate-200">
            团队规则
            {activeRules.length > 0 ? (
              <span className="rounded-full bg-emerald-50 dark:bg-emerald-950/40 px-2 py-0.5 text-micro font-bold text-emerald-600">已生成 · {activeRules.length} 类</span>
            ) : (
              <span className="rounded-full bg-slate-100 dark:bg-slate-800 px-2 py-0.5 text-micro font-bold text-slate-400">未生成</span>
            )}
          </div>
          <p className="text-cap mt-1 leading-4 text-slate-400 dark:text-slate-500">
            {activeRules.length > 0
              ? `上次生成：${lastGen ? new Date(lastGen).toLocaleString() : '—'} · 规则原文不在界面展示，不满意可重新生成，或用上方开关整体停用`
              : '用自然语言描述你的团队要求（如审查维度、方案必备章节），AI 起草规则并立即生效'}
          </p>
        </div>
        <button
          onClick={() => openAi('global')}
          className="flex shrink-0 items-center gap-1.5 rounded-lg bg-gradient-to-br from-violet-500 to-indigo-500 px-4 py-2.5 text-[12px] font-bold text-white shadow-md shadow-violet-500/25 hover:from-violet-600 hover:to-indigo-600"
        >
          <Sparkles size={13} /> {activeRules.length > 0 ? '管理规则' : '生成规则'}
        </button>
      </div>

      {/* 排查兜底微链接 */}
      <div className="flex items-center gap-3 px-1">
        <button onClick={exportRules} className="text-micro text-slate-300 underline decoration-slate-200 underline-offset-2 hover:text-slate-500 dark:text-slate-600 dark:hover:text-slate-400">
          导出当前规则
        </button>
        <span className="text-slate-200 dark:text-slate-700">·</span>
        {confirmClear ? (
          <button onClick={clearRules} className="text-micro font-bold text-red-500 hover:text-red-600">
            确认清空？
          </button>
        ) : (
          <button onClick={() => setConfirmClear(true)} onMouseLeave={() => setConfirmClear(false)} className="text-micro text-slate-300 underline decoration-slate-200 underline-offset-2 hover:text-red-500 dark:text-slate-600">
            清空自定义规则
          </button>
        )}
        <span className="text-slate-200 dark:text-slate-700">·</span>
        <button onClick={() => setShowScopes((v) => !v)} className="text-micro text-slate-300 underline decoration-slate-200 underline-offset-2 hover:text-slate-500 dark:text-slate-600 dark:hover:text-slate-400">
          查看生效范围
        </button>
      </div>

      {/* 生效范围面板：只显示状态点，不展示规则内容 */}
      {showScopes && (
        <div className="rounded-md border border-slate-200 dark:border-slate-700 bg-slate-50/60 dark:bg-slate-900/60 px-4 py-3">
          {SCOPES.map((s) => {
            const sl = slotOf(s.key)
            const on = sl?.exists && sl.enabled
            return (
              <div key={s.key} className="flex items-center gap-2 py-1.5">
                <span className={`h-1.5 w-1.5 rounded-full ${on ? 'bg-emerald-500' : 'bg-slate-300 dark:bg-slate-600'}`} />
                <span className="w-16 text-[11px] font-semibold text-slate-600 dark:text-slate-300">{s.label}</span>
                <span className="text-cap text-slate-400 dark:text-slate-500">{s.desc}</span>
                <span className="ml-auto text-micro text-slate-300 dark:text-slate-600">{on ? '已生效' : '未设置'}</span>
              </div>
            )
          })}
          {slotOf('development')?.exists && (
            <div className="flex items-center gap-2 border-t border-slate-100 dark:border-slate-800 py-1.5 mt-1">
              <span className="h-1.5 w-1.5 rounded-full bg-amber-400" />
              <span className="w-16 text-[11px] font-semibold text-slate-500 dark:text-slate-400">兜底规则</span>
              <span className="text-cap text-slate-400 dark:text-slate-500">旧版通用补充（新规则未覆盖的环节回落使用）</span>
            </div>
          )}
        </div>
      )}

      {/* 规则弹窗：选择环节 → AI 生成 或 手动输入 → 保存即生效 */}
      {ai && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 backdrop-blur-[2px]" onClick={() => setAi(null)}>
          <div className="w-[560px] max-w-[92vw] rounded-xl bg-white dark:bg-slate-900 shadow-2xl overflow-hidden" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center gap-2 px-4 pt-4">
              <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-gradient-to-br from-violet-500 to-indigo-500 text-white">
                {ai.mode === 'ai' ? <Sparkles size={14} /> : <Pencil size={13} />}
              </span>
              <b className="text-[13px] text-slate-800 dark:text-slate-100">{ai.mode === 'ai' ? 'AI 生成团队规则' : '手动编写团队规则'}</b>
              {slotOf(ai.scope)?.exists && <span className="rounded bg-red-50 dark:bg-red-950/40 px-1.5 py-0.5 text-micro font-bold text-red-500">将替换该范围现有规则</span>}
            </div>
            {/* 方式切换：AI 起草 / 自己的文本 */}
            <div className="flex gap-1.5 px-4 pt-3">
              {([
                ['ai', '✨ AI 生成'],
                ['manual', '✍️ 手动输入'],
              ] as const).map(([m, label]) => (
                <button
                  key={m}
                  onClick={() => setAi({ ...ai, mode: m })}
                  className={`rounded-full border px-3 py-1.5 text-micro font-bold transition-colors ${
                    ai.mode === m
                      ? 'border-violet-400 bg-violet-50 dark:bg-violet-950/40 text-violet-700 dark:text-violet-300'
                      : 'border-slate-200 dark:border-slate-700 text-slate-400 hover:border-violet-200 hover:text-violet-500'
                  }`}
                >
                  {label}
                </button>
              ))}
            </div>
            {/* 环节选择：五个 harness 类型 */}
            <div className="flex flex-wrap gap-1.5 px-4 pt-2.5">
              {SCOPES.map((s) => (
                <button
                  key={s.key}
                  onClick={() => setAi({ ...ai, scope: s.key })}
                  className={`rounded-full border px-3 py-1.5 text-micro font-bold transition-colors ${
                    ai.scope === s.key
                      ? 'border-violet-400 bg-violet-50 dark:bg-violet-950/40 text-violet-700 dark:text-violet-300'
                      : 'border-slate-200 dark:border-slate-700 text-slate-400 hover:border-violet-200 hover:text-violet-500'
                  }`}
                >
                  {s.label}
                </button>
              ))}
            </div>
            <p className="px-4 pt-2 text-micro leading-4 text-slate-400 dark:text-slate-500">
              {SCOPES.find((s) => s.key === ai.scope)?.desc}——规则原文不在界面展示；与出厂规则冲突时以补充为准。
            </p>
            {ai.mode === 'ai' ? (
              <textarea
                autoFocus
                value={ai.description}
                onChange={(e) => setAi({ ...ai, description: e.target.value })}
                rows={5}
                placeholder="例：我们团队做金融系统。所有数据库查询必须参数化；代码审查必须额外检查越权访问和敏感数据日志；方案文档必须包含性能影响与回滚方式两节……"
                className="mx-4 mt-3 w-[calc(100%-2rem)] resize-none rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-950/60 p-3 text-[12px] leading-5 text-slate-700 dark:text-slate-200 outline-none focus:ring-2 focus:ring-violet-500/30"
              />
            ) : (
              <textarea
                autoFocus
                value={ai.manual}
                onChange={(e) => setAi({ ...ai, manual: e.target.value })}
                rows={9}
                placeholder={'直接粘贴或编写你的规则文本（Markdown），例如：\n1. 所有公开函数必须有文档注释。\n2. 代码审查必须检查 SQL 注入与越权访问。\n3. 方案文档必须包含「性能影响」与「回滚方式」两节。'}
                className="mono mx-4 mt-3 w-[calc(100%-2rem)] resize-y rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-950/60 p-3 text-[11.5px] leading-5 text-slate-700 dark:text-slate-200 outline-none focus:ring-2 focus:ring-violet-500/30"
              />
            )}
            <div className="flex items-center gap-2 px-4 py-3.5">
              <span className="text-micro text-slate-300 dark:text-slate-600">
                {ai.mode === 'ai' ? '将由已配置的 LLM 生成 · 生成即生效' : '保存即生效 · 仅追加为补充，不影响出厂规则'}
              </span>
              <button onClick={() => setAi(null)} className="ml-auto rounded-md border border-slate-200 dark:border-slate-700 px-3 py-1.5 text-micro font-semibold text-slate-500 dark:text-slate-400 hover:text-slate-700">
                取消
              </button>
              <button
                onClick={ai.mode === 'ai' ? runAi : saveManual}
                disabled={ai.generating || (ai.mode === 'ai' ? !ai.description.trim() : !ai.manual.trim())}
                className="flex items-center gap-1 rounded-md bg-gradient-to-br from-violet-500 to-indigo-500 px-3.5 py-1.5 text-micro font-bold text-white hover:from-violet-600 hover:to-indigo-600 disabled:opacity-40"
              >
                {ai.generating ? <Loader2 size={11} className="animate-spin" /> : ai.mode === 'ai' ? <Sparkles size={11} /> : <Save size={11} />}
                {ai.mode === 'ai' ? '生成并生效' : '保存并生效'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
