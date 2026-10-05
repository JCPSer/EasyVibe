// 高级参数分区：上下文预算 / 单次最大输出 / 定时巡检。
// 拆自 SettingsPanel.tsx（2026-10-05 防膨胀）。
import { field } from './common'
import { Field, Toggle } from './controls'

type Adv = { contextBudget: number; maxTokens: number; autoPatrolEnabled: boolean; autoPatrolHours: number }

export function AdvancedSection({ adv, setAdv, errors }: {
  adv: Adv
  setAdv: (updater: (p: Adv) => Adv) => void
  errors: Record<string, string>
}) {
  return (
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
  )
}
