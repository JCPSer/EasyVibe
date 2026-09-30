import { useState } from 'react'
import { X, Loader2, Check, Send, ChevronDown, ChevronUp, Wrench, ArrowRight} from 'lucide-react'
import type { TaskDraft } from '@/lib/taskContext'
import type { Module } from '@/types/map'

interface Props {
  backendRepo: string | null
  draft: TaskDraft
  map: { modules: Module[] }
  onClose: () => void
  /** S1-5：创建成功后的引导（默认前往任务页签） */
  onCreated?: () => void
}

// 任务表单（任务表单原型.png 的实现）：三字段极简 + 上下文注入预览 + 审批两档
export function TaskFormPanel({ backendRepo, draft, map, onClose, onCreated }: Props) {
  const [description, setDescription] = useState(draft.description)
  const [acceptance, setAcceptance] = useState(draft.acceptance)
  const [selected, setSelected] = useState<string[]>(draft.modules)
  const [trust, setTrust] = useState<'manual' | 'auto'>('manual')
  const [showContext, setShowContext] = useState(true)
  const [sending, setSending] = useState(false)
  const [created, setCreated] = useState<string | null>(null)

  const inject = (draft.context as { inject?: { violations?: unknown[] } }).inject
  const violationCount = inject?.violations?.length ?? 0

  const toggleModule = (id: string) =>
    setSelected((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]))

  const submit = () => {
    if (!backendRepo || sending || !description.trim()) return
    setSending(true)
    fetch(`/api/repos/${backendRepo}/tasks`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        title: draft.title || '修复任务',
        description,
        modules: selected,
        acceptance,
        source: draft.source,
        context: draft.context,
        trust,
      }),
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.json()
      })
      .then((d: { data: { id: string } }) => {
        setCreated(d.data.id)
      })
      .catch(() => alert('任务创建失败（需要本地后端在线）'))
      .finally(() => setSending(false))
  }

  return (
    <div className="fixed inset-y-0 right-0 z-30 flex w-[400px] flex-col border-l border-slate-200 bg-white shadow-xl">
      <div className="flex items-center justify-between border-b border-slate-100 px-4 py-3">
        <span className="flex items-center gap-1.5 text-[13px] font-bold text-slate-800">
          <Wrench size={14} className="text-blue-500" /> 发起任务
          <span className="rounded-full bg-blue-50 px-1.5 py-px text-[9px] font-semibold text-blue-500">
            {{ module: '模块', concern: '问题', layer: '层', manual: '手动' }[draft.source]}
          </span>
        </span>
        <button onClick={onClose} className="rounded p-1 text-slate-400 hover:bg-slate-100 hover:text-slate-600">
          <X size={16} />
        </button>
      </div>

      <div className="flex-1 space-y-4 overflow-y-auto p-4">
        {/* 上下文注入预览（F4 事前注入的原料，透明展示） */}
        <div className="rounded-lg border border-blue-100 bg-blue-50/50 p-3">
          <button onClick={() => setShowContext((v) => !v)} className="flex w-full items-center justify-between text-[11px] font-semibold text-blue-600">
            <span>已组织上下文（将随任务注入 agent）</span>
            {showContext ? <ChevronUp size={12} /> : <ChevronDown size={12} />}
          </button>
          {showContext && (
            <div className="mt-2 space-y-1 text-[10.5px] leading-4 text-slate-500">
              <p>
                模块 {selected.length} 个 · 职责/健康度/边界 · 相关违规边 {violationCount} 条
              </p>
              {violationCount > 0 && <p className="text-red-500">包含 direction_violation 证据，agent 修复后需消除</p>}
              <p className="text-slate-400">grill-me 澄清将在执行前由 Supervisor 按需追问（M3-3）</p>
            </div>
          )}
        </div>

        <div>
          <label className="mb-1 block text-[11px] font-semibold text-slate-500">需求描述</label>
          <textarea
            rows={5}
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            className="w-full resize-none rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-[12px] leading-5 text-slate-700 outline-none focus:border-blue-300"
          />
        </div>

        <div>
          <label className="mb-1 block text-[11px] font-semibold text-slate-500">影响模块（从地图选择）</label>
          <div className="flex flex-wrap gap-1.5 rounded-lg border border-slate-200 p-2.5">
            {map.modules.map((m) => (
              <button
                key={m.id}
                onClick={() => toggleModule(m.id)}
                className={`rounded-full border px-2.5 py-1 text-[10.5px] font-medium transition-colors ${
                  selected.includes(m.id)
                    ? 'border-blue-300 bg-blue-50 text-blue-700'
                    : 'border-slate-200 bg-white text-slate-500 hover:bg-slate-50'
                }`}
              >
                {m.name}
              </button>
            ))}
          </div>
        </div>

        <div>
          <label className="mb-1 block text-[11px] font-semibold text-slate-500">验收标准</label>
          <textarea
            rows={3}
            value={acceptance}
            onChange={(e) => setAcceptance(e.target.value)}
            className="w-full resize-none rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-[12px] leading-5 text-slate-700 outline-none focus:border-blue-300"
          />
        </div>

        <div>
          <label className="mb-1 block text-[11px] font-semibold text-slate-500">审批模式（F5 两档）</label>
          <div className="flex rounded-lg border border-slate-200 p-0.5">
            {(['manual', 'auto'] as const).map((t) => (
              <button
                key={t}
                onClick={() => setTrust(t)}
                className={`flex-1 rounded-md py-1.5 text-[11.5px] font-semibold transition-colors ${
                  trust === t ? 'bg-blue-600 text-white' : 'text-slate-500 hover:bg-slate-50'
                }`}
              >
                {t === 'manual' ? '手动（三道关）' : '自动（全程留痕）'}
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="border-t border-slate-100 p-3">
        {created ? (
          /* S1-5：创建成功不自动消失——引导用户去任务页签跟踪审批（此前链路断在面板静默关闭） */
          <div className="space-y-2">
            <p className="flex items-center gap-1.5 rounded-lg bg-emerald-50 px-3 py-2 text-[11.5px] font-semibold text-emerald-700">
              <Check size={13} /> 任务已创建{trust === 'manual' ? '，等待计划审批' : '，自动模式直通执行'}
            </p>
            <button
              onClick={() => {
                onClose()
                onCreated?.()
              }}
              className="flex w-full items-center justify-center gap-1.5 rounded-lg bg-blue-600 py-2 text-[12px] font-bold text-white transition-colors hover:bg-blue-700"
            >
              <ArrowRight size={13} /> 前往任务页签跟踪
            </button>
            <button onClick={onClose} className="w-full py-0.5 text-[10.5px] text-slate-400 hover:text-slate-600">
              留在画布
            </button>
          </div>
        ) : (
          <button
            onClick={submit}
            disabled={sending || !backendRepo || !description.trim()}
            className="flex w-full items-center justify-center gap-1.5 rounded-lg bg-blue-600 py-2 text-[12px] font-bold text-white transition-colors hover:bg-blue-700 disabled:opacity-50"
          >
            {sending ? <Loader2 size={13} className="animate-spin" /> : <Send size={13} />}
            {sending ? '提交中…' : '提交任务'}
          </button>
        )}
        {!backendRepo && !created && <p className="mt-1.5 text-center text-[10.5px] text-slate-400">需要本地后端在线</p>}
      </div>
    </div>
  )
}
