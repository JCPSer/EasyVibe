// 设置面板拆分产物：共享开关 / 字段组件（本文件只导出组件，满足 react-refresh/only-export-components）。
// 拆自 SettingsPanel.tsx（2026-10-05 防膨胀）。常量与类型见 ./common。

export function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className={`relative h-[22px] w-[38px] flex-shrink-0 rounded-full transition-colors duration-200 ${checked ? 'bg-blue-500' : 'bg-slate-300 dark:bg-slate-600'}`}
    >
      <span className={`absolute top-[2px] h-[18px] w-[18px] rounded-full bg-white shadow transition-all duration-200 ${checked ? 'left-[18px]' : 'left-[2px]'}`} />
    </button>
  )
}

export function IosToggle({ on, onChange }: { on: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      onClick={() => onChange(!on)}
      className={`relative h-[25px] w-[42px] flex-shrink-0 rounded-full transition-colors duration-200 ${on ? 'bg-emerald-500' : 'bg-slate-300 dark:bg-slate-600'}`}
    >
      <span className={`absolute top-[2px] h-[21px] w-[21px] rounded-full bg-white shadow transition-all duration-200 ${on ? 'left-[19px]' : 'left-[2px]'}`} />
    </button>
  )
}

export const Field = ({
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
