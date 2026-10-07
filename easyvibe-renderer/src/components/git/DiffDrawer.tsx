import { useCallback, useEffect, useMemo, useState } from 'react'
import { AlertTriangle, FileQuestion, FileWarning, Loader2, X } from 'lucide-react'
import { gitDiff } from '@/api/git'
import { useLang } from '@/runtime/i18n'
import { countDiffStats, parseDiffText } from './diffView'
import type { GitFile } from './types'

// 后端 easyvibe-git::FileDiff 序列化形状（camelCase）。
interface FileDiffData {
  path: string
  untracked: boolean
  binary: boolean
  empty: boolean
  truncated: boolean
  totalLines: number
  text: string
}

/** 截断展示上限（与后端 DIFF_MAX_LINES 对齐，用于提示文案）。 */
const MAX_LINES = 2000

const ST_CLS: Record<string, string> = {
  M: 'bg-amber-100 text-amber-700',
  A: 'bg-emerald-100 text-emerald-700',
  D: 'bg-red-100 text-red-600',
  R: 'bg-indigo-100 text-indigo-700',
  '?': 'bg-slate-100 dark:bg-slate-800 text-slate-500 dark:text-slate-400',
}

/** Git 页 diff 抽屉：点击变更文件后右侧滑出，展示统一差异。Esc / ✕ / 点遮罩关闭。 */
export function DiffDrawer({
  backendRepo,
  file,
  onClose,
}: {
  backendRepo: string
  file: GitFile
  onClose: () => void
}) {
  const { t } = useLang()
  const [data, setData] = useState<FileDiffData | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)

  const load = useCallback(() => {
    setLoading(true)
    setError(null)
    setData(null)
    gitDiff(backendRepo, file.path, false)
      .then(async (r) => {
        const d = await r.json().catch(() => null)
        if (!r.ok) throw new Error(d?.error ?? `HTTP ${r.status}`)
        return d?.data as FileDiffData | undefined
      })
      .then((d) => setData(d ?? null))
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setLoading(false))
  }, [backendRepo, file.path])

  useEffect(() => {
    load()
  }, [load])

  // Esc 关闭（ViewsDrawer 同款交互）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])

  const lines = useMemo(() => (data && !data.binary && !data.empty ? parseDiffText(data.text) : []), [data])
  const stats = useMemo(() => countDiffStats(lines), [lines])

  const displayPath = file.orig ? `${file.orig} → ${file.path}` : file.path

  return (
    <div className="anim-fade-in-fast fixed inset-0 z-50 flex justify-end bg-slate-900/20" onClick={onClose}>
      <div
        className="anim-drawer-in flex h-full w-[55%] min-w-[420px] flex-col border-l border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label={t('git.diff.title')}
      >
        {/* 头部：路径 + 状态 + 对比基线 + 关闭 */}
        <div className="flex items-center gap-2 border-b border-slate-100 dark:border-slate-800 px-4 py-2.5">
          <span
            className={`flex h-[17px] w-[17px] shrink-0 items-center justify-center rounded-[5px] text-micro font-extrabold ${ST_CLS[file.status] ?? ST_CLS['?']}`}
          >
            {file.status}
          </span>
          <span className="mono min-w-0 flex-1 truncate text-[12px] font-semibold text-slate-700 dark:text-slate-200" title={displayPath}>
            {displayPath}
          </span>
          <span className="shrink-0 rounded-full bg-slate-100 dark:bg-slate-800 px-2 py-0.5 text-micro font-semibold text-slate-500 dark:text-slate-400">
            {t('git.diff.baseWorktree')}
          </span>
          <button
            onClick={onClose}
            title={t('git.diff.closeTip')}
            className="shrink-0 rounded p-1 text-slate-400 dark:text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-600"
          >
            <X size={15} />
          </button>
        </div>

        {/* 内容区 */}
        <div className="min-h-0 flex-1 overflow-auto bg-slate-50/60 dark:bg-slate-950/60">
          {loading && (
            <div className="flex h-full items-center justify-center gap-2 text-[12px] text-slate-400 dark:text-slate-500">
              <Loader2 size={13} className="animate-spin" />
              {t('git.diff.loading')}
            </div>
          )}
          {!loading && error && (
            <div className="flex h-full flex-col items-center justify-center gap-2 px-6 text-center">
              <AlertTriangle size={18} className="text-amber-500" />
              <p className="text-[12px] text-slate-500 dark:text-slate-400">
                {t('git.diff.loadFailed')}：{error}
              </p>
              <button
                onClick={load}
                className="rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-1 text-cap font-semibold text-slate-600 dark:text-slate-300 hover:bg-slate-50 dark:hover:bg-slate-800/70"
              >
                {t('git.diff.retry')}
              </button>
            </div>
          )}
          {!loading && !error && data?.binary && (
            <div className="flex h-full flex-col items-center justify-center gap-2 px-6 text-center">
              <FileWarning size={18} className="text-slate-400 dark:text-slate-500" />
              <p className="text-[12px] text-slate-500 dark:text-slate-400">{t('git.diff.binary')}</p>
            </div>
          )}
          {!loading && !error && data?.empty && (
            <div className="flex h-full flex-col items-center justify-center gap-2 px-6 text-center">
              <FileQuestion size={18} className="text-slate-400 dark:text-slate-500" />
              <p className="text-[12px] text-slate-500 dark:text-slate-400">{t('git.diff.empty')}</p>
              <p className="text-micro text-slate-300 dark:text-slate-600">{t('git.diff.stagedHint')}</p>
            </div>
          )}
          {!loading && !error && data && !data.binary && !data.empty && (
            <div className="py-2">
              {data.untracked && (
                <p className="px-4 pb-1 text-micro font-semibold text-emerald-600 dark:text-emerald-400">
                  {t('git.diff.untracked')}
                </p>
              )}
              {lines.map((l, i) => (
                <div
                  key={i}
                  className={
                    l.kind === 'add'
                      ? 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-400'
                      : l.kind === 'del'
                        ? 'bg-red-500/10 text-red-600 dark:text-red-400'
                        : l.kind === 'context'
                          ? 'text-slate-600 dark:text-slate-300'
                          : 'text-slate-400 dark:text-slate-500' // file / hunk / meta 弱化
                  }
                >
                  <span className="mono block whitespace-pre-wrap break-all px-4 py-px text-[11.5px] leading-[18px]">{l.text}</span>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* 底部：截断提示 + 行数统计 */}
        <div className="flex items-center gap-3 border-t border-slate-100 dark:border-slate-800 px-4 py-2 text-micro text-slate-400 dark:text-slate-500">
          {data?.truncated && (
            <span className="flex items-center gap-1 font-semibold text-amber-600">
              <AlertTriangle size={10} />
              {t('git.diff.truncated', { max: MAX_LINES, total: data.totalLines })}
            </span>
          )}
          <span className="tnum ml-auto">
            <i className="not-italic font-bold text-emerald-600">+{stats.adds}</i>{' '}
            <i className="not-italic font-bold text-red-400">−{stats.dels}</i>
            {lines.length > 0 && <> · {t('git.diff.lineCount', { count: data?.totalLines ?? lines.length })}</>}
          </span>
        </div>
      </div>
    </div>
  )
}
