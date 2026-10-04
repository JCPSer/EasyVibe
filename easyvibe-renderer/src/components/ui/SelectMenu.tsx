import { useEffect, useRef, useState } from 'react'
import { Check, ChevronDown } from 'lucide-react'

/** 自定义下拉（2026-10-04 替换原生 select——原生控件在暗色下是系统拟物风，与设计语言割裂）。
 *  触发器 = 与输入框同族的设计；弹层 = 浮动卡片 + 选中勾 + 键盘可达（Enter/Esc/方向键）。 */
export function Select({
  value,
  options,
  onChange,
  className = '',
  ariaLabel,
}: {
  value: string
  options: { value: string; label: string }[]
  onChange: (v: string) => void
  className?: string
  ariaLabel?: string
}) {
  const [open, setOpen] = useState(false)
  const [idx, setIdx] = useState(() => Math.max(0, options.findIndex((o) => o.value === value)))
  const ref = useRef<HTMLDivElement>(null)
  const current = options.find((o) => o.value === value)
  useEffect(() => {
    if (!open) return
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false)
    }
    window.addEventListener('mousedown', close)
    return () => window.removeEventListener('mousedown', close)
  }, [open])
  const pick = (i: number) => {
    setIdx(i)
    onChange(options[i].value)
    setOpen(false)
  }
  return (
    <div ref={ref} className={`relative ${className}`}>
      <button
        onClick={() => setOpen((v) => !v)}
        aria-label={ariaLabel}
        aria-expanded={open}
        className="flex w-full items-center gap-1.5 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1.5 text-cap text-slate-600 dark:text-slate-300 transition-colors hover:border-blue-300 dark:hover:border-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-100 dark:focus:ring-blue-900/50"
      >
        <span className="min-w-0 flex-1 truncate text-left">{current?.label ?? value}</span>
        <ChevronDown size={11} className={`shrink-0 text-slate-300 transition-transform duration-200 ${open ? 'rotate-180' : ''}`} />
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-30" onClick={() => setOpen(false)} />
          <div className="glass anim-scale-in absolute left-0 right-0 top-full z-40 mt-1 max-h-56 overflow-y-auto rounded-xl border border-slate-200 dark:border-slate-700 p-1 shadow-xl">
            {options.map((o, i) => (
              <button
                key={o.value}
                onMouseEnter={() => setIdx(i)}
                onClick={() => pick(i)}
                className={`flex w-full items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-left text-cap transition-colors ${
                  i === idx ? 'bg-blue-50 dark:bg-blue-950/50 font-semibold text-blue-700 dark:text-blue-300' : 'text-slate-600 dark:text-slate-300 hover:bg-slate-50 dark:hover:bg-slate-800/70'
                }`}
              >
                <span className="min-w-0 flex-1 truncate">{o.label}</span>
                {o.value === value && <Check size={11} className="shrink-0 text-blue-500" />}
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  )
}
