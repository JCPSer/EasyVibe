import { Moon, Sun } from 'lucide-react'
import { useLang } from '@/runtime/i18n'

/** 主题开关（2026-10-04）：苹果式滑动拨块——亮=太阳 / 暗=月亮，
 *  拨块带滑动过渡与图标交叉淡化；主题本体由 App 持有（html.dark class + localStorage）。 */
export function ThemeToggle({ dark, onChange }: { dark: boolean; onChange: (v: boolean) => void }) {
  const { t } = useLang()
  const label = t(dark ? 'shell.topbar.toLight' : 'shell.topbar.toDark')
  return (
    <button
      onClick={() => onChange(!dark)}
      role="switch"
      aria-checked={dark}
      aria-label={label}
      title={label}
      className={`relative h-6 w-[52px] shrink-0 rounded-full transition-colors duration-300 ${
        dark ? 'bg-slate-700' : 'bg-slate-200'
      }`}
    >
      {/* 两端图标：太阳在左（亮）、月亮在右（暗） */}
      <Sun
        size={11}
        className={`absolute left-1.5 top-1/2 -translate-y-1/2 transition-opacity duration-300 ${
          dark ? 'text-slate-500 opacity-60' : 'text-amber-500 opacity-0'
        }`}
      />
      <Moon
        size={11}
        className={`absolute right-1.5 top-1/2 -translate-y-1/2 transition-opacity duration-300 ${
          dark ? 'text-blue-300 opacity-0' : 'text-slate-400 opacity-60'
        }`}
      />
      {/* 滑动拨块：亮色时居左含太阳，暗色时居右含月亮 */}
      <span
        className={`absolute top-0.5 flex h-5 w-5 items-center justify-center rounded-full bg-white shadow-md transition-transform duration-300 ${
          dark ? 'translate-x-[28px]' : 'translate-x-0.5'
        }`}
      >
        <Sun size={11} className={`absolute text-amber-500 transition-opacity duration-200 ${dark ? 'opacity-0' : 'opacity-100'}`} />
        <Moon size={11} className={`absolute text-indigo-600 transition-opacity duration-200 ${dark ? 'opacity-100' : 'opacity-0'}`} />
      </span>
    </button>
  )
}
