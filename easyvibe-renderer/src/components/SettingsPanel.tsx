import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  Bot, Check, Eye, EyeOff, Info, KeyRound, Loader2, Pencil, Plus, RotateCcw, Save, ShieldCheck, SlidersHorizontal, Sparkles, Terminal, Trash2, X, Zap,
} from 'lucide-react'
import { toast } from '@/lib/toast'
import { Select } from '@/components/ui/SelectMenu'

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
  { id: 'harness', label: 'Harness', icon: ShieldCheck, hint: '自定义补充：追加团队规则' },
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
    'w-full rounded-md border bg-slate-50 dark:bg-slate-950/70 px-2.5 py-1.5 text-[12px] text-slate-700 dark:text-slate-200 outline-none transition-colors focus:border-blue-300 focus:bg-white'

  const Field = ({
    label, hint, error, children,
  }: { label: string; hint?: string; error?: string; children: React.ReactNode }) => (
    <div>
      <div className="mb-1 text-cap font-semibold text-slate-500 dark:text-slate-400">{label}</div>
      {children}
      {error ? (
        <p className="text-micro mt-1 font-medium text-red-500">{error}</p>
      ) : hint ? (
        <p className="text-micro mt-1 text-slate-300 dark:text-slate-600">{hint}</p>
      ) : null}
    </div>
  )

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
            /* M1 执行 agent 区（方案 §5.1） */
            <div className="space-y-4">
              <p className="text-cap text-slate-400 dark:text-slate-500">
                归纳 / 巡检 / 任务由本地 CLI agent 执行——配置在 spawn 时现读，保存后对下一次执行生效
              </p>

              {/* 状态卡：探测结果 + 生效配置 + 测试连接 */}
              <div className="space-y-2 rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="text-cap font-semibold text-slate-500 dark:text-slate-400">探测结果</span>
                  {(agentStatus?.presets ?? []).map((p) => {
                    const det = agentStatus?.detected.find((x) => x.command === p.id)
                    return (
                      <span
                        key={p.id}
                        className={`rounded-full px-2 py-0.5 text-micro font-semibold ${
                          det ? 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-600' : 'bg-slate-50 dark:bg-slate-950/70 text-slate-400 dark:text-slate-500'
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
                    className="ml-auto flex items-center gap-0.5 rounded-md border border-slate-200 dark:border-slate-700 px-2 py-0.5 text-micro font-semibold text-slate-400 dark:text-slate-500 hover:border-blue-300 hover:text-blue-600 disabled:opacity-40"
                  >
                    <RotateCcw size={10} className={agentBusy === 'detect' ? 'animate-spin' : ''} /> 重新探测
                  </button>
                </div>
                <p className="text-micro leading-4 text-slate-400 dark:text-slate-500">
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
                  <span className="text-micro text-slate-300 dark:text-slate-600">测试作用于已保存的配置（先保存再测）</span>
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
                          ? 'border-blue-300 dark:border-blue-800 bg-blue-50 dark:bg-blue-950/40 text-blue-700'
                          : 'border-slate-200 dark:border-slate-700 text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70'
                      }`}
                    >
                      {p.label}
                      {p.stability === 'experimental' && (
                        <span className="rounded-full bg-amber-50 dark:bg-amber-950/40 px-1 text-[9px] font-bold text-amber-600" title="无流式，完成后输出全文；参数为初值，可用测试连接验证">
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
                  <button onClick={() => void loadAgent()} className="rounded-md border border-slate-200 dark:border-slate-700 px-2.5 py-1.5 text-cap text-slate-400 dark:text-slate-500 hover:text-slate-600">
                    放弃
                  </button>
                )}
                <span className="ml-auto text-micro text-slate-300 dark:text-slate-600">
                  协议：{agentStatus?.effective.type === 'claude' ? 'stream-json 流式' : 'plain 纯文本（完成后输出）'}
                </span>
              </div>
            </div>
          ) : section === 'services' ? (
            <div className="space-y-4">
              <div className="flex items-center justify-between">
                <p className="text-cap text-slate-400 dark:text-slate-500">配置可复用的 LLM 服务，再绑定到功能槽位</p>
                <button
                  onClick={addService}
                  className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 px-2.5 py-1 text-cap font-semibold text-slate-500 dark:text-slate-400 transition-colors hover:border-blue-300 hover:text-blue-600"
                >
                  <Plus size={11} /> 添加服务
                </button>
              </div>
              <div className="space-y-3">
                {Object.values(services).map((s) => (
                  <div key={s.id} className="lift elev-1 space-y-3 rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
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
                            className="rounded-md p-1.5 text-slate-300 dark:text-slate-600 transition-colors hover:bg-blue-50 dark:hover:bg-blue-950/40 hover:text-blue-500 disabled:opacity-40"
                            title="测试连接：用当前表单值 ping 服务端点（max_tokens=1）"
                          >
                            {testingSvc === s.id ? <Loader2 size={13} className="animate-spin" /> : <Zap size={13} />}
                          </button>
                          <button
                            onClick={() => setConfirmDelete(s.id)}
                            className="rounded-md p-1.5 text-slate-300 dark:text-slate-600 transition-colors hover:bg-red-50 dark:hover:bg-red-950/40 hover:text-red-500"
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
                        <KeyRound size={12} className="absolute left-2.5 top-2.5 text-slate-300 dark:text-slate-600" />
                        <input
                          className={`${field} mono pl-7 pr-8`}
                          type={showKey[s.id] ? 'text' : 'password'}
                          placeholder={s.apiKey ? '已配置（输入以更换）' : 'sk-…'}
                          value={s.apiKey}
                          onChange={(e) => updService(s.id, { apiKey: e.target.value })}
                        />
                        <button
                          onClick={() => setShowKey((p) => ({ ...p, [s.id]: !p[s.id] }))}
                          className="absolute right-2 top-2 rounded p-0.5 text-slate-300 dark:text-slate-600 hover:text-slate-500"
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
              <div className="rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
                <div className="mb-3 flex items-center gap-1.5">
                  <ShieldCheck size={13} className="text-slate-400 dark:text-slate-500" />
                  <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">槽位绑定</span>
                  <span className="text-micro ml-auto text-slate-300 dark:text-slate-600">哪个功能用哪个服务</span>
                </div>
                <div className="space-y-2.5">
                  {SLOTS.map(([slot, label, desc]) => (
                    <div key={slot} className="flex items-center gap-3">
                      <div className="w-24 shrink-0">
                        <p className="text-[12px] font-semibold text-slate-600 dark:text-slate-300">{label}</p>
                        <p className="text-micro text-slate-300 dark:text-slate-600">{desc}</p>
                      </div>
                      <Select
                        className="flex-1"
                        value={slots[slot] ?? 'default'}
                        onChange={(v) => setSlots((p) => ({ ...p, [slot]: v }))}
                        options={Object.values(services).map((s) => ({ value: s.id, label: s.name || s.id }))}
                      />
                    </div>
                  ))}
                </div>
              </div>
            </div>
          ) : section === 'harness' ? (
            <HarnessSection about={about} onVersionChange={(v) => setAbout((p) => (p ? { ...p, harness: v } : p))} />
          ) : section === 'advanced' ? (
            <div className="space-y-4">
              <div className="rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
                <div className="mb-3 text-[13px] font-bold text-slate-700 dark:text-slate-200">上下文与输出</div>
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

              <div className="rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
                <div className="mb-1 flex items-center justify-between">
                  <div>
                    <div className="text-[13px] font-bold text-slate-700 dark:text-slate-200">定时巡检</div>
                    <p className="text-cap mt-0.5 text-slate-400 dark:text-slate-500">按间隔自动体检仓库健康度（消耗 LLM token，无活动会话才触发）</p>
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
                <div key={row.label} className="flex items-center justify-between rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-4 py-3">
                  <span className="text-[12px] text-slate-500 dark:text-slate-400">{row.label}</span>
                  <span className="mono text-cap font-semibold text-slate-700 dark:text-slate-200">{row.value}</span>
                </div>
              ))}
              <p className="text-micro px-1 leading-4 text-slate-300 dark:text-slate-600">
                地图与产物保存在各仓库的 .easyvibe/ 目录；全部数据不出本机。
              </p>
            </div>
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

/// Harness 分区（两栏改造 2026-10-05 方案 v2 + 用户裁定）：出厂层不对用户展示，
/// 单栏只呈现「自定义补充」——两槽卡片（iOS 开关：停用≠删除）、示例模板/AI 生成
/// 都走"不落盘、不保存不生效"纪律；✨ AI 辅助生成 = 自然语言需求 → LLM 草稿 → 编辑器待审。
const SLOT_WHERE: Record<string, string> = {
  'global.md': '全部 9 个 agent 上下文——任务流水线（需求分析 / 方案设计 / 实施 / 审查 / 初审）、归纳、子图分析、巡检、自动归纳、入口对话',
  'rule_development.md': 'development 流程——阶段 1/2 需求与方案、实施任务、独立代码审查、阶段产物初审',
}
const SLOT_LABEL: Record<string, string> = { global: 'global.md', development: 'rule_development.md' }

interface CustomSlot {
  path: string
  slot: string
  exists: boolean
  size: number
  mtimeMs: number
  enabled: boolean
}

function IosToggle({ on, disabled, onChange }: { on: boolean; disabled?: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      disabled={disabled}
      onClick={() => onChange(!on)}
      className={`relative h-[25px] w-[42px] flex-shrink-0 rounded-full transition-colors duration-200 disabled:opacity-40 ${
        on ? 'bg-emerald-500' : 'bg-slate-300 dark:bg-slate-600'
      }`}
    >
      <span
        className={`absolute top-[2px] h-[21px] w-[21px] rounded-full bg-white shadow transition-all duration-200 ${
          on ? 'left-[19px]' : 'left-[2px]'
        }`}
      />
    </button>
  )
}

function HarnessSection({ about, onVersionChange }: { about: { backend: string; harness: string } | null; onVersionChange: (v: string) => void }) {
  const [slots, setSlots] = useState<CustomSlot[] | null>(null)
  const [busy, setBusy] = useState(false)
  // 编辑态：mode=create 时是"尚未落盘的草稿"（模板/AI 预填），保存才真正创建
  const [editing, setEditing] = useState<{ path: string; content: string; loaded: string; mode: 'edit' | 'create'; source: 'template' | 'ai' | null } | null>(null)
  const [saving, setSaving] = useState(false)
  const [confirmDel, setConfirmDel] = useState<string | null>(null)
  const [confirmClear, setConfirmClear] = useState(false)
  // AI 生成弹窗
  const [ai, setAi] = useState<{ slot: string; description: string; generating: boolean } | null>(null)

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

  const refresh = () => {
    setBusy(true)
    load()
    setTimeout(() => setBusy(false), 300)
  }

  const openCreate = (slotKey: string) => {
    const path = SLOT_LABEL[slotKey]
    fetch(`/api/harness/custom/template?slot=${encodeURIComponent(slotKey)}`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data?: { content?: string } }) => {
        const content = d?.data?.content ?? ''
        setEditing({ path, content, loaded: '', mode: 'create', source: 'template' })
      })
      .catch(() => toast('示例模板加载失败', 'error'))
  }

  const openEdit = (path: string) => {
    fetch(`/api/harness/custom/file?path=${encodeURIComponent(path)}`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data?: { content?: string } }) => {
        const content = d?.data?.content ?? ''
        setEditing({ path, content, loaded: content, mode: 'edit', source: null })
      })
      .catch(() => toast('文件加载失败', 'error'))
  }

  const save = () => {
    if (!editing) return
    setSaving(true)
    fetch('/api/harness/custom/file', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: editing.path, content: editing.content }),
    })
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then(() => {
        toast(`已保存并热装载——${editing.path} 立即注入 agent 上下文`, 'info')
        setEditing(null)
        refresh()
      })
      .catch(() => toast('保存失败（路径防线或磁盘错误）', 'error'))
      .finally(() => setSaving(false))
  }

  const remove = (path: string) => {
    fetch(`/api/harness/custom/file?path=${encodeURIComponent(path)}`, { method: 'DELETE' })
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then(() => {
        toast(`已删除 ${path}，agent 上下文恢复为出厂规则`, 'info')
        if (editing?.path === path) setEditing(null)
        refresh()
      })
      .catch(() => toast('删除失败', 'error'))
      .finally(() => setConfirmDel(null))
  }

  const toggle = (path: string, enabled: boolean) => {
    fetch('/api/harness/custom/toggle', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path, enabled }),
    })
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then(() => {
        toast(enabled ? `${path} 已启用，立即注入 agent 上下文` : `${path} 已停用（文件保留，仅停止注入）`, 'info')
        refresh()
      })
      .catch(() => toast('开关切换失败', 'error'))
  }

  const clearAll = () => {
    const paths = (slots ?? []).filter((s) => s.exists).map((s) => s.path)
    Promise.all(paths.map((p) => fetch(`/api/harness/custom/file?path=${encodeURIComponent(p)}`, { method: 'DELETE' })))
      .then(() => {
        toast('已清空全部自定义补充', 'info')
        setEditing(null)
        refresh()
      })
      .catch(() => toast('清空失败', 'error'))
      .finally(() => setConfirmClear(false))
  }

  const openAi = (slotKey: string) => setAi({ slot: slotKey, description: '', generating: false })

  const runAi = () => {
    if (!ai || !ai.description.trim()) return
    setAi({ ...ai, generating: true })
    fetch('/api/harness/custom/generate', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ slot: ai.slot, description: ai.description.trim() }),
    })
      .then((r) => (r.ok ? r.json() : r.json().then((e: { error?: string }) => Promise.reject(new Error(e?.error ?? String(r.status))))))
      .then((d: { data?: { content?: string } }) => {
        const content = d?.data?.content ?? ''
        const path = SLOT_LABEL[ai.slot]
        // 与示例模板同一纪律：填入编辑器待审，不落盘；不覆盖未保存的既有草稿
        setEditing((prev) =>
          prev && prev.path === path
            ? { ...prev, content, source: 'ai' }
            : { path, content, loaded: '', mode: prev?.path === path ? prev.mode : 'create', source: 'ai' }
        )
        setAi(null)
        toast('AI 草稿已填入编辑器——审阅后保存才生效', 'info')
      })
      .catch((e: Error) => toast(`AI 生成失败：${e.message}`, 'error'))
      .finally(() => setAi((a) => (a ? { ...a, generating: false } : null)))
  }

  const slotCards = (slots ?? []).map((s) => {
    const statePill = !s.exists ? (
      <span className="rounded-full bg-slate-100 dark:bg-slate-800 px-2 py-0.5 text-micro font-bold text-slate-400">未设置</span>
    ) : s.enabled ? (
      <span className="rounded-full bg-emerald-50 dark:bg-emerald-950/40 px-2 py-0.5 text-micro font-bold text-emerald-600">● 已生效</span>
    ) : (
      <span className="rounded-full bg-red-50 dark:bg-red-950/40 px-2 py-0.5 text-micro font-bold text-red-500">已停用</span>
    )
    return (
      <div
        key={s.path}
        className={`rounded-md border p-3 transition-colors ${
          s.exists && !s.enabled
            ? 'border-dashed border-slate-200 dark:border-slate-700 bg-slate-50/50 dark:bg-slate-900/40'
            : 'border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900'
        }`}
      >
        <div className="flex items-center gap-2">
          <span className={`mono text-[12px] font-bold ${s.exists && !s.enabled ? 'text-slate-400 dark:text-slate-500' : 'text-slate-700 dark:text-slate-200'}`}>
            {s.path}
          </span>
          {statePill}
          <span className="ml-auto flex items-center gap-2">
            {s.exists && (
              <span className="hidden text-micro text-slate-300 dark:text-slate-600 sm:inline">
                {(s.size / 1024).toFixed(1)} KB · {s.mtimeMs ? new Date(s.mtimeMs).toLocaleString() : '—'} {!s.enabled && '· 文件保留'}
              </span>
            )}
            {s.exists && <IosToggle on={s.enabled} onChange={(v) => toggle(s.path, v)} />}
          </span>
        </div>
        <p className="mt-1.5 text-cap leading-4 text-slate-400 dark:text-slate-500">
          <b className="font-semibold text-slate-500 dark:text-slate-400">生效位置：</b>
          {SLOT_WHERE[s.path]}
        </p>
        <div className="mt-2 flex items-center gap-1.5">
          {s.exists ? (
            <>
              <button
                onClick={() => openEdit(s.path)}
                className="flex items-center gap-1 rounded-md border border-slate-200 dark:border-slate-700 px-2 py-1 text-micro font-semibold text-slate-500 dark:text-slate-400 hover:border-blue-300 hover:text-blue-600"
              >
                <Pencil size={11} /> 编辑
              </button>
              {confirmDel === s.path ? (
                <button
                  onClick={() => remove(s.path)}
                  className="rounded-md bg-red-500 px-2 py-1 text-micro font-bold text-white hover:bg-red-600"
                >
                  确认删除
                </button>
              ) : (
                <button
                  onClick={() => setConfirmDel(s.path)}
                  onMouseLeave={() => setConfirmDel(null)}
                  className="flex items-center gap-1 rounded-md border border-slate-200 dark:border-slate-700 px-2 py-1 text-micro font-semibold text-slate-500 dark:text-slate-400 hover:border-red-300 hover:text-red-500"
                >
                  <Trash2 size={11} /> 删除
                </button>
              )}
            </>
          ) : (
            <button
              onClick={() => openCreate(s.slot)}
              className="flex items-center gap-1 rounded-md bg-blue-600 px-2.5 py-1 text-micro font-bold text-white hover:bg-blue-700"
            >
              <Plus size={11} /> 创建
            </button>
          )}
          <button
            onClick={() => openAi(s.slot)}
            className="flex items-center gap-1 rounded-md border border-violet-200 dark:border-violet-900/60 bg-violet-50 dark:bg-violet-950/30 px-2 py-1 text-micro font-bold text-violet-700 dark:text-violet-300 hover:bg-violet-100 dark:hover:bg-violet-900/40"
          >
            <Sparkles size={11} /> AI 辅助生成
          </button>
          {editing?.path === s.path && (
            <span className="text-micro font-semibold text-blue-500">编辑中…</span>
          )}
        </div>
      </div>
    )
  })

  return (
    <div className="space-y-3">
      {/* 头部：标题 + 副文案 + 出厂版本（出厂层不展示，只留版本事实） */}
      <div className="flex items-start justify-between">
        <div>
          <h3 className="text-[15px] font-bold text-slate-800 dark:text-slate-100">Harness 自定义补充</h3>
          <p className="text-cap mt-0.5 leading-4 text-slate-400 dark:text-slate-500">
            出厂工作流护栏随软件版本自动更新，不在此展示、不可修改；
            <br />
            以下内容追加到所有 agent 上下文，与出厂规则冲突时以补充为准
          </p>
        </div>
        <span className="shrink-0 rounded-full bg-blue-50 dark:bg-blue-950/40 px-2.5 py-1 text-micro font-bold text-blue-600">
          出厂版本 v{about?.harness ?? '…'}
        </span>
      </div>

      {/* 概念条：追加不是修改；停用不是删除 */}
      <div className="flex items-start gap-2 rounded-md border border-slate-200 dark:border-slate-700 bg-gradient-to-r from-slate-50 to-slate-100/60 dark:from-slate-900 dark:to-slate-900/60 px-3 py-2.5">
        <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-md bg-indigo-100 dark:bg-indigo-950/60 text-[11px]">🧩</span>
        <p className="text-cap leading-4 text-slate-500 dark:text-slate-400">
          <b className="font-semibold text-slate-600 dark:text-slate-300">追加，不是修改；停用，不是删除。</b>
          补充写在出厂规则之后，agent 同时看到两者；拨动开关可临时停用（文件保留，随时恢复）；✨ AI 辅助生成帮你起草，保存前不会生效。
        </p>
      </div>

      {/* 槽位卡片 */}
      <div className="space-y-2">
        {slots === null && <p className="text-cap py-2 text-slate-400 dark:text-slate-500">加载中…</p>}
        {slotCards}
        {/* bugfix 槽占位：依赖任务类型字段 */}
        <div className="rounded-md border border-slate-200 dark:border-slate-700 bg-slate-50/60 dark:bg-slate-900/40 p-3 opacity-70">
          <div className="flex items-center gap-2">
            <span className="mono text-[12px] font-bold text-slate-400 dark:text-slate-500">rule_bugfix.md</span>
            <span className="rounded-full bg-amber-50 dark:bg-amber-950/40 px-2 py-0.5 text-micro font-bold text-amber-600">即将推出</span>
          </div>
          <p className="mt-1.5 text-cap leading-4 text-slate-400 dark:text-slate-500">
            <b className="font-semibold text-slate-400 dark:text-slate-500">生效位置：</b>
            bugfix 类任务与审查 —— 依赖「任务类型」字段，落地后开放
          </p>
        </div>
      </div>

      {/* 编辑器：创建（模板/AI 预填）与编辑共用；不落盘纪律在顶部提示条明示 */}
      {editing && (
        <div className="overflow-hidden rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
          <div className="flex items-center gap-1.5 border-b border-slate-100 dark:border-slate-800 px-3 py-2">
            <span className="mono text-[11px] font-bold text-slate-600 dark:text-slate-300">{editing.path}</span>
            <span className="rounded-full border border-slate-200 dark:border-slate-700 px-2 py-0.5 text-micro text-slate-400">≤ 64 KB</span>
            <span className="rounded-full border border-slate-200 dark:border-slate-700 px-2 py-0.5 text-micro text-slate-400">Markdown</span>
            <span className="rounded-full border border-slate-200 dark:border-slate-700 px-2 py-0.5 text-micro text-slate-400">.claude 路径自动换姓</span>
            <button
              onClick={() => setEditing(null)}
              className="ml-auto flex items-center gap-0.5 rounded-md border border-slate-200 dark:border-slate-700 px-2 py-1 text-micro font-semibold text-slate-400 hover:text-slate-600"
            >
              <X size={11} /> {editing.mode === 'create' ? '取消创建' : '关闭'}
            </button>
          </div>
          {editing.source && (
            <div className="flex items-center gap-1.5 border-b border-violet-100 dark:border-violet-900/50 bg-violet-50 dark:bg-violet-950/30 px-3 py-1.5 text-micro text-violet-700 dark:text-violet-300">
              <Sparkles size={11} className="shrink-0" />
              {editing.source === 'template'
                ? '示例模板已填入——按需删减后保存；不保存则不会创建，对 agent 无任何影响'
                : 'AI 生成草稿——请审阅修改后保存；不保存则不会生效，也不会覆盖现有规则'}
            </div>
          )}
          <textarea
            value={editing.content}
            onChange={(e) => setEditing({ ...editing, content: e.target.value })}
            rows={14}
            spellCheck={false}
            className="mono select-text w-full resize-y bg-slate-900 p-3 text-[11px] leading-5 text-slate-200 outline-none transition-colors focus:ring-2 focus:ring-blue-500/30 focus:ring-inset"
          />
          <div className="flex items-center gap-2 border-t border-slate-100 dark:border-slate-800 bg-slate-50/60 dark:bg-slate-900/60 px-3 py-2">
            <p className="text-micro text-slate-400 dark:text-slate-500">
              {editing.mode === 'create' ? (
                <span className="font-bold text-amber-500">● 未保存 · 槽位尚未创建</span>
              ) : editing.content === editing.loaded ? (
                '未修改'
              ) : (
                <span className="font-bold text-amber-500">● 有未保存修改</span>
              )}
              <span className="ml-1">保存后 agent 上下文立即生效，无需重启</span>
            </p>
            <button
              onClick={save}
              disabled={saving || (editing.mode === 'edit' ? editing.content === editing.loaded : !editing.content.trim())}
              className="ml-auto flex items-center gap-1 rounded-md bg-blue-600 px-3 py-1.5 text-micro font-bold text-white hover:bg-blue-700 disabled:opacity-40"
            >
              {saving ? <Loader2 size={11} className="animate-spin" /> : <Save size={11} />} 保存并热装载
            </button>
          </div>
        </div>
      )}

      {/* 危险区：清空全部 */}
      <div className="rounded-md border border-red-200 dark:border-red-900/60 bg-red-50/60 dark:bg-red-950/20 p-3">
        <div className="flex items-center justify-between">
          <div>
            <div className="text-[12px] font-bold text-red-700 dark:text-red-400">清空全部自定义补充</div>
            <p className="text-cap mt-0.5 text-red-500/80 dark:text-red-400/60">所有槽位文件与开关状态一并清除，agent 上下文恢复为纯出厂规则</p>
          </div>
          {confirmClear ? (
            <button
              onClick={clearAll}
              disabled={busy}
              className="shrink-0 rounded-md bg-red-500 px-3 py-1.5 text-micro font-bold text-white hover:bg-red-600 disabled:opacity-40"
            >
              确认清空
            </button>
          ) : (
            <button
              onClick={() => setConfirmClear(true)}
              onMouseLeave={() => setConfirmClear(false)}
              className="shrink-0 rounded-md border border-red-300 dark:border-red-800 bg-white dark:bg-slate-900 px-3 py-1.5 text-micro font-semibold text-red-600 hover:bg-red-100 dark:hover:bg-red-900/40"
            >
              清空
            </button>
          )}
        </div>
      </div>

      {/* AI 生成弹窗 */}
      {ai && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-900/40 backdrop-blur-[2px]" onClick={() => setAi(null)}>
          <div
            className="w-[520px] max-w-[92vw] rounded-xl bg-white dark:bg-slate-900 shadow-2xl overflow-hidden"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center gap-2 px-4 pt-4">
              <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-gradient-to-br from-violet-500 to-indigo-500 text-[13px] text-white">
                <Sparkles size={14} />
              </span>
              <b className="text-[13px] text-slate-800 dark:text-slate-100">AI 辅助生成</b>
              <span className="rounded-full border border-violet-200 dark:border-violet-900/60 bg-violet-50 dark:bg-violet-950/30 px-2 py-0.5 text-micro text-violet-700 dark:text-violet-300">
                {SLOT_LABEL[ai.slot]}
              </span>
            </div>
            <p className="px-4 pt-1.5 text-cap leading-4 text-slate-400 dark:text-slate-500">
              描述你的团队规则需求，AI 起草补充规则填入编辑器。
              <b className="text-violet-600 dark:text-violet-300">生成内容需你审阅并保存后才生效。</b>
            </p>
            <textarea
              autoFocus
              value={ai.description}
              onChange={(e) => setAi({ ...ai, description: e.target.value })}
              rows={5}
              placeholder="例：我们团队做金融系统，所有数据库查询必须参数化，代码审查必须额外检查越权访问和敏感数据日志……"
              className="mx-4 mt-3 w-[calc(100%-2rem)] resize-none rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-950/60 p-3 text-[12px] leading-5 text-slate-700 dark:text-slate-200 outline-none focus:ring-2 focus:ring-violet-500/30"
            />
            <div className="flex flex-wrap gap-1.5 px-4 pt-2">
              {['所有 DB 查询必须参数化', '方案必须含性能与回滚章节', '公开 API 必须有文档注释'].map((c) => (
                <button
                  key={c}
                  onClick={() => setAi({ ...ai, description: `例：我们要求${c}` })}
                  className="rounded-full border border-violet-100 dark:border-violet-900/40 bg-violet-50/60 dark:bg-violet-950/20 px-2.5 py-1 text-micro text-violet-600 dark:text-violet-300 hover:bg-violet-100 dark:hover:bg-violet-900/30"
                >
                  例：{c}
                </button>
              ))}
            </div>
            <div className="flex items-center gap-2 px-4 py-3.5">
              <span className="text-micro text-slate-300 dark:text-slate-600">将由已配置的 LLM 生成 · 不会自动生效</span>
              <button
                onClick={() => setAi(null)}
                className="ml-auto rounded-md border border-slate-200 dark:border-slate-700 px-3 py-1.5 text-micro font-semibold text-slate-500 dark:text-slate-400 hover:text-slate-700"
              >
                取消
              </button>
              <button
                onClick={runAi}
                disabled={ai.generating || !ai.description.trim()}
                className="flex items-center gap-1 rounded-md bg-gradient-to-br from-violet-500 to-indigo-500 px-3.5 py-1.5 text-micro font-bold text-white hover:from-violet-600 hover:to-indigo-600 disabled:opacity-40"
              >
                {ai.generating ? <Loader2 size={11} className="animate-spin" /> : <Sparkles size={11} />} 生成草稿
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

/** 开关组件（替代裸 checkbox：企业级表单控件最低要求） */
function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {  return (
    <button
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className={`relative h-5 w-9 shrink-0 rounded-full transition-colors duration-150 ${checked ? 'bg-blue-600' : 'bg-slate-200'}`}
    >
      <i
        className={`absolute top-0.5 h-4 w-4 rounded-full bg-white dark:bg-slate-900 elev-1 transition-all duration-150 ${checked ? 'left-[18px]' : 'left-0.5'}`}
      />
    </button>
  )
}
