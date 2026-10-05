/** 画布底部过滤按钮基元（画布私有） */
export function FilterButton({ active, onClick, label, activeClass }: { active: boolean; onClick: () => void; label: string; activeClass: string }) {
  return (
    <button
      onClick={onClick}
      className={`rounded-full border px-3 py-1 text-[11px] font-semibold transition-colors ${
        active ? activeClass : 'border-transparent text-slate-500 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-700/70'
      }`}
    >
      {label}
    </button>
  )
}
