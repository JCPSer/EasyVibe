import { useState } from 'react'
import { AlertTriangle, Info } from 'lucide-react'

// M4-1.5 陪审团：图例默认收起为角落小条（此前大卡片压在画布黄金区），点击展开
export function Legend({ violations }: { violations: number }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="rounded-xl border border-slate-200 dark:border-slate-700 bg-white/95 dark:bg-slate-900/95 px-3 py-2 text-[11px] text-slate-600 dark:text-slate-300 shadow-sm backdrop-blur">
      <button onClick={() => setOpen((v) => !v)} className="flex items-center gap-1.5 text-micro font-bold uppercase tracking-wider text-slate-400 dark:text-slate-500 hover:text-slate-600">
        <Info size={11} /> 图例{open ? ' ▴' : ' ▾'}
      </button>
      {open && (
        <div className="mt-1.5 space-y-1.5 anim-fade-in-fast">
          <div className="flex items-center gap-2">
            <span className="inline-block h-2.5 w-2.5 rounded-full" style={{ background: '#10b981' }} /> Healthy（≥75）
          </div>
          <div className="flex items-center gap-2">
            <span className="inline-block h-2.5 w-2.5 rounded-full" style={{ background: '#f59e0b' }} /> Warning（60–74）
          </div>
          <div className="flex items-center gap-2">
            <span className="inline-block h-2.5 w-2.5 rounded-full" style={{ background: '#ef4444' }} /> Error（&lt;60）
          </div>
          <div className="flex items-center gap-2">
            <span className="inline-block w-5 border-t-2 border-slate-400" /> Dependency
          </div>
          <div className="flex items-center gap-2">
            <span className="inline-block w-5 border-t-2 border-dashed border-red-500" />
            <span className="flex items-center gap-1">
              <AlertTriangle size={11} className="text-red-500" /> 逆向依赖 violation（{violations}）
            </span>
          </div>
          <div className="flex items-center gap-2">
            <span className="inline-block w-5 border-t-2 border-dashed border-orange-500" />
            <span className="flex items-center gap-1 text-orange-600">内部循环依赖（子图）</span>
          </div>
        </div>
      )}
    </div>
  )
}
