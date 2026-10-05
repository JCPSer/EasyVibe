// 模型服务分区：服务列表（名称/Base URL/模型/API Key）+ 槽位绑定。
// 拆自 SettingsPanel.tsx（2026-10-05 防膨胀）。
import { Eye, EyeOff, KeyRound, Loader2, Plus, ShieldCheck, Trash2, Zap } from 'lucide-react'
import { Select } from '@/components/ui/SelectMenu'
import { SLOTS, field, type Service } from './common'
import { Field } from './controls'

export function ServicesSection({
  services, slots, errors, testingSvc, confirmDelete, showKey,
  onAdd, onRemove, onTest, onUpdate, onSlotChange, onConfirmDelete, onShowKey,
}: {
  services: Record<string, Service>
  slots: Record<string, string>
  errors: Record<string, string>
  testingSvc: string | null
  confirmDelete: string | null
  showKey: Record<string, boolean>
  onAdd: () => void
  onRemove: (id: string) => void
  onTest: (id: string) => void
  onUpdate: (id: string, patch: Partial<Service>) => void
  onSlotChange: (slot: string, v: string) => void
  onConfirmDelete: (id: string | null) => void
  onShowKey: (id: string) => void
}) {
  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <p className="text-cap text-slate-400 dark:text-slate-500">配置可复用的 LLM 服务，再绑定到功能槽位</p>
        <button
          onClick={onAdd}
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
                  <input className={field} placeholder="如：默认服务" value={s.name} onChange={(e) => onUpdate(s.id, { name: e.target.value })} />
                </Field>
              </div>
              {confirmDelete === s.id ? (
                <button
                  onClick={() => onRemove(s.id)}
                  onMouseLeave={() => onConfirmDelete(null)}
                  className="mt-5 shrink-0 rounded-md bg-red-500 px-2 py-1 text-micro font-bold text-white"
                >
                  确认删除
                </button>
              ) : (
                <span className="mt-5 flex shrink-0 items-center gap-0.5">
                  {/* 审计 P2：测试连接（现填值即可测，无需先保存） */}
                  <button
                    onClick={() => void onTest(s.id)}
                    disabled={testingSvc === s.id}
                    className="rounded-md p-1.5 text-slate-300 dark:text-slate-600 transition-colors hover:bg-blue-50 dark:hover:bg-blue-950/40 hover:text-blue-500 disabled:opacity-40"
                    title="测试连接：用当前表单值 ping 服务端点（max_tokens=1）"
                  >
                    {testingSvc === s.id ? <Loader2 size={13} className="animate-spin" /> : <Zap size={13} />}
                  </button>
                  <button
                    onClick={() => onConfirmDelete(s.id)}
                    className="rounded-md p-1.5 text-slate-300 dark:text-slate-600 transition-colors hover:bg-red-50 dark:hover:bg-red-950/40 hover:text-red-500"
                    title={Object.values(slots).includes(s.id) ? '该服务被槽位绑定，不可删除' : '删除服务'}
                  >
                    <Trash2 size={13} />
                  </button>
                </span>
              )}
            </div>
            <Field label="Base URL" error={errors[`baseUrl:${s.id}`]} hint="Anthropic 兼容端点，如 http://127.0.0.1:8787">
              <input className={`${field} mono`} placeholder="https://…" value={s.baseUrl} onChange={(e) => onUpdate(s.id, { baseUrl: e.target.value })} />
            </Field>
            <Field label="模型" hint="如 deepseek-v4.1-flash、claude-sonnet-4-5">
              <input className={`${field} mono`} placeholder="模型名" value={s.model} onChange={(e) => onUpdate(s.id, { model: e.target.value })} />
            </Field>
            <Field label="API Key" error={errors[`apiKey:${s.id}`]}>
              <div className="relative">
                <KeyRound size={12} className="absolute left-2.5 top-2.5 text-slate-300 dark:text-slate-600" />
                <input
                  className={`${field} mono pl-7 pr-8`}
                  type={showKey[s.id] ? 'text' : 'password'}
                  placeholder={s.apiKey ? '已配置（输入以更换）' : 'sk-…'}
                  value={s.apiKey}
                  onChange={(e) => onUpdate(s.id, { apiKey: e.target.value })}
                />
                <button
                  onClick={() => onShowKey(s.id)}
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
                onChange={(v) => onSlotChange(slot, v)}
                options={Object.values(services).map((s) => ({ value: s.id, label: s.name || s.id }))}
              />
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}
