// ① 需求分析·任务书待批子态：任务书 + 影响模块 + 验收 + 批准/打回。
// 拆自 TaskWorkflowPage.tsx（2026-10-05 防膨胀）。
import { CheckCircle2, ClipboardList, Loader2 } from 'lucide-react'
import type { CodeMap } from '@/types/map'
import type { Approval, TaskItem } from '../types'

export function AnalysisStage({
  sel, map, approvals, deciding, rejecting, rejectNote, setRejectNote, setRejecting, onDecide,
}: {
  sel: TaskItem
  map: CodeMap
  approvals: Approval[]
  deciding: string | null
  rejecting: boolean
  rejectNote: string
  setRejectNote: (v: string) => void
  setRejecting: (v: boolean) => void
  onDecide: (d: 'approved' | 'rejected', note?: string) => void
}) {
  return (
    <div className="m-4 rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
      <p className="flex items-center gap-1 text-micro font-bold uppercase tracking-wider text-slate-400 dark:text-slate-500">
        <ClipboardList size={10} /> 任务书 · 已就绪，批准后先做需求分析
      </p>
      {/* TaskPanel 碎片③：监督模式风险预评（flagged 留痕——审批人必见风险理由） */}
      {(() => {
        const flagged = approvals.find((a) => a.decision === 'flagged' && a.note)
        return flagged ? (
          <p className="mt-2 rounded-lg border border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-3 py-1.5 text-micro leading-4 text-amber-700">⚠ {flagged.note}</p>
        ) : null
      })()}
      <p className="mt-2 line-clamp-6 text-[12px] leading-5 text-slate-700 dark:text-slate-200">{sel.description}</p>
      {(sel.modules?.length ?? 0) > 0 && (
        <div className="mt-2 flex flex-wrap gap-1">
          {sel.modules!.map((mid) => (
            <span key={mid} className="rounded-full bg-slate-50 dark:bg-slate-950/70 px-2 py-px text-micro font-semibold text-slate-500 dark:text-slate-400 ring-1 ring-slate-200">
              {map.modules.find((m) => m.id === mid)?.name ?? mid}
            </span>
          ))}
        </div>
      )}
      {sel.acceptance && <p className="mt-2 text-micro leading-4 text-slate-400 dark:text-slate-500">验收：{sel.acceptance}</p>}
      <div className="mt-3 flex gap-2">
        <button
          onClick={() => onDecide('approved')}
          disabled={!!deciding}
          className="flex flex-1 items-center justify-center gap-1 rounded-lg bg-blue-600 px-3 py-2 text-[12px] font-bold text-white hover:bg-blue-700 disabled:opacity-40"
        >
          {deciding === 'approved' ? <Loader2 size={12} className="animate-spin" /> : <CheckCircle2 size={12} />} 批准执行
        </button>
        <button
          onClick={() => setRejecting(true)}
          disabled={!!deciding}
          className="flex-1 rounded-lg border border-red-200 dark:border-red-900/60 bg-white dark:bg-slate-900 px-3 py-2 text-[12px] font-bold text-red-600 hover:bg-red-50 dark:hover:bg-red-950/40 disabled:opacity-40"
        >
          打回…
        </button>
      </div>
      {rejecting && (
        <div className="mt-2 space-y-1.5">
          <textarea
            autoFocus
            value={rejectNote}
            onChange={(e) => setRejectNote(e.target.value)}
            rows={3}
            placeholder="打回意见（必填）——将作为新任务的上下文"
            className="w-full resize-none rounded-lg border border-red-200 dark:border-red-900/60 bg-red-50/40 dark:bg-red-950/30 px-2.5 py-1.5 text-[12px] outline-none focus:border-red-300"
          />
          <div className="flex gap-2">
            <button
              onClick={() => onDecide('rejected')}
              disabled={!!deciding || !rejectNote.trim()}
              className="flex-1 rounded-lg bg-red-600 px-3 py-1.5 text-[12px] font-bold text-white hover:bg-red-700 disabled:opacity-40"
            >
              确认打回
            </button>
            <button onClick={() => setRejecting(false)} className="rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-1.5 text-[12px] text-slate-500 dark:text-slate-400">
              取消
            </button>
          </div>
        </div>
      )}
    </div>
  )
}
