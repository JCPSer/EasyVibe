// 阶段产物评审卡：拉该阶段产物文档全文 + 通过/打回（需求矩阵/方案设计的评审载体）。
// compareDirHint：上一阶段产物目录（只读对照）；readonly：回看模式（隐藏裁决，露出重开入口）。
// 拆自 TaskWorkflowPage.tsx（2026-10-05 防膨胀）。
import { useEffect, useState } from 'react'
import { CheckCircle2, FileText, Loader2, ShieldAlert } from 'lucide-react'
import { useLang } from '@/runtime/i18n'
import { MarkdownMessage } from '@/shared/primitives/MarkdownMessage'
import { devDoc, devDocs } from '@/api/task'

/** 阶段产物评审卡：拉该阶段产物文档全文 + 通过/打回（需求矩阵/方案设计的评审载体）。
 *  compareDirHint：上一阶段产物目录（方案评审时回看需求矩阵）——只读对照，不带裁决按钮
 *  readonly（2026-10-05 管道回看）：隐藏裁决按钮，底部改「回到此关」重开入口（onRewind） */
export function PhaseDocReview({
  backendRepo,
  taskId,
  dirHint,
  title,
  deciding,
  review,
  compareDirHint,
  compareTitle,
  onDecide,
  readonly,
  onRewind,
  rewinding,
}: {
  backendRepo: string
  taskId: string
  /** 产物目录特征串：1_requirements_matrix / 2_requirements_solutions */
  dirHint: string
  title: string
  deciding: string | null
  /** 子 agent 阶段初审结论（2026-10-03：到人工关前的预筛，fail 不自动打回——人终审） */
  review?: { verdict: string; summary: string } | null
  /** 对照文档（上一阶段产物，只读回看） */
  compareDirHint?: string
  compareTitle?: string
  onDecide?: (d: 'approved' | 'rejected', note?: string) => void
  /** 回看模式：只读，不带裁决 */
  readonly?: boolean
  /** 回看模式底部重开入口（不可重开时不传，按钮不渲染） */
  onRewind?: () => void
  rewinding?: boolean
}) {
  const { t } = useLang()
  const [doc, setDoc] = useState<{ path: string; content: string } | null>(null)
  const [missing, setMissing] = useState(false)
  // 2026-10-03 实弹 bug：全文加载失败此前静默渲染空白——显式错误态 + 重试
  const [loadErr, setLoadErr] = useState(false)
  const [retryTick, setRetryTick] = useState(0)
  const [rejecting, setRejecting] = useState(false)
  const [note, setNote] = useState('')
  // 对照回看（2026-10-03 用户反馈：到方案阶段后无法回看需求矩阵）
  const [compareDoc, setCompareDoc] = useState<{ path: string; content: string } | null>(null)
  const [compareMissing, setCompareMissing] = useState(false)
  const [tab, setTab] = useState<'main' | 'compare'>('main')

  useEffect(() => {
    let dead = false
    devDocs(backendRepo, taskId)
      .then((r) => (r.ok ? r.json() : null))
      .then(async (d: { data?: { docs?: { path: string; mtime: number }[] } } | null) => {
        const hit = (d?.data?.docs ?? [])
          .filter((x) => x.path.includes(dirHint))
          .sort((a, b) => b.mtime - a.mtime)[0]
        if (!hit) {
          if (!dead) {
            setMissing(true)
            setLoadErr(false)
          }
          return
        }
        const resp = await devDoc(backendRepo, hit.path)
        const full = resp.ok ? await resp.json().catch(() => null) : null
        if (dead) return
        if (!full?.data) {
          setLoadErr(true)
          return
        }
        setLoadErr(false)
        setDoc({ path: hit.path, content: full.data.content ?? '' })
      })
      .catch(() => {
        if (!dead) setLoadErr(true)
      })
    return () => {
      dead = true
    }
  }, [backendRepo, taskId, dirHint, retryTick])

  // 对照文档加载（独立 effect：不阻塞主文档，缺失静默；任务切换由父级 key 重挂载兜底）
  useEffect(() => {
    if (!compareDirHint) return
    let dead = false
    devDocs(backendRepo, taskId)
      .then((r) => (r.ok ? r.json() : null))
      .then(async (d: { data?: { docs?: { path: string; mtime: number }[] } } | null) => {
        if (!dead) {
          setCompareMissing(false)
          setCompareDoc(null)
        }
        const hit = (d?.data?.docs ?? [])
          .filter((x) => x.path.includes(compareDirHint))
          .sort((a, b) => b.mtime - a.mtime)[0]
        if (!hit) {
          if (!dead) setCompareMissing(true)
          return
        }
        const full = await devDoc(backendRepo, hit.path).then((r) => (r.ok ? r.json().catch(() => null) : null))
        if (!dead && full?.data) setCompareDoc({ path: hit.path, content: full.data.content ?? '' })
      })
      .catch(() => {})
    return () => {
      dead = true
    }
  }, [backendRepo, taskId, compareDirHint, retryTick])

  return (
    <div className="m-4 flex min-h-0 flex-1 flex-col rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
      <div className="flex items-center gap-2 border-b border-slate-100 dark:border-slate-800 px-4 py-3">
        <FileText size={13} className="text-blue-600" />
        <div className="min-w-0 flex-1">
          <p className="text-[13px] font-bold text-slate-800 dark:text-slate-100">{title}</p>
          {doc && <p className="mono mt-0.5 truncate text-[10px] text-slate-400 dark:text-slate-500">{doc.path}</p>}
        </div>
        <span className={`shrink-0 rounded-full px-2 py-0.5 text-micro font-bold ${readonly ? 'bg-violet-100 text-violet-700' : 'bg-amber-100 text-amber-700'}`}>{readonly ? t('task.phaseReadonly') : t('task.phaseAwaiting')}</span>
      </div>
      {/* 对照回看标签栏：方案评审时可回看需求矩阵（只读）；缺失时标签不出现 */}
      {compareDirHint && !compareMissing && (
        <div className="flex items-center gap-1 border-b border-slate-100 dark:border-slate-800 px-4 py-1.5">
          {(
            [
              ['main', t('task.phaseReviewing', { title })],
              ['compare', compareTitle ?? t('task.phaseCompareDefault')],
            ] as ['main' | 'compare', string][]
          ).map(([k, label]) => (
            <button
              key={k}
              onClick={() => setTab(k)}
              className={`rounded-md px-2 py-0.5 text-[11px] font-semibold ${
                tab === k ? 'bg-blue-50 dark:bg-blue-950/40 text-blue-700 ring-1 ring-blue-200' : 'text-slate-400 dark:text-slate-500 hover:bg-slate-50 dark:hover:bg-slate-800/70 hover:text-slate-600'
              }`}
            >
              {label}
            </button>
          ))}
        </div>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        {tab === 'compare' ? (
          compareDoc ? (
            /* 对照文档只读——裁决按钮只对当前阶段产物 */
            <MarkdownMessage content={compareDoc.content} />
          ) : (
            <p className="flex items-center gap-2 py-8 text-center text-[12px] text-slate-400 dark:text-slate-500">
              <Loader2 size={13} className="animate-spin" /> {t('task.phaseLoadingCompare')}
            </p>
          )
        ) : (
          <>
            {/* 子 agent 阶段初审横幅：与 diff 关"子agent初审"同款语义——
                fail 只是预筛警报，最终裁决仍是下方的人工通过/打回 */}
        {review && (
          <div
            className={`mb-3 flex items-start gap-2 rounded-lg border px-3 py-2 ${
              review.verdict === 'pass' ? 'border-emerald-200 dark:border-emerald-900/60 bg-emerald-50/60' : 'border-amber-200 dark:border-amber-900/60 bg-amber-50/60'
            }`}
          >
            <ShieldAlert size={11} className={review.verdict === 'pass' ? 'mt-0.5 text-emerald-600' : 'mt-0.5 text-amber-600'} />
            <div className="min-w-0 flex-1">
              <p className={`text-micro font-bold ${review.verdict === 'pass' ? 'text-emerald-700' : 'text-amber-700'}`}>
                {t('task.phasePreReview')}{review.verdict === 'pass' ? t('task.phasePass') : t('task.phaseFail')}
                {review.verdict !== 'pass' && <span className="ml-1 font-normal">{t('task.phaseFailHint')}</span>}
              </p>
              <p className="mt-0.5 text-micro leading-4 text-slate-600 dark:text-slate-300">{review.summary}</p>
            </div>
            {/* 2026-10-05 用户裁定：初审已给出意见——打回不许再让用户手填理由。
                一键按初审意见打回（意见随打回注入，agent 带着重跑本阶段）。 */}
            {!readonly && review.verdict !== 'pass' && onDecide && (
              <button
                onClick={() => onDecide('rejected', review.summary)}
                disabled={!!deciding}
                className="shrink-0 rounded-lg bg-red-600 px-2.5 py-1 text-micro font-bold text-white hover:bg-red-700 disabled:opacity-40"
                title={t('task.rejectByReviewTip')}
              >
                {t('task.rejectByReview')}
              </button>
            )}
          </div>
        )}
        {!doc && !missing && (
          <p className="flex items-center gap-2 py-8 text-center text-[12px] text-slate-400 dark:text-slate-500">
            <Loader2 size={13} className="animate-spin" /> {t('task.phaseLoadingDoc')}
          </p>
        )}
        {missing && (
          <div className="rounded-lg border border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-3 py-2.5 text-[12px] leading-5 text-amber-700">
            {t('task.phaseMissing')}
          </div>
        )}
        {loadErr && (
          /* 2026-10-03 实弹 bug：全文 404 曾静默空白——失败必须显式可见 */
          <div className="rounded-lg border border-red-200 dark:border-red-900/60 bg-red-50 dark:bg-red-950/40 px-3 py-2.5 text-[12px] leading-5 text-red-600">
            {t('task.phaseLoadErr')}
            <button
              onClick={() => {
                setDoc(null)
                setLoadErr(false)
                setRetryTick((t) => t + 1)
              }}
              className="ml-2 rounded-md border border-red-200 dark:border-red-900/60 bg-white dark:bg-slate-900 px-2 py-0.5 text-micro font-bold text-red-500 hover:bg-red-100"
            >
              {t('common.retry')}
            </button>
          </div>
        )}
        {doc && (
          /* Markdown 渲染（非裸文本——2026-10-03 用户反馈：矩阵/方案是 md，pre 纯文本看不清结构） */
          <MarkdownMessage content={doc.content} />
        )}
          </>
        )}
      </div>
      <div className="border-t border-slate-100 dark:border-slate-800 p-3">
        {tab === 'compare' && (
          <p className="mb-1.5 text-center text-[10px] text-slate-400 dark:text-slate-500">
            {readonly ? t('task.compareReadonly') : t('task.compareActive', { title })}
          </p>
        )}
        {readonly ? (
          /* 回看模式（2026-10-05 管道回看）：产物为最新版本，只读；重开走 rewind 端点 */
          <div className="space-y-1.5">
            <p className="text-center text-[10px] text-slate-400 dark:text-slate-500">
              {t('task.phaseLatest')}
            </p>
            {onRewind && (
              <button
                onClick={onRewind}
                disabled={!!rewinding}
                className="w-full rounded-lg bg-violet-600 px-3 py-2 text-[12px] font-bold text-white hover:bg-violet-700 disabled:opacity-40"
                title={t('task.rewindBtnTip')}
              >
                {rewinding ? t('task.rewinding') : t('task.rewindBtn')}
              </button>
            )}
          </div>
        ) : rejecting ? (
          <div className="space-y-1.5">
            <textarea
              autoFocus
              value={note}
              onChange={(e) => setNote(e.target.value)}
              rows={2}
              placeholder={t('task.phaseRejectPh')}
              className="w-full resize-none rounded-lg border border-red-200 dark:border-red-900/60 bg-red-50/40 dark:bg-red-950/30 px-2.5 py-1.5 text-[12px] outline-none focus:border-red-300"
            />
            <div className="flex gap-2">
              <button
                onClick={() => onDecide?.('rejected', note.trim())}
                disabled={!!deciding || !note.trim()}
                className="flex-1 rounded-lg bg-red-600 px-3 py-1.5 text-[12px] font-bold text-white hover:bg-red-700 disabled:opacity-40"
              >
                {t('task.confirmRejectStage')}
              </button>
              <button onClick={() => setRejecting(false)} className="rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-1.5 text-[12px] text-slate-500 dark:text-slate-400">
                取消
              </button>
            </div>
          </div>
        ) : (
          <div className="flex gap-2">
            <button
              onClick={() => onDecide?.('approved')}
              disabled={!!deciding}
              className="flex flex-1 items-center justify-center gap-1 rounded-lg bg-blue-600 px-3 py-2 text-[12px] font-bold text-white hover:bg-blue-700 disabled:opacity-40"
            >
              {deciding === 'approved' ? <Loader2 size={12} className="animate-spin" /> : <CheckCircle2 size={12} />} 评审通过，进入下一阶段
            </button>
            <button
              onClick={() => {
                // 2026-10-05 用户裁定：初审已给意见——打回理由预填初审摘要（可改），不强迫用户手填
                if (review && review.verdict !== 'pass') setNote(review.summary)
                setRejecting(true)
              }}
              disabled={!!deciding}
              className="flex-1 rounded-lg border border-red-200 dark:border-red-900/60 bg-white dark:bg-slate-900 px-3 py-2 text-[12px] font-bold text-red-600 hover:bg-red-50 dark:hover:bg-red-950/40 disabled:opacity-40"
            >
              打回重做…
            </button>
          </div>
        )}
      </div>
    </div>
  )
}
