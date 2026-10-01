import { useCallback, useEffect, useState } from 'react'
import { X, Plus, Trash2, Save, Loader2, KeyRound, Bot, SlidersHorizontal } from 'lucide-react'

interface Service {
  id: string
  name: string
  baseUrl: string
  model: string
  apiKey: string
}

interface Props {
  backendRepo: string | null
  onClose: () => void
  /** M4-1：作为设置"页"嵌入应用壳（非滑出抽屉） */
  embedded?: boolean
}

const SLOTS = [
  ['induction', '地图归纳'],
  ['patrol', '巡检'],
  ['chat', '对话'],
] as const

// 设置面板（backend-design §10 两档展示：常规 = 服务列表+槽位绑定；高级 = 预算类）
export function SettingsPanel({ backendRepo, onClose, embedded }: Props) {
  const [tab, setTab] = useState<'general' | 'advanced'>('general')
  const [services, setServices] = useState<Record<string, Service>>({})
  const [slots, setSlots] = useState<Record<string, string>>({})
  const [adv, setAdv] = useState({ contextBudget: 256000, maxTokens: 8192, autoPatrolEnabled: false, autoPatrolHours: 24 })
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)

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
    // 还原服务列表
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
    setServices(svc)
    setSlots(sl)
    setAdv({
      contextBudget: Number(merged['adv.contextBudget'] ?? 256000),
      maxTokens: Number(merged['adv.maxTokens'] ?? 8192),
      autoPatrolEnabled: Boolean(merged['adv.autoPatrolEnabled'] ?? false),
      autoPatrolHours: Number(merged['adv.autoPatrolHours'] ?? 24),
    })
    setLoading(false)
  }, [backendRepo])

  useEffect(() => {
    load()
  }, [load])

  const save = async () => {
    setSaving(true)
    setSaved(false)
    const scope = 'global'
    const puts: Promise<Response>[] = []
    for (const s of Object.values(services)) {
      puts.push(
        fetch('/api/settings/set', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ scope, key: `llm.service.${s.id}`, value: { name: s.name, baseUrl: s.baseUrl, model: s.model } }),
        }),
        fetch('/api/settings/set', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ scope, key: `llm.service.${s.id}.apiKey`, value: s.apiKey }),
        }),
      )
    }
    for (const [s] of SLOTS) {
      puts.push(
        fetch('/api/settings/set', {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ scope, key: `slot.${s}`, value: slots[s] ?? 'default' }),
        }),
      )
    }
    if (tab === 'advanced') {
      puts.push(
        fetch('/api/settings/set', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ scope, key: 'adv.contextBudget', value: adv.contextBudget }) }),
        fetch('/api/settings/set', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ scope, key: 'adv.maxTokens', value: adv.maxTokens }) }),
        fetch('/api/settings/set', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ scope, key: 'adv.autoPatrolEnabled', value: adv.autoPatrolEnabled }) }),
        fetch('/api/settings/set', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ scope, key: 'adv.autoPatrolHours', value: adv.autoPatrolHours }) }),
      )
    }
    await Promise.all(puts)
    setSaving(false)
    setSaved(true)
    setTimeout(() => setSaved(false), 2500)
  }

  const addService = () => {
    const id = `svc${Date.now() % 100000}`
    setServices((p) => ({ ...p, [id]: { id, name: '新服务', baseUrl: '', model: '', apiKey: '' } }))
  }

  const removeService = async (id: string) => {
    setServices((p) => {
      const { [id]: _d, ...rest } = p
      return rest
    })
    await Promise.all([
      fetch(`/api/settings/global/${encodeURIComponent(`llm.service.${id}`)}`, { method: 'DELETE' }),
      fetch(`/api/settings/global/${encodeURIComponent(`llm.service.${id}.apiKey`)}`, { method: 'DELETE' }),
    ])
  }

  const input = 'w-full rounded-md border border-slate-200 bg-slate-50 px-2 py-1.5 text-[11.5px] text-slate-700 outline-none focus:border-blue-300'

  return (
    <div className={`${embedded ? 'h-full w-full' : 'fixed inset-y-0 right-0 z-30 w-[400px] shadow-xl'} flex flex-col border-l border-slate-200 bg-white`}>
      <div className="flex items-center justify-between border-b border-slate-100 px-4 py-3">
        <span className="text-[13px] font-bold text-slate-800">设置</span>
        <div className="flex gap-1">
          {(['general', 'advanced'] as const).map((t) => (
            <button
              key={t}
              onClick={() => setTab(t)}
              className={`rounded-md px-2.5 py-1.5 text-[12px] font-semibold ${tab === t ? 'bg-blue-50 text-blue-600' : 'text-slate-400 hover:text-slate-600'}`}
            >
              {t === 'general' ? '常规' : '高级'}
            </button>
          ))}
          <button onClick={onClose} className="ml-1 rounded p-1 text-slate-400 hover:bg-slate-100 hover:text-slate-600">
            <X size={16} />
          </button>
        </div>
      </div>

      <div className="flex-1 space-y-4 overflow-y-auto p-4">
        {loading ? (
          <div className="flex items-center justify-center gap-2 pt-16 text-[12px] text-slate-400">
            <Loader2 size={14} className="animate-spin" /> 加载配置…
          </div>
        ) : tab === 'general' ? (
          <>
            <div>
              <div className="mb-2 flex items-center justify-between">
                <span className="flex items-center gap-1.5 text-[11px] font-bold uppercase tracking-wider text-slate-400">
                  <Bot size={12} /> LLM 服务列表
                </span>
                <button onClick={addService} className="flex items-center gap-1 rounded-full border border-slate-200 px-2 py-0.5 text-[10.5px] font-semibold text-slate-500 hover:bg-slate-50">
                  <Plus size={10} /> 添加
                </button>
              </div>
              <div className="space-y-3">
                {Object.values(services).map((s) => (
                  <div key={s.id} className="space-y-1.5 rounded-lg border border-slate-200 p-3">
                    <div className="flex items-center gap-1.5">
                      <input className={`${input} font-semibold`} value={s.name} onChange={(e) => setServices((p) => ({ ...p, [s.id]: { ...s, name: e.target.value } }))} />
                      <button onClick={() => removeService(s.id)} className="rounded p-1 text-slate-300 hover:text-red-500" title="删除服务">
                        <Trash2 size={13} />
                      </button>
                    </div>
                    <input className={input} placeholder="Base URL（如 http://127.0.0.1:8787）" value={s.baseUrl} onChange={(e) => setServices((p) => ({ ...p, [s.id]: { ...s, baseUrl: e.target.value } }))} />
                    <input className={input} placeholder="模型（如 deepseek-v4.1-flash）" value={s.model} onChange={(e) => setServices((p) => ({ ...p, [s.id]: { ...s, model: e.target.value } }))} />
                    <div className="relative">
                      <KeyRound size={11} className="absolute left-2 top-2 text-slate-300" />
                      <input className={`${input} pl-6`} type="password" placeholder="API Key（加密存储）" value={s.apiKey} onChange={(e) => setServices((p) => ({ ...p, [s.id]: { ...s, apiKey: e.target.value } }))} />
                    </div>
                  </div>
                ))}
              </div>
            </div>

            <div>
              <span className="mb-2 block text-[11px] font-bold uppercase tracking-wider text-slate-400">槽位绑定（哪个槽位用哪个服务）</span>
              <div className="space-y-1.5 rounded-lg border border-slate-200 p-3">
                {SLOTS.map(([slot, label]) => (
                  <div key={slot} className="flex items-center gap-2">
                    <span className="w-20 text-[11.5px] text-slate-600">{label}</span>
                    <select className={input} value={slots[slot] ?? 'default'} onChange={(e) => setSlots((p) => ({ ...p, [slot]: e.target.value }))}>
                      {Object.values(services).map((s) => (
                        <option key={s.id} value={s.id}>{s.name}</option>
                      ))}
                    </select>
                  </div>
                ))}
              </div>
            </div>
          </>
        ) : (
          <div>
            <span className="mb-2 flex items-center gap-1.5 text-[11px] font-bold uppercase tracking-wider text-slate-400">
              <SlidersHorizontal size={12} /> 高级（槽位默认采样参数锁死，见定稿 §10 #3）
            </span>
            <div className="space-y-3 rounded-lg border border-slate-200 p-3">
              <div>
                <div className="mb-1 text-[11px] text-slate-500">上下文预算（token，默认 256K）</div>
                <input className={input} type="number" value={adv.contextBudget} onChange={(e) => setAdv((p) => ({ ...p, contextBudget: Number(e.target.value) }))} />
              </div>
              <div>
                <div className="mb-1 text-[11px] text-slate-500">LLM 最大输出（token，默认 8192）</div>
                <input className={input} type="number" value={adv.maxTokens} onChange={(e) => setAdv((p) => ({ ...p, maxTokens: Number(e.target.value) }))} />
              </div>
              <div className="flex items-center justify-between">
                <div className="text-[11px] text-slate-500">定时巡检（默认关——间隔小时数可调，无活动会话才触发）</div>
                <input type="checkbox" checked={adv.autoPatrolEnabled} onChange={(e) => setAdv((p) => ({ ...p, autoPatrolEnabled: e.target.checked }))} />
              </div>
              {adv.autoPatrolEnabled && (
                <div>
                  <div className="mb-1 text-[11px] text-slate-500">巡检间隔（小时，默认 24）</div>
                  <input className={input} type="number" value={adv.autoPatrolHours} onChange={(e) => setAdv((p) => ({ ...p, autoPatrolHours: Number(e.target.value) }))} />
                </div>
              )}
            </div>
          </div>
        )}
      </div>

      <div className="border-t border-slate-100 p-3">
        <button
          onClick={save}
          disabled={saving || loading || !backendRepo}
          className="flex w-full items-center justify-center gap-1.5 rounded-lg bg-blue-600 py-2 text-[12px] font-bold text-white transition-colors hover:bg-blue-700 disabled:opacity-50"
        >
          {saving ? <Loader2 size={13} className="animate-spin" /> : saved ? <Save size={13} /> : <Save size={13} />}
          {saving ? '保存中…' : saved ? '已保存 ✓' : '保存配置'}
        </button>
        {!backendRepo && <p className="mt-1.5 text-center text-[10.5px] text-slate-400">需要本地后端在线</p>}
        <p className="mt-1.5 text-center text-[10px] text-slate-300">API Key 以 AES-256-GCM 加密入库（§10 #5）</p>
      </div>
    </div>
  )
}
