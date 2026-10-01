import type { LucideIcon } from 'lucide-react'
import { Construction } from 'lucide-react'

// M4 占位页：诚实空态（验收清单⑪：一句说明 + 一个主行动按钮），不是"暂无数据"四个字。
interface Props {
  title: string
  milestone: string
  description: string
  action?: { label: string; onClick: () => void }
  icon?: LucideIcon
}

export function PlaceholderPage({ title, milestone, description, action, icon: Icon = Construction }: Props) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
      <span className="flex h-12 w-12 items-center justify-center rounded-2xl bg-blue-50 text-blue-500">
        <Icon size={22} />
      </span>
      <h2 className="text-[15px] font-bold text-slate-700">{title}</h2>
      <p className="max-w-md text-[12px] leading-6 text-slate-400">{description}</p>
      <span className="rounded-full bg-slate-100 px-2.5 py-0.5 text-[10px] font-semibold text-slate-400">{milestone} 交付</span>
      {action && (
        <button
          onClick={action.onClick}
          className="mt-1 rounded-lg bg-blue-600 px-4 py-2 text-[12px] font-semibold text-white hover:bg-blue-700"
        >
          {action.label}
        </button>
      )}
    </div>
  )
}
