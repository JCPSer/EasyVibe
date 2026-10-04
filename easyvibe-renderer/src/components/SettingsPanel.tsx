import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  Bot, Check, Eye, EyeOff, Info, KeyRound, Loader2, Plus, RotateCcw, Save, ShieldCheck, SlidersHorizontal, Terminal, Trash2, X, Zap,
} from 'lucide-react'
import { toast } from '@/lib/toast'

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

const SLOTS: [string, string, string][] = [
  ['induction', '地图归纳', '首次归纳与重新归纳'],
  ['patrol', '巡检', '健康巡检与健康写回'],
  ['chat', '对话', '入口对话与智能建议'],
]

const SECTIONS = [
  { id: 'agent', label: '执行 agent', icon: Terminal, hint: 'CLI agent 命令与参数' },
  { id: 'services', label: '模型服务', icon: Bot, hint: 'LLM 服务与槽位绑定' },
  { id: 'advanced', label: '高级参数', icon: SlidersHorizontal, hint: '上下文预算与自动巡检' },
  { id: 'about', label: '关于', icon: Info, hint: '版本与运行环境' },
] as const

type SectionId = (typeof SECTIONS)[number]['id']

// 设置面板 v2（2026-10-02 UI 专项）：分区导航 + 脏状态追踪 + 字段校验 + 开关组件 + 关于页。
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

  // ── M1 执行 agent 分区（独立脏状态/独立保存——不混入模型服务快照）──
  interface AgentPreset { id: string; label: string; defaultArgs: string[]; type: string; stability: string }
  interface AgentStatus {
    detected: { command: string; path: string; version: string | null }[]
    configured: { command: string | null; args: unknown; argsTask: unknown; argsReview: unknown; preset: string | null; type: string | null }
    effective: { command: string; args: string[]; type: string; source: string; found: boolean }
    presets: AgentPreset[]
    lastTest: { ok: boolean; latencyMs: number; protocol: string; sample: string } | null
  }
  interface AgentEdit { command: string; argsGlobal: string; argsTask: string; argsReview: string; preset: string }
  const [agentStatus, setAgentStatus] = useState<AgentStatus | null>(null)
  const [agentEdit, setAgentEdit] = useState<AgentEdit | null>(null)
  const [agentSnap, setAgentSnap] = useState('')
  const [agentBusy, setAgentBusy] = useState<'save' | 'test' | 'detect' | null>(null)
  const agentDirty = agentEdit !== null && JSON.stringify(agentEdit) !== agentSnap

  const loadAgent = useCallback(async () => {
    const r = await fetch('/api/agent/status')
    const d = await r.json().catch(() => null)
    const st: AgentStatus | null = d?.data ?? null
    if (!st) return
    setAgentStatus(st)
    const arr = (v: unknown) => (Array.isArray(v) ? (v as string[]).join(' ') : '')
    const edit: AgentEdit = {
      command: st.configured.command ?? '',
      argsGlobal: arr(st.configured.args) || st.effective.args.join(' '),
      argsTask: arr(st.configured.argsTask),
      argsReview: arr(st.configured.argsReview),
      preset: st.configured.preset ?? 'claude',
    }
    setAgentEdit(edit)
    setAgentSnap(JSON.stringify(edit))
  }, [])

  useEffect(() => {
    if (section === 'agent') void loadAgent()
  }, [section, loadAgent])

  const splitArgs = (s: string) => s.trim().split(/\s+/).filter(Boolean)

  const saveAgent = async () => {
    if (!agentEdit || agentBusy) return
    setAgentBusy('save')
    try {
      const put = (key: string, value: unknown) =>
        fetch('/api/settings/set', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ scope: 'global', key, value }) })
      const puts: Promise<Response>[] = [
        put('agent.command', agentEdit.command.trim() || 'claude'),
        put('agent.args.global', splitArgs(agentEdit.argsGlobal)),
        put('agent.preset', agentEdit.preset),
      ]
      // 槽位：留空 = 删除键（跟随全局）——整体替换语义
      for (const [key, val] of [['agent.args.task', agentEdit.argsTask], ['agent.args.review', agentEdit.argsReview]] as const) {
        puts.push(val.trim() ? put(key, splitArgs(val)) : fetch(`/api/settings/global/${encodeURIComponent(key)}`, { method: 'DELETE' }))
      }
      const rs = await Promise.all(puts)
      if (rs.some((x) => !x.ok)) throw new Error('部分配置写入失败')
      toast('已保存——对进行中的任务不生效，下一次执行起用新配置')
      await loadAgent()
    } catch (e) {
      toast(String(e), 'error')
    } finally {
      setAgentBusy(null)
    }
  }

  const testAgent = async () => {
    if (agentBusy) return
    setAgentBusy('test')
    try {
      const r = await fetch('/api/agent/test', { method: 'POST' })
      const d = await r.json().catch(() => null)
      if (!r.ok) throw new Error(d?.error ?? '测试失败')
      toast(d.data.ok ? `协议兼容，${d.data.latencyMs}ms` : `不兼容：${d.data.protocol}`, d.data.ok ? 'info' : 'error')
      await loadAgent()
    } catch (e) {
      toast(String(e), 'error')
    } finally {
      setAgentBusy(null)
    }
  }

  const detectAgent = async () => {
    if (agentBusy) return
    setAgentBusy('detect')
    try {
      const r = await fetch('/api/agent/detect', { method: 'POST' })
      if (!r.ok) throw new Error('探测失败')
      await loadAgent()
      toast('探测完成')
    } catch (e) {
      toast(String(e), 'error')
    } finally {
      setAgentBusy(null)
    }
  }

  const applyPreset = (p: AgentPreset) =>
    setAgentEdit((prev) => prev && { ...prev, preset: p.id, command: p.id, argsGlobal: p.defaultArgs.join(' ') })

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

  const field =
    'w-full rounded-md border bg-slate-50 px-2.5 py-1.5 text-[12px] text-slate-700 outline-none transition-colors focus:border-blue-300 focus:bg-white'

  const Field = ({
    label, hint, error, children,
  }: { label: string; hint?: string; error?: string; children: React.ReactNode }) => (
    <div>
      <div className="mb-1 text-cap font-semibold text-slate-500">{label}</div>
      {children}
      {error ? (
        <p className="text-micro mt-1 font-medium text-red-500">{error}</p>
      ) : hint ? (
        <p className="text-micro mt-1 text-slate-300">{hint}</p>
      ) : null}
    </div>
  )

  return (
    <div className={`${embedded ? 'h-full w-full' : 'fixed inset-y-0 right-0 z-30 w-[460px] elev-3'} flex border-l border-slate-200 bg-white`}>
      {/* 分区导航（macOS 系统设置式左栏） */}
      <nav className="flex w-40 shrink-0 flex-col gap-0.5 border-r border-slate-100 bg-slate-50/50 p-3">
        {SECTIONS.map((s) => (
          <button
            key={s.id}
            onClick={() => setSection(s.id)}
            className={`flex items-start gap-2 rounded-lg px-2.5 py-2 text-left transition-colors ${
              section === s.id ? 'bg-white elev-1 text-blue-600' : 'text-slate-500 hover:bg-white/70'
            }`}
          >
            <s.icon size={14} className="mt-0.5 shrink-0" />
            <span>
              <span className={`block text-[12px] font-bold ${section === s.id ? 'text-slate-800' : ''}`}>{s.label}</span>
              <span className="text-micro block text-slate-400">{s.hint}</span>
            </span>
          </button>
        ))}
        <div className="mt-auto text-micro px-2.5 leading-4 text-slate-300">
          API Key 加密存储
          <br />
          仅本机可解密
        </div>
      </nav>

      {/* 内容区 */}
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex items-center justify-between border-b border-slate-100 px-5 py-3.5">
          <div className="flex items-center gap-2.5">
            <h2 className="text-[15px] font-bold text-slate-800">{SECTIONS.find((s) => s.id === section)?.label}</h2>
            {dirty && (
              <span className="anim-scale-in flex items-center gap-1 rounded-full bg-amber-50 px-2 py-0.5 text-micro font-semibold text-amber-600">
                <i className="h-1 w-1 rounded-full bg-amber-500" /> 未保存
              </span>
            )}
          </div>
          <button onClick={onClose} className="rounded-md p-1 text-slate-400 transition-colors hover:bg-slate-100 hover:text-slate-600">
            <X size={15} />
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto p-5">
          {loading ? (
            <div className="flex items-center justify-center gap-2 pt-20 text-[12px] text-slate-400">
              <Loader2 size={14} className="animate-spin" /> 加载配置…
            </div>
          ) : section === 'agent' ? (
            /* M1 执行 agent 区（方案 §5.1） */
            <div className="space-y-4">
              <p className="text-cap text-slate-400">
                归纳 / 巡检 / 任务由本地 CLI agent 执行——配置在 spawn 时现读，保存后对下一次执行生效
              </p>

              {/* 状态卡：探测结果 + 生效配置 + 测试连接 */}
              <div className="space-y-2 rounded-md border border-slate-200 bg-white p-4">
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="text-cap font-semibold text-slate-500">探测结果</span>
                  {(agentStatus?.presets ?? []).map((p) => {
                    const det = agentStatus?.detected.find((x) => x.command === p.id)
                    return (
                      <span
                        key={p.id}
                        className={`rounded-full px-2 py-0.5 text-micro font-semibold ${
                          det ? 'bg-emerald-50 text-emerald-600' : 'bg-slate-50 text-slate-400'
                        }`}
                        title={det?.path ?? '未在 PATH 与常见安装位找到'}
                      >
                        {det ? `● ${p.label} ${det.version ?? ''}`.trim() : `○ ${p.label} 未安装`}
                      </span>
                    )
                  })}
                  <button
                    onClick={() => void detectAgent()}
                    disabled={agentBusy !== null}
                    className="ml-auto flex items-center gap-0.5 rounded-md border border-slate-200 px-2 py-0.5 text-micro font-semibold text-slate-400 hover:border-blue-300 hover:text-blue-600 disabled:opacity-40"
                  >
                    <RotateCcw size={10} className={agentBusy === 'detect' ? 'animate-spin' : ''} /> 重新探测
                  </button>
                </div>
                <p className="text-micro leading-4 text-slate-400">
                  生效：<code className="mono">{agentStatus?.effective.command ?? '…'}</code>
                  {agentStatus && <span>（{({ settings: '来自设置', env: '来自环境变量', default: '默认' } as Record<string, string>)[agentStatus.effective.source] ?? agentStatus.effective.source}）</span>}
                  {agentStatus && !agentStatus.effective.found && <span className="font-semibold text-red-500"> · 未找到可执行文件</span>}
                </p>
                {agentStatus?.lastTest && (
                  <p className={`flex items-center gap-1 text-micro font-semibold ${agentStatus.lastTest.ok ? 'text-emerald-600' : 'text-red-500'}`}>
                    <ShieldCheck size={10} />
                    {agentStatus.lastTest.ok
                      ? `协议兼容 · ${agentStatus.lastTest.latencyMs}ms`
                      : `不兼容：${agentStatus.lastTest.protocol}`}
                  </p>
                )}
                <div className="flex items-center gap-2 pt-1">
                  <button
                    onClick={() => void testAgent()}
                    disabled={agentBusy !== null}
                    className="flex items-center gap-1 rounded-md bg-blue-600 px-2.5 py-1 text-micro font-bold text-white hover:bg-blue-700 disabled:opacity-40"
                    title="用当前已保存的配置发起一次最小真实调用（约 1-30 秒），验证协议兼容"
                  >
                    {agentBusy === 'test' ? <Loader2 size={10} className="animate-spin" /> : <Check size={10} />} 测试连接
                  </button>
                  <span className="text-micro text-slate-300">测试作用于已保存的配置（先保存再测）</span>
                </div>
              </div>

              {/* 预设 */}
              <Field label="预设" hint="选定后命令与参数自动填充，仍可微调；微调后预设记为自定义">
                <div className="flex flex-wrap gap-1.5">
                  {(agentStatus?.presets ?? []).map((p) => (
                    <button
                      key={p.id}
                      onClick={() => applyPreset(p)}
                      className={`flex items-center gap-1 rounded-full border px-2.5 py-1 text-cap font-semibold transition-colors ${
                        agentEdit?.preset === p.id
                          ? 'border-blue-300 bg-blue-50 text-blue-700'
                          : 'border-slate-200 text-slate-500 hover:bg-slate-50'
                      }`}
                    >
                      {p.label}
                      {p.stability === 'experimental' && (
                        <span className="rounded-full bg-amber-50 px-1 text-[9px] font-bold text-amber-600" title="无流式，完成后输出全文；参数为初值，可用测试连接验证">
                          实验
                        </span>
                      )}
                    </button>
                  ))}
                </div>
              </Field>

              <Field label="命令" hint="命令名或绝对路径；未找到时任务会失败并给出原因">
                <input
                  className={`${field} mono`}
                  value={agentEdit?.command ?? ''}
                  onChange={(e) => setAgentEdit((p) => p && { ...p, command: e.target.value, preset: 'custom' })}
                />
              </Field>
              <Field label="全局参数" hint="空白分隔">
                <input
                  className={`${field} mono`}
                  value={agentEdit?.argsGlobal ?? ''}
                  onChange={(e) => setAgentEdit((p) => p && { ...p, argsGlobal: e.target.value, preset: 'custom' })}
                />
              </Field>
              <Field label="任务槽参数（可选）" hint="留空 = 跟随全局；设置后任务执行整体使用本参数">
                <input
                  className={`${field} mono`}
                  placeholder="（留空 = 跟随全局）"
                  value={agentEdit?.argsTask ?? ''}
                  onChange={(e) => setAgentEdit((p) => p && { ...p, argsTask: e.target.value })}
                />
              </Field>
              <Field label="审查槽参数（可选）" hint="留空 = 跟随全局；可给审查换便宜模型">
                <input
                  className={`${field} mono`}
                  placeholder="（留空 = 跟随全局）"
                  value={agentEdit?.argsReview ?? ''}
                  onChange={(e) => setAgentEdit((p) => p && { ...p, argsReview: e.target.value })}
                />
              </Field>

              <div className="flex items-center gap-2 pt-1">
                <button
                  onClick={() => void saveAgent()}
                  disabled={!agentDirty || agentBusy !== null}
                  className="flex items-center gap-1 rounded-md bg-blue-600 px-3 py-1.5 text-cap font-bold text-white hover:bg-blue-700 disabled:opacity-40"
                >
                  {agentBusy === 'save' ? <Loader2 size={11} className="animate-spin" /> : <Save size={11} />} 保存 agent 配置
                </button>
                {agentDirty && (
                  <button onClick={() => void loadAgent()} className="rounded-md border border-slate-200 px-2.5 py-1.5 text-cap text-slate-400 hover:text-slate-600">
                    放弃
                  </button>
                )}
                <span className="ml-auto text-micro text-slate-300">
                  协议：{agentStatus?.effective.type === 'claude' ? 'stream-json 流式' : 'plain 纯文本（完成后输出）'}
                </span>
              </div>
            </div>
          ) : section === 'services' ? (
            <div className="space-y-4">
              <div className="flex items-center justify-between">
                <p className="text-cap text-slate-400">配置可复用的 LLM 服务，再绑定到功能槽位</p>
                <button
                  onClick={addService}
                  className="flex items-center gap-1 rounded-full border border-slate-200 px-2.5 py-1 text-cap font-semibold text-slate-500 transition-colors hover:border-blue-300 hover:text-blue-600"
                >
                  <Plus size={11} /> 添加服务
                </button>
              </div>
              <div className="space-y-3">
                {Object.values(services).map((s) => (
                  <div key={s.id} className="lift elev-1 space-y-3 rounded-md border border-slate-200 bg-white p-4">
                    <div className="flex items-start gap-2">
                      <div className="min-w-0 flex-1">
                        <Field label="服务名称" error={errors[`name:${s.id}`]}>
                          <input className={field} placeholder="如：默认服务" value={s.name} onChange={(e) => updService(s.id, { name: e.target.value })} />
                        </Field>
                      </div>
                      {confirmDelete === s.id ? (
                        <button
                          onClick={() => removeService(s.id)}
                          onMouseLeave={() => setConfirmDelete(null)}
                          className="mt-5 shrink-0 rounded-md bg-red-500 px-2 py-1 text-micro font-bold text-white"
                        >
                          确认删除
                        </button>
                      ) : (
                        <span className="mt-5 flex shrink-0 items-center gap-0.5">
                          {/* 审计 P2：测试连接（现填值即可测，无需先保存） */}
                          <button
                            onClick={() => void testService(s.id)}
                            disabled={testingSvc === s.id}
                            className="rounded-md p-1.5 text-slate-300 transition-colors hover:bg-blue-50 hover:text-blue-500 disabled:opacity-40"
                            title="测试连接：用当前表单值 ping 服务端点（max_tokens=1）"
                          >
                            {testingSvc === s.id ? <Loader2 size={13} className="animate-spin" /> : <Zap size={13} />}
                          </button>
                          <button
                            onClick={() => setConfirmDelete(s.id)}
                            className="rounded-md p-1.5 text-slate-300 transition-colors hover:bg-red-50 hover:text-red-500"
                            title={Object.values(slots).includes(s.id) ? '该服务被槽位绑定，不可删除' : '删除服务'}
                          >
                            <Trash2 size={13} />
                          </button>
                        </span>
                      )}
                    </div>
                    <Field label="Base URL" error={errors[`baseUrl:${s.id}`]} hint="Anthropic 兼容端点，如 http://127.0.0.1:8787">
                      <input className={`${field} mono`} placeholder="https://…" value={s.baseUrl} onChange={(e) => updService(s.id, { baseUrl: e.target.value })} />
                    </Field>
                    <Field label="模型" hint="如 deepseek-v4.1-flash、claude-sonnet-4-5">
                      <input className={`${field} mono`} placeholder="模型名" value={s.model} onChange={(e) => updService(s.id, { model: e.target.value })} />
                    </Field>
                    <Field label="API Key" error={errors[`apiKey:${s.id}`]}>
                      <div className="relative">
                        <KeyRound size={12} className="absolute left-2.5 top-2.5 text-slate-300" />
                        <input
                          className={`${field} mono pl-7 pr-8`}
                          type={showKey[s.id] ? 'text' : 'password'}
                          placeholder={s.apiKey ? '已配置（输入以更换）' : 'sk-…'}
                          value={s.apiKey}
                          onChange={(e) => updService(s.id, { apiKey: e.target.value })}
                        />
                        <button
                          onClick={() => setShowKey((p) => ({ ...p, [s.id]: !p[s.id] }))}
                          className="absolute right-2 top-2 rounded p-0.5 text-slate-300 hover:text-slate-500"
                          title={showKey[s.id] ? '隐藏' : '显示'}
                        >
                          {showKey[s.id] ? <EyeOff size={12} /> : <Eye size={12} />}
                        </button>
                      </div>
                    </Field>
                  </div>
                ))}
              </div>

              {/* 槽位绑定 */}
              <div className="rounded-md border border-slate-200 bg-white p-4">
                <div className="mb-3 flex items-center gap-1.5">
                  <ShieldCheck size={13} className="text-slate-400" />
                  <span className="text-[13px] font-bold text-slate-700">槽位绑定</span>
                  <span className="text-micro ml-auto text-slate-300">哪个功能用哪个服务</span>
                </div>
                <div className="space-y-2.5">
                  {SLOTS.map(([slot, label, desc]) => (
                    <div key={slot} className="flex items-center gap-3">
                      <div className="w-24 shrink-0">
                        <p className="text-[12px] font-semibold text-slate-600">{label}</p>
                        <p className="text-micro text-slate-300">{desc}</p>
                      </div>
                      <select
                        className={`${field} flex-1`}
                        value={slots[slot] ?? 'default'}
                        onChange={(e) => setSlots((p) => ({ ...p, [slot]: e.target.value }))}
                      >
                        {Object.values(services).map((s) => (
                          <option key={s.id} value={s.id}>{s.name || s.id}</option>
                        ))}
                      </select>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          ) : section === 'advanced' ? (
            <div className="space-y-4">
              <div className="rounded-md border border-slate-200 bg-white p-4">
                <div className="mb-3 text-[13px] font-bold text-slate-700">上下文与输出</div>
                <div className="space-y-3.5">
                  <Field label="上下文预算（token）" error={errors['contextBudget']} hint="会话占满预算的 80% 时自动压缩摘要（默认 256K）">
                    <input
                      className={`${field} tnum`}
                      type="number"
                      min={1000}
                      step={1000}
                      value={adv.contextBudget}
                      onChange={(e) => setAdv((p) => ({ ...p, contextBudget: Number(e.target.value) }))}
                    />
                  </Field>
                  <Field label="单次最大输出（token）" error={errors['maxTokens']} hint="LLM 一次回复的上限（默认 8192）">
                    <input
                      className={`${field} tnum`}
                      type="number"
                      min={256}
                      step={256}
                      value={adv.maxTokens}
                      onChange={(e) => setAdv((p) => ({ ...p, maxTokens: Number(e.target.value) }))}
                    />
                  </Field>
                </div>
              </div>

              <div className="rounded-md border border-slate-200 bg-white p-4">
                <div className="mb-1 flex items-center justify-between">
                  <div>
                    <div className="text-[13px] font-bold text-slate-700">定时巡检</div>
                    <p className="text-cap mt-0.5 text-slate-400">按间隔自动体检仓库健康度（消耗 LLM token，无活动会话才触发）</p>
                  </div>
                  <Toggle checked={adv.autoPatrolEnabled} onChange={(v) => setAdv((p) => ({ ...p, autoPatrolEnabled: v }))} />
                </div>
                {adv.autoPatrolEnabled && (
                  <div className="anim-scale-in mt-3">
                    <Field label="巡检间隔（小时）" error={errors['autoPatrolHours']}>
                      <input
                        className={`${field} tnum`}
                        type="number"
                        min={1}
                        value={adv.autoPatrolHours}
                        onChange={(e) => setAdv((p) => ({ ...p, autoPatrolHours: Number(e.target.value) }))}
                      />
                    </Field>
                  </div>
                )}
              </div>
            </div>
          ) : (
            <div className="space-y-3">
              {[
                { label: '应用后端版本', value: about?.backend ?? '加载中…' },
                { label: 'Harness 版本', value: about?.harness ?? '加载中…' },
                { label: '数据存储', value: '本机 SQLite（会话/任务/巡检历史）' },
              ].map((row) => (
                <div key={row.label} className="flex items-center justify-between rounded-md border border-slate-200 bg-white px-4 py-3">
                  <span className="text-[12px] text-slate-500">{row.label}</span>
                  <span className="mono text-cap font-semibold text-slate-700">{row.value}</span>
                </div>
              ))}
              <p className="text-micro px-1 leading-4 text-slate-300">
                地图与产物保存在各仓库的 .easyvibe/ 目录；全部数据不出本机。
              </p>
            </div>
          )}
        </div>

        {/* 保存栏（脏状态驱动） */}
        <div className="flex items-center gap-2 border-t border-slate-100 px-5 py-3">
          {dirty && (
            <button
              onClick={load}
              disabled={saving}
              className="flex items-center gap-1 rounded-md border border-slate-200 px-2.5 py-1.5 text-cap font-semibold text-slate-500 transition-colors hover:bg-slate-50 disabled:opacity-40"
            >
              <RotateCcw size={11} /> 放弃更改
            </button>
          )}
          <span className="text-micro ml-auto text-slate-300">{backendRepo ? '' : '需要本地后端在线'}</span>
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

/** 开关组件（替代裸 checkbox：企业级表单控件最低要求） */
function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className={`relative h-5 w-9 shrink-0 rounded-full transition-colors duration-150 ${checked ? 'bg-blue-600' : 'bg-slate-200'}`}
    >
      <i
        className={`absolute top-0.5 h-4 w-4 rounded-full bg-white elev-1 transition-all duration-150 ${checked ? 'left-[18px]' : 'left-0.5'}`}
      />
    </button>
  )
}
