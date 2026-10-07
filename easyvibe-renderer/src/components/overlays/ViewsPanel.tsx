import { useCallback, useEffect, useState } from 'react'
import { toast } from '@/runtime/toast'
import { useLang } from '@/runtime/i18n'
import { Loader2, ExternalLink, Trash2, Bookmark, Check, Download, Pencil } from 'lucide-react'
import { MarkdownMessage } from '@/shared/primitives/MarkdownMessage'
import { views as viewsApi, renameView, deleteView } from '@/api/chat'

interface ViewItem {
  slug: string
  name: string
  createdAt: string
  nodes: number
  view: {
    nodes?: { ref: string }[]
    annotations?: { note?: string; type?: string; content?: string }[]
  }
}

interface Props {
  backendRepo: string | null
  /** 打开视图：定位到首个模块（画布选中 + 详情展示） */
  onOpenView: (moduleIds: string[]) => void
  /** R6：当前地图模块 id 集合——打开视图时校验引用有效性（漂移提醒） */
  validModuleIds?: Set<string>
}

// 视图列表（F1b 读侧）：对话中"存为视图"的引用式资产可回读、可打开、可删除。
// 四态齐全（§0 标准）：加载 / 空 / 错误 / 成功；删除二次确认。
export function ViewsPanel({ backendRepo, onOpenView, validModuleIds }: Props) {
  const { t } = useLang()
  const [views, setViews] = useState<ViewItem[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [confirming, setConfirming] = useState<string | null>(null)
  const [deleting, setDeleting] = useState<string | null>(null)
  // 重审 P1：视图改名（此前只能删了重建——保存冲突还会静默另存新文件造成列表膨胀）
  const [renamingSlug, setRenamingSlug] = useState<string | null>(null)
  const [renameVal, setRenameVal] = useState('')
  const [openedSlug, setOpenedSlug] = useState<string | null>(null)
  const [zoomed, setZoomed] = useState<{ name: string; content: string } | null>(null)

  const load = useCallback(() => {
    if (!backendRepo) return
    setError(null)
    viewsApi(backendRepo)
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.json()
      })
      .then((d: { data: ViewItem[] }) => setViews(d.data))
      .catch(() => {
        setViews(null)
        setError(t('views.loadFail'))
      })
  }, [backendRepo])

  useEffect(() => {
    setViews(null)
    load()
  }, [load])

  // §12d 兑现：视图导出（JSON 文件下载，含 mermaid——资产可外带）
  const download = (v: ViewItem) => {
    const blob = new Blob([JSON.stringify(v.view, null, 2)], { type: 'application/json' })
    const a = document.createElement('a')
    a.href = URL.createObjectURL(blob)
    a.download = `${v.slug}.json`
    a.click()
    URL.revokeObjectURL(a.href)
  }

  const remove = (slug: string) => {
    if (!backendRepo) return
    setDeleting(slug)
    deleteView(backendRepo, slug)
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        setConfirming(null)
        load()
      })
      .catch(() => toast(t('views.deleteFail'), 'error'))
      .finally(() => setDeleting(null))
  }

  const rename = (slug: string) => {
    if (!backendRepo || !renameVal.trim()) return
    renameView(backendRepo, slug, renameVal.trim())
      .then(async (r) => {
        if (!r.ok) {
          const d = await r.json().catch(() => null)
          throw new Error(d?.error ?? String(r.status))
        }
        setRenamingSlug(null)
        toast(t('views.renamed'))
        load()
      })
      .catch((e) => toast(e instanceof Error ? e.message : t('views.renameFail'), 'error'))
  }

  const open = (v: ViewItem) => {
    const ids = (v.view.nodes ?? []).map((n) => n.ref.replace(/^module:/, '')).filter(Boolean)
    if (ids.length === 0) return
    // R6 清债：视图与主地图漂移——失效引用不静默跳过，提醒用户图已失真
    if (validModuleIds) {
      const stale = ids.filter((id) => !validModuleIds.has(id))
      if (stale.length > 0) {
        toast(t('views.staleRefs', { n: stale.length, list: stale.slice(0, 3).join('、') + (stale.length > 3 ? '…' : '') }), 'error')
      }
      const alive = ids.filter((id) => validModuleIds.has(id))
      if (alive.length === 0) {
        toast(t('views.allStale'), 'error')
        return
      }
      onOpenView(alive)
    } else {
      onOpenView(ids)
    }
    setOpenedSlug(v.slug)
    setTimeout(() => setOpenedSlug(null), 2000)
  }

  if (!backendRepo) {
    return <p className="py-8 text-center text-[12px] text-slate-400 dark:text-slate-500">{t('common.needBackend')}</p>
  }
  if (error) {
    return (
      <div className="py-8 text-center">
        <p className="text-[12px] text-red-500">{error}</p>
        <button onClick={load} className="mt-2 rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-1 text-[11px] text-slate-600 dark:text-slate-300 hover:bg-slate-50 dark:hover:bg-slate-800/70">
          重试
        </button>
      </div>
    )
  }
  if (views === null) {
    return (
      <p className="flex items-center justify-center gap-2 py-8 text-[12px] text-slate-400 dark:text-slate-500">
        <Loader2 size={13} className="animate-spin" /> {t('views.loading')}
      </p>
    )
  }
  if (views.length === 0) {
    return (
      <p className="py-8 text-center text-[12px] leading-5 text-slate-400 dark:text-slate-500">
        {t('views.empty1')}
        <br />
        {t('views.empty2')}
        <br />
        <span className="text-slate-500 dark:text-slate-400">{t('views.empty3')}</span>
      </p>
    )
  }

  return (
    <div className="space-y-2.5">
      <p className="text-cap leading-4 text-slate-400 dark:text-slate-500">
        {t('views.countLine', { n: views.length })}
      </p>
      {views.map((v) => {
        const moduleIds = (v.view.nodes ?? []).map((n) => n.ref.replace(/^module:/, '')).filter(Boolean)
        const note = v.view.annotations?.find((a) => a.note)?.note
        const mermaid = v.view.annotations?.find((a) => a.type === 'mermaid' && a.content)?.content
        return (
          <div key={v.slug} className="rounded-lg border border-slate-200 dark:border-slate-700 p-3">
            <div className="flex items-center gap-1.5">
              <Bookmark size={11} className="shrink-0 text-emerald-500" />
              {renamingSlug === v.slug ? (
                <span className="flex min-w-0 flex-1 items-center gap-1">
                  <input
                    autoFocus
                    value={renameVal}
                    onChange={(e) => setRenameVal(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') rename(v.slug)
                      if (e.key === 'Escape') setRenamingSlug(null)
                    }}
                    className="min-w-0 flex-1 rounded border border-blue-200 dark:border-blue-900/60 bg-white dark:bg-slate-900 px-1.5 py-px text-[12px] outline-none focus:border-blue-400"
                  />
                  <button onClick={() => rename(v.slug)} className="shrink-0 rounded bg-blue-600 p-0.5 text-white" title={t('views.saveName')}>
                    <Check size={10} />
                  </button>
                  <button onClick={() => setRenamingSlug(null)} className="shrink-0 rounded p-0.5 text-slate-400 dark:text-slate-500 hover:text-slate-600" title={t('common.cancel')}>
                    <span className="text-[10px]">✕</span>
                  </button>
                </span>
              ) : (
                <span className="truncate text-[12px] font-semibold text-slate-800 dark:text-slate-100">{v.name}</span>
              )}
              <span className="ml-auto shrink-0 rounded-full bg-slate-100 dark:bg-slate-800 px-1.5 py-px text-micro text-slate-400 dark:text-slate-500">
                {v.nodes === 0 && v.view.annotations?.some((a) => a.type === 'mermaid') ? t('views.pureMermaid') : t('views.modulesCount', { n: v.nodes })}
              </span>
            </div>
            {note && <p className="mt-1 line-clamp-2 text-micro leading-4 text-slate-400 dark:text-slate-500">{note}</p>}
            {moduleIds.length > 0 && (
              <div className="mt-1.5 flex flex-wrap gap-1">
                {moduleIds.slice(0, 6).map((id) => (
                  <span key={id} className="rounded-full bg-slate-50 dark:bg-slate-950/70 px-1.5 py-px font-mono text-micro text-slate-500 dark:text-slate-400">
                    {id}
                  </span>
                ))}
                {moduleIds.length > 6 && (
                  <span className="rounded-full bg-slate-50 dark:bg-slate-950/70 px-1.5 py-px text-micro text-slate-400 dark:text-slate-500">
                    +{moduleIds.length - 6}
                  </span>
                )}
              </div>
            )}
            {mermaid && (
              <details className="mt-1.5 rounded-md border border-slate-100 dark:border-slate-800 bg-white dark:bg-slate-900 p-1.5">
                <summary className="cursor-pointer text-micro font-semibold text-slate-500 dark:text-slate-400 hover:text-slate-700">
                  {t('views.mermaidAttached')}
                </summary>
                <div className="mt-1 max-h-72 overflow-auto">
                  <MarkdownMessage content={mermaid} />
                </div>
                <button
                  onClick={() => setZoomed({ name: v.name, content: mermaid })}
                  className="mt-1 rounded-full border border-slate-200 dark:border-slate-700 px-2 py-0.5 text-micro text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70"
                >
                  {t('views.zoom')}
                </button>
              </details>
            )}
            <div className="mt-2 flex items-center gap-2">
              <button
                onClick={() => open(v)}
                disabled={moduleIds.length === 0}
                className="flex items-center gap-1 rounded-lg bg-blue-600 px-2.5 py-1 text-cap font-bold text-white hover:bg-blue-700 disabled:opacity-40"
                title={t('views.openTip')}
              >
                {openedSlug === v.slug ? <Check size={10} /> : <ExternalLink size={10} />}
                {openedSlug === v.slug ? t('views.located') : t('views.open')}
              </button>
              <button
                onClick={() => download(v)}
                className="flex items-center gap-1 rounded-lg border border-slate-200 dark:border-slate-700 px-2 py-1 text-cap text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70"
                title={t('views.downloadTip')}
              >
                <Download size={10} />
              </button>
              <button
                onClick={() => {
                  setRenamingSlug(renamingSlug === v.slug ? null : v.slug)
                  setRenameVal(v.name)
                }}
                className="flex items-center gap-1 rounded-lg border border-slate-200 dark:border-slate-700 px-2 py-1 text-cap text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70 hover:text-blue-600"
                title={t('views.renameTip')}
              >
                <Pencil size={10} />
              </button>
              {confirming === v.slug ? (
                <>
                  <button
                    onClick={() => remove(v.slug)}
                    disabled={deleting !== null}
                    className="flex items-center gap-1 rounded-lg bg-red-600 px-2.5 py-1 text-cap font-bold text-white hover:bg-red-700 disabled:opacity-40"
                  >
                    {deleting === v.slug ? <Loader2 size={10} className="animate-spin" /> : <Trash2 size={10} />}
                    {t('views.delConfirm')}
                  </button>
                  <button
                    onClick={() => setConfirming(null)}
                    className="rounded-lg px-2 py-1 text-cap text-slate-400 dark:text-slate-500 hover:text-slate-600"
                  >
                    {t('common.cancel')}
                  </button>
                </>
              ) : (
                <button
                  onClick={() => setConfirming(v.slug)}
                  className="flex items-center gap-1 rounded-lg border border-slate-200 dark:border-slate-700 px-2.5 py-1 text-cap text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70 hover:text-red-600"
                  title={t('views.deleteTip')}
                >
                  <Trash2 size={10} />
                  {t('views.delete')}
                </button>
              )}
              <span className="ml-auto text-micro text-slate-300 dark:text-slate-600">{v.createdAt?.slice(0, 10)}</span>
            </div>
          </div>
        )
      })}
      {/* 深挖#B/E：流程图大图查看——340px 卡片里字小看不清，放大到全屏 */}
      {zoomed && (
        <div className="fixed inset-0 z-50 flex flex-col bg-slate-900/80 p-6" onClick={() => setZoomed(null)}>
          <div className="flex flex-1 flex-col overflow-hidden rounded-xl bg-white dark:bg-slate-900 p-4" onClick={(e) => e.stopPropagation()}>
            <div className="mb-2 flex items-center justify-between">
              <span className="text-[13px] font-bold text-slate-800 dark:text-slate-100">{zoomed.name}</span>
              <button onClick={() => setZoomed(null)} className="rounded-full bg-slate-100 dark:bg-slate-800 px-3 py-1 text-[11px] text-slate-500 dark:text-slate-400 hover:bg-slate-200">{t('views.close')}</button>
            </div>
            <div className="flex-1 overflow-auto">
              <MarkdownMessage content={zoomed.content} />
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
