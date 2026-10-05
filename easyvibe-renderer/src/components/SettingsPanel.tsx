import { useCallback, useEffect, useMemo, useState } from 'react'
import { Check, Loader2, RotateCcw, Save, X } from 'lucide-react'
import { toast } from '@/lib/toast'
import { SECTIONS, SLOTS, type SectionId, type Service } from './settings/common'
import { AgentSection } from './settings/AgentSection'
import { ServicesSection } from './settings/ServicesSection'
import { HarnessSection } from './settings/HarnessSection'
import { AdvancedSection } from './settings/AdvancedSection'
import { AboutSection } from './settings/AboutSection'

interface Props {
  backendRepo: string | null
  onClose: () => void
  /** M4-1：作为设置"页"嵌入应用壳（非滑出抽屉） */
  embedded?: boolean
}

// 设置面板 v2（2026-10-02 UI 专项）：分区导航 + 脏状态追踪 + 字段校验 + 开关组件 + 关于页。
// 本文件为壳：状态装配 + 加载/保存 + 分区调度；各分区见 ./settings/*（2026-10-05 防膨胀拆分）。
// 文案纪律：只说价值与现状，不提内部施工编号。

export function SettingsPanel({ backendRepo, onClose, embedded }: Props) {
  const [section, setSection] = useState<SectionId>('services')
  const [services, setServices] = useState<Record<string, Service>>({})
  const [slots, setSlots] = useState<Record<string, string>>({})
  const [adv, setAdv] = useState({ contextBudget: 256000, maxTokens: 8192, autoPatrolEnabled: false, autoPatrolHours: 24 })
  const [snapshot, setSnapshot] = useState<string>('')
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [about, setAbout] = useState<{ backend: string; harness: string } | null>(null)
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null)
  const [showKey, setShowKey] = useState<Record<string, boolean>>({})

  const load = useCallback(async () => {
    setLoading(true)
    const scopes = ['global', backendRepo].filter(Boolean) as string[]
    const merged: Record<string, unknown> = {}
    for (const scope of scopes) {
      const r = await fetch(`/api/settings?scope=${encodeURIComponent(scope)}`)
      if (!r.ok) continue
      const d = await r.json()
      for (const item of d.data as { key: string; value: unknown }[]) merged[item.key] = item.value
    }
    const svc: Record<string, Service> = {}
    for (const [k, v] of Object.entries(merged)) {
      const m = k.match(/^llm\.service\.([^.]+)$/)
      if (m && typeof v === 'object' && v !== null) {
        const o = v as Record<string, unknown>
        svc[m[1]] = {
          id: m[1],
          name: String(o.name ?? m[1]),
          baseUrl: String(o.baseUrl ?? ''),
          model: String(o.model ?? ''),
          apiKey: String(merged[`llm.service.${m[1]}.apiKey`] ?? ''),
        }
      }
    }
    if (Object.keys(svc).length === 0) {
      svc.default = { id: 'default', name: '默认服务', baseUrl: '', model: '', apiKey: '' }
    }
    const sl: Record<string, string> = {}
    for (const [s] of SLOTS) sl[s] = String(merged[`slot.${s}`] ?? 'default')
    const next = {
      services: svc,
      slots: sl,
      adv: {
        contextBudget: Number(merged['adv.contextBudget'] ?? 256000),
        maxTokens: Number(merged['adv.maxTokens'] ?? 8192),
        autoPatrolEnabled: Boolean(merged['adv.autoPatrolEnabled'] ?? false),
        autoPatrolHours: Number(merged['adv.autoPatrolHours'] ?? 24),
      },
    }
    setServices(next.services)
    setSlots(next.slots)
    setAdv(next.adv)
    setSnapshot(JSON.stringify(next))
    setLoading(false)
  }, [backendRepo])

  useEffect(() => {
    load()
  }, [load])

  // 关于页：后端版本 + harness 版本（只读事实，自动加载）
  useEffect(() => {
    if (section !== 'about') return
    Promise.all([
      fetch('/api/health').then((r) => (r.ok ? r.json() : null)).catch(() => null),
      fetch('/api/harness').then((r) => (r.ok ? r.json() : null)).catch(() => null),
    ]).then(([h, hw]) => {
      setAbout({
        backend: h?.data?.version ?? h?.version ?? '未知',
        harness: hw?.data?.manifest?.version ?? '未安装',
      })
    })
  }, [section])

  const dirty = useMemo(() => JSON.stringify({ services, slots, adv }) !== snapshot, [services, slots, adv, snapshot])

  // 字段校验：保存门前拦截，错误就地显示（企业级表单的最低纪律）
  const errors = useMemo(() => {
    const e: Record<string, string> = {}
    for (const s of Object.values(services)) {
      if (!s.name.trim()) e[`name:${s.id}`] = '名称不能为空'
      if (s.baseUrl && !/^https?:\/\//.test(s.baseUrl.trim())) e[`baseUrl:${s.id}`] = '须以 http(s):// 开头'
      if (Object.values(slots).includes(s.id) && !s.apiKey.trim()) e[`apiKey:${s.id}`] = '被槽位绑定的服务需要 API Key'
    }
    if (adv.contextBudget < 1000) e['contextBudget'] = '至少 1K token'
    if (adv.maxTokens < 256) e['maxTokens'] = '至少 256 token'
    if (adv.autoPatrolEnabled && adv.autoPatrolHours < 1) e['autoPatrolHours'] = '至少 1 小时'
    return e
  }, [services, slots, adv])

  const save = async () => {
    if (Object.keys(errors).length > 0) {
      toast('还有字段未通过校验，请检查标红项', 'error')
      return
    }
    setSaving(true)
    try {
      const puts: Promise<Response>[] = []
      for (const s of Object.values(services)) {
        puts.push(
          fetch('/api/settings/set', {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ scope: 'global', key: `llm.service.${s.id}`, value: { name: s.name, baseUrl: s.baseUrl, model: s.model } }),
          }),
          fetch('/api/settings/set', {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ scope: 'global', key: `llm.service.${s.id}.apiKey`, value: s.apiKey }),
          }),
        )
      }
      for (const [s] of SLOTS) {
        puts.push(
          fetch('/api/settings/set', {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ scope: 'global', key: `slot.${s}`, value: slots[s] ?? 'default' }),
          }),
        )
      }
      puts.push(
        fetch('/api/settings/set', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ scope: 'global', key: 'adv.contextBudget', value: adv.contextBudget }) }),
        fetch('/api/settings/set', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ scope: 'global', key: 'adv.maxTokens', value: adv.maxTokens }) }),
        fetch('/api/settings/set', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ scope: 'global', key: 'adv.autoPatrolEnabled', value: adv.autoPatrolEnabled }) }),
        fetch('/api/settings/set', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ scope: 'global', key: 'adv.autoPatrolHours', value: adv.autoPatrolHours }) }),
      )
      const rs = await Promise.all(puts)
      if (rs.some((r) => !r.ok)) throw new Error('部分配置写入失败')
      setSnapshot(JSON.stringify({ services, slots, adv }))
      toast('配置已保存')
    } catch (e) {
      toast(String(e), 'error')
    } finally {
      setSaving(false)
    }
  }

  const addService = () => {
    const id = `svc${Date.now() % 100000}`
    setServices((p) => ({ ...p, [id]: { id, name: '', baseUrl: '', model: '', apiKey: '' } }))
  }

  const removeService = async (id: string) => {
    // 审计 P2：槽位占用防护——被绑定的服务禁止删除（否则槽位指向死服务，运行时静默回退 env）
    const bound = Object.entries(slots).filter(([, v]) => v === id).map(([k]) => SLOTS.find(([s]) => s === k)?.[1] ?? k)
    if (bound.length > 0) {
      toast(`该服务被槽位绑定（${bound.join('、')}）——请先在下方槽位绑定中改用其他服务`, 'error')
      setConfirmDelete(null)
      return
    }
    setServices((p) => {
      const rest = { ...p }
      delete rest[id]
      return rest
    })
    await Promise.all([
      fetch(`/api/settings/global/${encodeURIComponent(`llm.service.${id}`)}`, { method: 'DELETE' }),
      fetch(`/api/settings/global/${encodeURIComponent(`llm.service.${id}.apiKey`)}`, { method: 'DELETE' }),
    ])
    toast('服务已删除')
  }

  // 审计 P2：测试连接——用当前表单值（未保存也能测）ping 服务端点，结果落 toast
  const [testingSvc, setTestingSvc] = useState<string | null>(null)
  const testService = async (id: string) => {
    const s = services[id]
    if (!s) return
    setTestingSvc(id)
    try {
      const r = await fetch('/api/llm/test', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ service_id: id, base_url: s.baseUrl, model: s.model, api_key: s.apiKey }),
      })
      const d: { data?: { ok: boolean; latencyMs: number; protocol: string; error?: string } } = r.ok ? await r.json() : null
      if (!d?.data) {
        toast('测试失败（后端响应异常）', 'error')
      } else if (d.data.ok) {
        toast(`连接正常 · ${d.data.latencyMs}ms（${d.data.protocol}）`, 'info')
      } else {
        toast(`连接失败：${d.data.error?.slice(0, 120) ?? d.data.protocol}`, 'error')
      }
    } catch {
      toast('测试失败（需要后端在线）', 'error')
    } finally {
      setTestingSvc(null)
    }
  }

  const updService = (id: string, patch: Partial<Service>) =>
    setServices((p) => ({ ...p, [id]: { ...p[id], ...patch } }))

    return (
    <div className={`${embedded ? 'h-full w-full' : 'anim-drawer-in fixed inset-y-0 right-0 z-30 w-[460px] elev-3'} flex border-l border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900`}>
      {/* 分区导航（macOS 系统设置式左栏） */}
      <nav className="flex w-40 shrink-0 flex-col gap-0.5 border-r border-slate-100 dark:border-slate-800 bg-slate-50/50 dark:bg-slate-900/50 dark:bg-slate-900/40 p-3">
        {SECTIONS.map((s) => (
          <button
            key={s.id}
            onClick={() => setSection(s.id)}
            className={`flex items-start gap-2 rounded-lg px-2.5 py-2 text-left transition-colors ${
              section === s.id ? 'bg-white dark:bg-slate-900 elev-1 text-blue-600' : 'text-slate-500 dark:text-slate-400 hover:bg-white/70 dark:hover:bg-slate-800/70'
            }`}
          >
            <s.icon size={14} className="mt-0.5 shrink-0" />
            <span>
              <span className={`block text-[12px] font-bold ${section === s.id ? 'text-slate-800 dark:text-slate-100' : ''}`}>{s.label}</span>
              <span className="text-micro block text-slate-400 dark:text-slate-500">{s.hint}</span>
            </span>
          </button>
        ))}
        <div className="mt-auto text-micro px-2.5 leading-4 text-slate-300 dark:text-slate-600">
          API Key 加密存储
          <br />
          仅本机可解密
        </div>
      </nav>

      {/* 内容区 */}
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex items-center justify-between border-b border-slate-100 dark:border-slate-800 px-5 py-3.5">
          <div className="flex items-center gap-2.5">
            <h2 className="text-[15px] font-bold text-slate-800 dark:text-slate-100">{SECTIONS.find((s) => s.id === section)?.label}</h2>
            {dirty && (
              <span className="anim-scale-in flex items-center gap-1 rounded-full bg-amber-50 dark:bg-amber-950/40 px-2 py-0.5 text-micro font-semibold text-amber-600">
                <i className="h-1 w-1 rounded-full bg-amber-500" /> 未保存
              </span>
            )}
          </div>
          <button onClick={onClose} className="rounded-md p-1 text-slate-400 dark:text-slate-500 transition-colors hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-600">
            <X size={15} />
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto p-5">
          {loading ? (
            <div className="flex items-center justify-center gap-2 pt-20 text-[12px] text-slate-400 dark:text-slate-500">
              <Loader2 size={14} className="animate-spin" /> 加载配置…
            </div>
          ) : section === 'agent' ? (
            <AgentSection />
          ) : section === 'services' ? (
            <ServicesSection
              services={services}
              slots={slots}
              errors={errors}
              testingSvc={testingSvc}
              confirmDelete={confirmDelete}
              showKey={showKey}
              onAdd={addService}
              onRemove={(id) => void removeService(id)}
              onTest={(id) => void testService(id)}
              onUpdate={updService}
              onSlotChange={(slot, v) => setSlots((p) => ({ ...p, [slot]: v }))}
              onConfirmDelete={setConfirmDelete}
              onShowKey={(id) => setShowKey((p) => ({ ...p, [id]: !p[id] }))}
            />
          ) : section === 'harness' ? (
            <HarnessSection about={about} onVersionChange={(v) => setAbout((p) => (p ? { ...p, harness: v } : p))} />
          ) : section === 'advanced' ? (
            <AdvancedSection adv={adv} setAdv={setAdv} errors={errors} />
          ) : (
            <AboutSection about={about} />
          )}

        </div>

        {/* 保存栏（脏状态驱动） */}
        <div className="flex items-center gap-2 border-t border-slate-100 dark:border-slate-800 px-5 py-3">
          {dirty && (
            <button
              onClick={load}
              disabled={saving}
              className="flex items-center gap-1 rounded-md border border-slate-200 dark:border-slate-700 px-2.5 py-1.5 text-cap font-semibold text-slate-500 dark:text-slate-400 transition-colors hover:bg-slate-50 dark:hover:bg-slate-800/70 disabled:opacity-40"
            >
              <RotateCcw size={11} /> 放弃更改
            </button>
          )}
          <span className="text-micro ml-auto text-slate-300 dark:text-slate-600">{backendRepo ? '' : '需要本地后端在线'}</span>
          <button
            onClick={save}
            disabled={saving || loading || !dirty || !backendRepo}
            className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-4 py-1.5 text-[12px] font-bold text-white transition-all hover:bg-blue-700 disabled:opacity-40"
          >
            {saving ? <Loader2 size={12} className="animate-spin" /> : dirty ? <Save size={12} /> : <Check size={12} />}
            {saving ? '保存中…' : dirty ? '保存配置' : '已是最新'}
          </button>
        </div>
      </div>
    </div>
  )
}
