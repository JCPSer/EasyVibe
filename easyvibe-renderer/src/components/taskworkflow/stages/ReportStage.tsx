// ⑤ report 关：审查报告（变更摘要 + 合约警告折叠 + 通过归档/打回）。
// 拆自 TaskWorkflowPage.tsx（2026-10-05 防膨胀）。
import { useState } from 'react'
import { CheckCircle2, Loader2, Lock } from 'lucide-react'
import type { TaskItem } from '../types'

export function ReportStage({
  sel, impact, deciding, rejecting, rejectNote, setRejectNote, setRejecting, onDecide,
}: {
  sel: TaskItem
  impact: { adds: number; dels: number; files: number }[]
  deciding: string | null
  rejecting: boolean
  rejectNote: string
  setRejectNote: (v: string) => void
  setRejecting: (v: boolean) => void
  onDecide: (d: 'approved' | 'rejected', note?: string) => void
}) {
  // warnings 列表折叠（默认 3 条，防路径墙刷屏）
  const [showAllWarnings, setShowAllWarnings] = useState(false)
  return (
    <div className="m-4 overflow-y-auto rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
      <p className="flex items-center gap-1 text-micro font-bold uppercase tracking-wider text-slate-400 dark:text-slate-500">
        <Lock size={10} /> 审查报告 · 通过后锁定归档
      </p>
      <div className="mt-2 space-y-1.5 text-[12px] leading-5 text-slate-700 dark:text-slate-200">
        {impact.map((im, i) => (
          <p key={i} className="tnum text-micro text-slate-500 dark:text-slate-400">
            本次变更：<span className="text-emerald-600">+{im.adds}</span> <span className="text-red-500">−{im.dels}</span> · {im.files} 文件
          </p>
        ))}
        {(sel.result?.warnings?.length ?? 0) > 0 && (
          <div className="mt-1">
            <ul className="space-y-0.5">
              {(showAllWarnings ? sel.result!.warnings! : sel.result!.warnings!.slice(0, 3)).map((w, i) => (
                // 2026-10-04 实弹：合约警告内嵌文件路径列表（曾一次刷出 644 条路径墙）——
                // 单行钳制两行 + title 悬浮看全文，超 3 条折叠
                <li key={i} className="line-clamp-2 break-all text-micro text-amber-600" title={w}>⚠ {w}</li>
              ))}
            </ul>
            {(sel.result!.warnings!.length > 3) && (
              <button
                onClick={() => setShowAllWarnings((v) => !v)}
                className="mt-0.5 text-micro font-semibold text-amber-500 hover:text-amber-600"
              >
                {showAllWarnings ? '收起' : `展开全部 ${sel.result!.warnings!.length} 条`}
              </button>
            )}
          </div>
        )}
        <p className="text-micro text-slate-400 dark:text-slate-500">审查报告全文随归档产出；通过后任务锁定，STAR 记忆与操作日志留痕。</p>
      </div>
      <div className="mt-3 flex gap-2">
        <button
          onClick={() => onDecide('approved')}
          disabled={!!deciding}
          className="flex flex-1 items-center justify-center gap-1 rounded-lg bg-blue-600 px-3 py-2 text-[12px] font-bold text-white hover:bg-blue-700 disabled:opacity-40"
        >
          {deciding === 'approved' ? <Loader2 size={12} className="animate-spin" /> : <CheckCircle2 size={12} />} 通过并归档
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
            placeholder="打回意见（必填）——agent 带意见重跑实施，完成后自动复审"
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
