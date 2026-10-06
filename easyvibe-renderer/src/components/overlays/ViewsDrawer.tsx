import { X } from 'lucide-react'
import type { CodeMap } from '@/types/map'
import { ViewsPanel } from './ViewsPanel'

/** 顶栏抽屉：我的视图（右栏三页签瘦身后的新居所） */
export function ViewsDrawer({
  backendRepo,
  map,
  onClose,
  onOpenView,
}: {
  backendRepo: string | null
  map: CodeMap
  onClose: () => void
  onOpenView: (ids: string[]) => void
}) {
  return (
    <div className="anim-fade-in-fast fixed inset-0 z-50 flex justify-end bg-slate-900/20" onClick={onClose}>
      <div className="glass anim-drawer-in flex h-full w-[460px] flex-col shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between border-b border-slate-100 dark:border-slate-800 px-3 py-2">
          <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">我的视图</span>
          <button onClick={onClose} className="rounded p-1 text-slate-400 dark:text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-600">
            <X size={15} />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">
          <ViewsPanel
            backendRepo={backendRepo}
            onOpenView={onOpenView}
            validModuleIds={new Set(map.modules.map((m) => m.id))}
          />
        </div>
      </div>
    </div>
  )
}
