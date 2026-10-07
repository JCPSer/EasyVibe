import { toast } from '@/runtime/toast'
import { useState } from 'react'
import { X, Loader2, Check, Send, ChevronDown, ChevronUp, Wrench, ArrowRight, Crosshair} from 'lucide-react'
import type { TaskDraft } from '@/shared/logic/taskContext'
import type { Module } from '@/types/map'
import { createTask } from '@/api/task'
import { useLang } from '@/runtime/i18n'

interface Props {
  backendRepo: string | null
  draft: TaskDraft
  map: { modules: Module[] }
  onClose: () => void
  /** S1-5：创建成功后的引导（默认前往任务页签） */
  onCreated?: (taskId: string) => void
  /** 试用反馈#3：影响模块与地图联动——点定位按钮画布飞过去看 */
  onLocateModule?: (id: string) => void
  /** M2 降级：agent 缺失（false）时禁止提交——建议转任务/对话转任务都经此表单统一兜底 */
  agentReady?: boolean
}

// 任务表单（任务表单原型.png 的实现）：三字段极简 + 上下文注入预览 + 审批两档
export function TaskFormPanel({ backendRepo, draft, map, onClose, onCreated, onLocateModule, agentReady }: Props) {
  const { t } = useLang()
  // 重审 P0：标题曾是隐藏兜底（全部任务都叫"修复任务"）——显式输入，draft 预填可改
  const [title, setTitle] = useState(draft.title || '')
  const [description, setDescription] = useState(draft.description)
  const [acceptance, setAcceptance] = useState(draft.acceptance)
  const [selected, setSelected] = useState<string[]>(draft.modules)
  // 2026-10-05 卡控修复：默认 manual——supervised 低危会自动过计划关直接开发，
// 用户预期是「先看需求矩阵与方案再放行」（盲测 P0 语义保留，按需手选 supervised/auto）
const [trust, setTrust] = useState<'manual' | 'auto' | 'supervised'>('manual')
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
    createTask(backendRepo, {
      title: title.trim() || draft.title || t('canvas.taskform.defaultTitle'),
      description,
      modules: selected,
      acceptance,
      source: draft.source,
      context: draft.context,
      trust,
      conversation_id: draft.conversation_id ?? null,
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.json()
      })
      .then((d: { data: { id: string } }) => {
        setCreated(d.data.id)
      })
      .catch(() => toast(t('canvas.taskform.createFailed'), 'error'))
      .finally(() => setSending(false))
  }

  return (
    <div className="anim-drawer-in fixed inset-y-0 right-0 z-30 flex w-[400px] flex-col border-l border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 shadow-xl">
      <div className="flex items-center justify-between border-b border-slate-100 dark:border-slate-800 px-4 py-3">
        <span className="flex items-center gap-1.5 text-[13px] font-bold text-slate-800 dark:text-slate-100">
          <Wrench size={14} className="text-blue-500" /> {t('canvas.taskform.title')}
          <span className="rounded-full bg-blue-50 dark:bg-blue-950/40 px-1.5 py-px text-micro font-semibold text-blue-500">
            {t(`canvas.taskform.source.${draft.source}`)}
          </span>
        </span>
        <button onClick={onClose} className="rounded p-1 text-slate-400 dark:text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-600">
          <X size={16} />
        </button>
      </div>

      <div className="flex-1 space-y-4 overflow-y-auto p-4">
        {/* 上下文注入预览（F4 事前注入的原料，透明展示） */}
        <div className="rounded-lg border border-blue-100 bg-blue-50/50 p-3">
          <button onClick={() => setShowContext((v) => !v)} className="flex w-full items-center justify-between text-[11px] font-semibold text-blue-600">
            <span>{t('canvas.taskform.contextTitle')}</span>
            {showContext ? <ChevronUp size={12} /> : <ChevronDown size={12} />}
          </button>
          {showContext && (
            <div className="mt-2 space-y-1 text-cap leading-4 text-slate-500 dark:text-slate-400">
              <p>
                {t('canvas.taskform.contextSummary', { count: selected.length, violations: violationCount })}
              </p>
              {violationCount > 0 && <p className="text-red-500">{t('canvas.taskform.contextViolations')}</p>}
              <p className="text-slate-400 dark:text-slate-500">{t('canvas.taskform.contextClarify')}</p>
            </div>
          )}
        </div>

        <div>
          <label className="mb-1 block text-[11px] font-semibold text-slate-500 dark:text-slate-400">{t('canvas.taskform.fieldTitle')}</label>
          <input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder={t('canvas.taskform.titlePlaceholder')}
            className="w-full rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-950/70 px-3 py-2 text-[12px] text-slate-700 dark:text-slate-200 outline-none focus:border-blue-300"
          />
        </div>

        <div>
          <label className="mb-1 block text-[11px] font-semibold text-slate-500 dark:text-slate-400">{t('canvas.taskform.fieldDescription')}</label>
          <textarea
            rows={5}
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            className="w-full resize-none rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-950/70 px-3 py-2 text-[12px] leading-5 text-slate-700 dark:text-slate-200 outline-none focus:border-blue-300"
          />
        </div>

        <div>
          <label className="mb-1 block text-[11px] font-semibold text-slate-500 dark:text-slate-400">{t('canvas.taskform.fieldModules')}</label>
          <div className="flex flex-wrap gap-1.5 rounded-lg border border-slate-200 dark:border-slate-700 p-2.5">
            {map.modules.map((m) => (
              <span key={m.id} className="flex items-center gap-0.5">
                <button
                  onClick={() => toggleModule(m.id)}
                  className={`rounded-full border px-2.5 py-1 text-cap font-medium transition-colors ${
                    selected.includes(m.id)
                      ? 'border-blue-300 dark:border-blue-800 bg-blue-50 dark:bg-blue-950/40 text-blue-700'
                      : 'border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70'
                  }`}
                >
                  {m.name}
                </button>
                {onLocateModule && (
                  <button
                    onClick={() => onLocateModule(m.id)}
                    className="rounded-full p-0.5 text-slate-300 dark:text-slate-600 hover:text-blue-500"
                    title={t('canvas.taskform.locateTip')}
                  >
                    <Crosshair size={9} />
                  </button>
                )}
              </span>
            ))}
          </div>
        </div>

        <div>
          <label className="mb-1 block text-[11px] font-semibold text-slate-500 dark:text-slate-400">{t('canvas.taskform.fieldAcceptance')}</label>
          <textarea
            rows={3}
            value={acceptance}
            onChange={(e) => setAcceptance(e.target.value)}
            className="w-full resize-none rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-950/70 px-3 py-2 text-[12px] leading-5 text-slate-700 dark:text-slate-200 outline-none focus:border-blue-300"
          />
        </div>

        <div>
          <label className="mb-1 block text-[11px] font-semibold text-slate-500 dark:text-slate-400">{t('canvas.taskform.fieldTrust')}</label>
          {/* 改进#7：监督档——计划时风险预评估，低危直通、高危才停审批关 */}
          <div className="flex rounded-lg border border-slate-200 dark:border-slate-700 p-0.5">
            {(['supervised', 'auto', 'manual'] as const).map((mode) => (
              <button
                key={mode}
                onClick={() => setTrust(mode)}
                className={`flex-1 rounded-md py-1.5 text-[11px] font-semibold transition-colors ${
                  trust === mode ? 'bg-blue-600 text-white' : 'text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70'
                }`}
                title={t(`canvas.taskform.trust.${mode}Tip`)}
              >
                {t(`canvas.taskform.trust.${mode}`)}
              </button>
            ))}
          </div>
          {/* ui-test P1：监督档的自动通过语义必须在提交前告知，不能事后才在评审轮回里看到 */}
          {trust === 'supervised' && (
            <p className="mt-1 text-[10px] leading-4 text-slate-400 dark:text-slate-500">
              {t('canvas.taskform.trust.supervisedHint')}
            </p>
          )}
          {trust === 'auto' && (
            <p className="mt-1 text-[10px] leading-4 text-slate-400 dark:text-slate-500">{t('canvas.taskform.trust.autoHint')}</p>
          )}
          {trust === 'manual' && (
            <p className="mt-1 text-[10px] leading-4 text-slate-400 dark:text-slate-500">{t('canvas.taskform.trust.manualHint')}</p>
          )}
        </div>
      </div>

      <div className="border-t border-slate-100 dark:border-slate-800 p-3">
        {created ? (
          /* S1-5：创建成功不自动消失——引导用户去任务页签跟踪审批（此前链路断在面板静默关闭） */
          <div className="space-y-2">
            <p className="flex items-center gap-1.5 rounded-lg bg-emerald-50 dark:bg-emerald-950/40 px-3 py-2 text-[12px] font-semibold text-emerald-700">
              <Check size={13} /> {t('canvas.taskform.createdBase')}{trust === 'manual' ? t('canvas.taskform.createdManual') : trust === 'supervised' ? t('canvas.taskform.createdSupervised') : t('canvas.taskform.createdAuto')}
            </p>
            <button
              onClick={() => {
                onClose()
                // ui-test P1：带上新任务 id 跳转——TaskPage 收到后切流水线并选中它
                if (created) onCreated?.(created)
              }}
              className="flex w-full items-center justify-center gap-1.5 rounded-lg bg-blue-600 py-2 text-[12px] font-bold text-white transition-colors hover:bg-blue-700"
            >
              <ArrowRight size={13} /> {t('canvas.taskform.goTask')}
            </button>
            <button onClick={onClose} className="w-full py-0.5 text-cap text-slate-400 dark:text-slate-500 hover:text-slate-600">
              {t('canvas.taskform.stay')}
            </button>
          </div>
        ) : (
          <>
            {agentReady === false && (
              /* M2 降级（R6）：agent 缺失时提交按钮禁用的原因必须可见 */
              <p className="mb-2 flex items-center gap-1 rounded-lg bg-amber-50 dark:bg-amber-950/40 px-3 py-1.5 text-cap font-semibold text-amber-700">
                {t('canvas.taskform.agentMissing')}
              </p>
            )}
            <button
              onClick={submit}
              disabled={sending || !backendRepo || !description.trim() || agentReady === false}
              className="flex w-full items-center justify-center gap-1.5 rounded-lg bg-blue-600 py-2 text-[12px] font-bold text-white transition-colors hover:bg-blue-700 disabled:opacity-50"
            >
              {sending ? <Loader2 size={13} className="animate-spin" /> : <Send size={13} />}
              {sending ? t('canvas.taskform.submitting') : t('canvas.taskform.submit')}
            </button>
          </>
        )}
        {!backendRepo && !created && <p className="mt-1.5 text-center text-cap text-slate-400 dark:text-slate-500">{t('canvas.taskform.backendRequired')}</p>}
      </div>
    </div>
  )
}
