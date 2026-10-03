import { toast } from '@/lib/toast'
import { useState } from 'react'
import { X, Loader2, Check, Send, ChevronDown, ChevronUp, Wrench, ArrowRight, Crosshair} from 'lucide-react'
import type { TaskDraft } from '@/lib/taskContext'
import type { Module } from '@/types/map'

interface Props {
  backendRepo: string | null
  draft: TaskDraft
  map: { modules: Module[] }
  onClose: () => void
  /** S1-5：创建成功后的引导（默认前往任务页签） */
  onCreated?: (taskId: string) => void
  /** 试用反馈#3：影响模块与地图联动——点定位按钮画布飞过去看 */
  onLocateModule?: (id: string) => void
}

// 任务表单（任务表单原型.png 的实现）：三字段极简 + 上下文注入预览 + 审批两档
export function TaskFormPanel({ backendRepo, draft, map, onClose, onCreated, onLocateModule }: Props) {
  // 重审 P0：标题曾是隐藏兜底（全部任务都叫"修复任务"）——显式输入，draft 预填可改
  const [title, setTitle] = useState(draft.title || '')
  const [description, setDescription] = useState(draft.description)
  const [acceptance, setAcceptance] = useState(draft.acceptance)
  const [selected, setSelected] = useState<string[]>(draft.modules)
  const [trust, setTrust] = useState<'manual' | 'auto' | 'supervised'>('supervised')
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
        title: title.trim() || draft.title || '修复任务',
        description,
        modules: selected,
        acceptance,
        source: draft.source,
        context: draft.context,
        trust,
        conversation_id: draft.conversation_id ?? null,
      }),
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.json()
      })
      .then((d: { data: { id: string } }) => {
        setCreated(d.data.id)
      })
      .catch(() => toast('任务创建失败（需要本地后端在线）', 'error'))
      .finally(() => setSending(false))
  }

  return (
    <div className="fixed inset-y-0 right-0 z-30 flex w-[400px] flex-col border-l border-slate-200 bg-white shadow-xl">
      <div className="flex items-center justify-between border-b border-slate-100 px-4 py-3">
        <span className="flex items-center gap-1.5 text-[13px] font-bold text-slate-800">
          <Wrench size={14} className="text-blue-500" /> 发起任务
          <span className="rounded-full bg-blue-50 px-1.5 py-px text-micro font-semibold text-blue-500">
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
            <div className="mt-2 space-y-1 text-cap leading-4 text-slate-500">
              <p>
                模块 {selected.length} 个 · 职责/健康度/边界 · 相关违规边 {violationCount} 条
              </p>
              {violationCount > 0 && <p className="text-red-500">包含 direction_violation 证据，agent 修复后需消除</p>}
              <p className="text-slate-400">执行前 agent 会就模糊点向你提问澄清，请留意任务页的通知</p>
            </div>
          )}
        </div>

        <div>
          <label className="mb-1 block text-[11px] font-semibold text-slate-500">任务标题</label>
          <input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="修复任务"
            className="w-full rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-[12px] text-slate-700 outline-none focus:border-blue-300"
          />
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
              <span key={m.id} className="flex items-center gap-0.5">
                <button
                  onClick={() => toggleModule(m.id)}
                  className={`rounded-full border px-2.5 py-1 text-cap font-medium transition-colors ${
                    selected.includes(m.id)
                      ? 'border-blue-300 bg-blue-50 text-blue-700'
                      : 'border-slate-200 bg-white text-slate-500 hover:bg-slate-50'
                  }`}
                >
                  {m.name}
                </button>
                {onLocateModule && (
                  <button
                    onClick={() => onLocateModule(m.id)}
                    className="rounded-full p-0.5 text-slate-300 hover:text-blue-500"
                    title="在画布中定位该模块"
                  >
                    <Crosshair size={9} />
                  </button>
                )}
              </span>
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
          <label className="mb-1 block text-[11px] font-semibold text-slate-500">审批模式</label>
          {/* 改进#7：监督档——计划时风险预评估，低危直通、高危才停审批关 */}
          <div className="flex rounded-lg border border-slate-200 p-0.5">
            {(['supervised', 'auto', 'manual'] as const).map((t) => (
              <button
                key={t}
                onClick={() => setTrust(t)}
                className={`flex-1 rounded-md py-1.5 text-[11px] font-semibold transition-colors ${
                  trust === t ? 'bg-blue-600 text-white' : 'text-slate-500 hover:bg-slate-50'
                }`}
                title={t === 'supervised' ? '风险预评估：低危自动过计划关，执行后停 Diff/报告两道人工关（推荐）' : t === 'auto' ? '三道关全跳过，全程留痕' : '计划/Diff/报告三道关逐个人审'}
              >
                {t === 'supervised' ? '监督' : t === 'auto' ? '自动' : '手动'}
              </button>
            ))}
          </div>
          {/* ui-test P1：监督档的自动通过语义必须在提交前告知，不能事后才在评审轮回里看到 */}
          {trust === 'supervised' && (
            <p className="mt-1 text-[10px] leading-4 text-slate-400">
              提交后先做风险预评估：低危自动通过任务书并立即执行，完成后停在 Diff 关等你审；高危会停下来等你批准。
            </p>
          )}
          {trust === 'auto' && (
            <p className="mt-1 text-[10px] leading-4 text-slate-400">提交后立即执行，三道关全跳过，全程留痕可回溯。</p>
          )}
          {trust === 'manual' && (
            <p className="mt-1 text-[10px] leading-4 text-slate-400">提交后停在任务书审批，批准后按 需求矩阵 → 方案设计 → 实施 逐阶段评审。</p>
          )}
        </div>
      </div>

      <div className="border-t border-slate-100 p-3">
        {created ? (
          /* S1-5：创建成功不自动消失——引导用户去任务页签跟踪审批（此前链路断在面板静默关闭） */
          <div className="space-y-2">
            <p className="flex items-center gap-1.5 rounded-lg bg-emerald-50 px-3 py-2 text-[12px] font-semibold text-emerald-700">
              <Check size={13} /> 任务已创建{trust === 'manual' ? '，等待计划审批' : trust === 'supervised' ? '，监督模式：执行完成后将停在 Diff 审批关' : '，自动模式直通执行'}
            </p>
            <button
              onClick={() => {
                onClose()
                // ui-test P1：带上新任务 id 跳转——TaskPage 收到后切流水线并选中它
                if (created) onCreated?.(created)
              }}
              className="flex w-full items-center justify-center gap-1.5 rounded-lg bg-blue-600 py-2 text-[12px] font-bold text-white transition-colors hover:bg-blue-700"
            >
              <ArrowRight size={13} /> 前往任务页签跟踪
            </button>
            <button onClick={onClose} className="w-full py-0.5 text-cap text-slate-400 hover:text-slate-600">
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
        {!backendRepo && !created && <p className="mt-1.5 text-center text-cap text-slate-400">需要本地后端在线</p>}
      </div>
    </div>
  )
}
