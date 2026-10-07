// ④ diff 关：双栏查看器 + 审查-修复闭环（发起子 agent 复审 / 带意见修改并复审）+ 裁决。
// 拆自 TaskWorkflowPage.tsx（2026-10-05 防膨胀）。
import { useEffect, useMemo, useRef, useState } from 'react'
import { CheckCircle2, FileCode2, Hammer, Loader2, ShieldAlert } from 'lucide-react'
import { toast } from '@/runtime/toast'
import { useLang } from '@/runtime/i18n'
import { absTime } from '@/shared/logic/diffStat'
import { remediateTask, reviewTask } from '@/components/taskworkflow/taskAdmin'
import { pickActiveDiff, splitDiffFiles } from '../diffParse'
import type { TaskItem } from '../types'

const DIFF_PAGE = 600

export function DiffStage({
  backendRepo, sel, selKey, diffFull, deciding, rejecting, rejectNote, setRejectNote, setRejecting, onDecide, onReload,
}: {
  backendRepo: string
  sel: TaskItem
  selKey: string | null
  diffFull: string | null
  deciding: string | null
  rejecting: boolean
  rejectNote: string
  setRejectNote: (v: string) => void
  setRejecting: (v: boolean) => void
  onDecide: (d: 'approved' | 'rejected', note?: string) => void
  onReload: () => void
}) {
  const { t } = useLang()
  const files = useMemo(() => splitDiffFiles(diffFull), [diffFull])
  const [fileSel, setFileSel] = useState<{ taskId: string | null; file: string | null }>({ taskId: null, file: null })
  if (fileSel.taskId !== selKey) setFileSel({ taskId: selKey, file: null })
  const activeFile = fileSel.taskId === selKey ? fileSel.file : null
  const activeDiff = useMemo(() => pickActiveDiff(diffFull, activeFile), [diffFull, activeFile])
  const [diffState, setDiffState] = useState<{ src: string | null; limit: number }>({ src: null, limit: DIFF_PAGE })
  if (diffState.src !== activeDiff) setDiffState({ src: activeDiff, limit: DIFF_PAGE })
  const diffLines = useMemo(() => (activeDiff ? activeDiff.split('\n') : []), [activeDiff])

  // 代码审查节点的审查-修复闭环（2026-10-05 用户裁定）：人工发起子 agent 复审
  const [reviewing, setReviewing] = useState(false)
  const reviewClickAt = useRef(0)
  const doReview = () => {
    if (!backendRepo || !sel || reviewing) return
    reviewClickAt.current = Date.now()
    setReviewing(true)
    reviewTask(backendRepo, sel.id)
      .then(() => toast(t('task.reviewStarted'), 'info'))
      .catch((e) => {
        setReviewing(false)
        toast(e instanceof Error ? e.message : t('task.reviewStartFail'), 'error')
      })
  }
  const doRemediate = () => {
    if (!backendRepo || !sel) return
    remediateTask(backendRepo, sel.id)
      .then(() => {
        toast(t('task.remediateToast'))
        onReload()
      })
      .catch((e) => toast(e instanceof Error ? e.message : t('task.opFail'), 'error'))
  }
  // 复审完成判定：result.review.at（服务端毫秒时间戳）≥ 点击时刻即本轮已出结论
  useEffect(() => {
    const at = sel?.result?.review?.at
    if (reviewing && typeof at === 'number' && at >= reviewClickAt.current) setReviewing(false)
  }, [sel?.result?.review?.at, reviewing])

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex min-h-0 flex-1">
      <div className="w-52 shrink-0 overflow-y-auto border-r border-slate-100 dark:border-slate-800 bg-slate-50/50 dark:bg-slate-900/50 p-2">
        <p className="flex items-center gap-1 px-1.5 pb-1.5 text-micro font-bold uppercase tracking-wider text-slate-400 dark:text-slate-500">
          <FileCode2 size={10} /> {t('task.filesTitle', { n: files.length })}
        </p>
        {files.map((f) => (
          <button
            key={f}
            onClick={() => setFileSel((fs) => ({ taskId: selKey, file: fs.file === f ? null : f }))}
            className={`block w-full truncate rounded px-1.5 py-1 text-left font-mono text-micro ${
              f === activeFile ? 'bg-blue-100 text-blue-700' : 'text-slate-500 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-700/70'
            }`}
            title={f}
          >
            {f}
          </button>
        ))}
        {files.length === 0 && <p className="px-1.5 py-2 text-micro text-slate-400 dark:text-slate-500">{t('task.noDiffData')}</p>}
      </div>
      <div className="min-w-0 flex-1 overflow-auto bg-white dark:bg-slate-900 p-3">
        {activeDiff ? (
          <pre className="select-text mono text-cap leading-4">
            {diffLines.slice(0, diffState.limit).map((line, i) => (
              <div
                key={i}
                className={
                  line.startsWith('+') && !line.startsWith('+++')
                    ? 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-700'
                    : line.startsWith('-') && !line.startsWith('---')
                      ? 'bg-red-50 dark:bg-red-950/40 text-red-600'
                      : 'text-slate-600 dark:text-slate-300'
                }
              >
                {line || ' '}
              </div>
            ))}
            {diffLines.length > diffState.limit && (
              <button
                onClick={() => setDiffState((s) => ({ ...s, limit: s.limit + DIFF_PAGE }))}
                className="mt-1 w-full rounded-md border border-dashed border-slate-200 dark:border-slate-700 py-1 text-cap font-semibold text-slate-400 dark:text-slate-500 hover:border-blue-300 hover:text-blue-600"
              >
                {t('task.loadMore', { n: diffLines.length - diffState.limit })}
              </button>
            )}
          </pre>
        ) : (
          <p className="py-8 text-center text-[11px] text-slate-400 dark:text-slate-500">{t('task.noArchive')}</p>
        )}
      </div>
      </div>
      {/* 2026-10-05 用户裁定：代码审查节点 = 审查-修复闭环——
          人工发起子 agent 复审；未通过 → 带意见修改并复审（修复后自动再审），直到通过 */}
      <div className="border-t border-slate-100 dark:border-slate-800 px-4 py-2.5">
        {sel.result?.review && (
          <div
            className={`mb-2 flex items-start gap-2 rounded-lg border px-3 py-2 ${
              sel.result.review.verdict === 'pass'
                ? 'border-emerald-200 dark:border-emerald-900/60 bg-emerald-50/60 dark:bg-emerald-950/40'
                : 'border-red-200 dark:border-red-900/60 bg-red-50/60 dark:bg-red-950/40'
            }`}
          >
            <ShieldAlert size={11} className={`mt-0.5 shrink-0 ${sel.result.review.verdict === 'pass' ? 'text-emerald-600' : 'text-red-600'}`} />
            <div className="min-w-0">
              <p className={`text-micro font-bold ${sel.result.review.verdict === 'pass' ? 'text-emerald-700' : 'text-red-700'}`}>
                {t('task.subReview')}{sel.result.review.verdict === 'pass' ? t('task.phasePass') : t('task.phaseFail')}
                {typeof sel.result.review.at === 'number' && (
                  <span className="ml-1 font-normal text-slate-400">{absTime(String(sel.result.review.at))}</span>
                )}
              </p>
              <p className="mt-0.5 text-micro leading-4 text-slate-600 dark:text-slate-300">{sel.result.review.summary}</p>
            </div>
          </div>
        )}
        <div className="flex items-center gap-2">
          <button
            onClick={doReview}
            disabled={reviewing || !!deciding}
            className="flex items-center gap-1 rounded-lg border border-blue-200 dark:border-blue-900/60 bg-white dark:bg-slate-900 px-3 py-1.5 text-[12px] font-bold text-blue-600 hover:bg-blue-50 dark:hover:bg-blue-950/40 disabled:opacity-40"
            title={t('task.reviewBtnTip')}
          >
            {reviewing ? <Loader2 size={12} className="animate-spin" /> : <ShieldAlert size={12} />} 发起子agent复审
          </button>
          {sel.result?.review?.verdict === 'fail' && (
            <button
              onClick={doRemediate}
              disabled={reviewing || !!deciding}
              className="flex items-center gap-1 rounded-lg bg-violet-600 px-3 py-1.5 text-[12px] font-bold text-white hover:bg-violet-700 disabled:opacity-40"
              title={t('task.remediateTip')}
            >
              <Hammer size={12} /> {t('task.remediateBtn')}
            </button>
          )}
          <span className="ml-auto text-[10px] text-slate-300 dark:text-slate-600">
            {reviewing ? t('task.reviewing') : t('task.loopHint')}
          </span>
        </div>
      </div>
      {/* 2026-10-03 实弹 bug：Diff 关此前没有裁决按钮——任务卡死在代码审查关无法推进。
          2026-10-05 打回语义修订：通过 = 进审查报告关（终审）；
          打回 = 带意见原地重跑实施（完成后子 agent 自动复审，回到本关再审，可循环）。 */}
      <div className="border-t border-slate-100 dark:border-slate-800 px-4 py-2.5">
        {rejecting ? (
          <div className="space-y-1.5">
            <textarea
              autoFocus
              value={rejectNote}
              onChange={(e) => setRejectNote(e.target.value)}
              rows={2}
              placeholder={t('task.diffRejectPh')}
              className="w-full resize-none rounded-lg border border-red-200 dark:border-red-900/60 bg-red-50/40 dark:bg-red-950/30 px-2.5 py-1.5 text-[12px] outline-none focus:border-red-300"
            />
            <div className="flex gap-2">
              <button
                onClick={() => onDecide('rejected')}
                disabled={!!deciding || !rejectNote.trim()}
                className="rounded-lg bg-red-600 px-3 py-1.5 text-[12px] font-bold text-white hover:bg-red-700 disabled:opacity-40"
              >
                {t('task.confirmReject')}
              </button>
              <button onClick={() => setRejecting(false)} className="rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-1.5 text-[12px] text-slate-500 dark:text-slate-400">
                取消
              </button>
            </div>
          </div>
        ) : (
          <div className="flex items-center gap-2">
            <button
              onClick={() => onDecide('approved')}
              disabled={!!deciding}
              className="flex items-center gap-1 rounded-lg bg-blue-600 px-3 py-1.5 text-[12px] font-bold text-white hover:bg-blue-700 disabled:opacity-40"
            >
              {deciding === 'approved' ? <Loader2 size={12} className="animate-spin" /> : <CheckCircle2 size={12} />} 通过 Diff 审批，进入审查报告
            </button>
            <button
              onClick={() => {
                // 2026-10-05 用户裁定：子 agent 审查未通过时理由预填审查意见（可改），不强迫手填
                const rv = sel.result?.review
                setRejectNote(rv && rv.verdict === 'fail' ? rv.summary : '')
                setRejecting(true)
              }}
              disabled={!!deciding}
              className="rounded-lg border border-red-200 dark:border-red-900/60 bg-white dark:bg-slate-900 px-3 py-1.5 text-[12px] font-bold text-red-600 hover:bg-red-50 dark:hover:bg-red-950/40 disabled:opacity-40"
            >
              打回…
            </button>
            <span className="ml-auto text-[10px] text-slate-300 dark:text-slate-600">
              {t('task.diffFooterNote')}
            </span>
          </div>
        )}
      </div>
    </div>
  )
}
