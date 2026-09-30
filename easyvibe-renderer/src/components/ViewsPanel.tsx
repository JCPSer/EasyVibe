import { useCallback, useEffect, useState } from 'react'
import { toast } from '@/lib/toast'
import { Loader2, ExternalLink, Trash2, Bookmark, Check, Download } from 'lucide-react'
import { MarkdownMessage } from '@/components/MarkdownMessage'

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
  const [views, setViews] = useState<ViewItem[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [confirming, setConfirming] = useState<string | null>(null)
  const [deleting, setDeleting] = useState<string | null>(null)
  const [openedSlug, setOpenedSlug] = useState<string | null>(null)
  const [zoomed, setZoomed] = useState<{ name: string; content: string } | null>(null)

  const load = useCallback(() => {
    if (!backendRepo) return
    setError(null)
    fetch(`/api/repos/${backendRepo}/views`)
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.json()
      })
      .then((d: { data: ViewItem[] }) => setViews(d.data))
      .catch(() => {
        setViews(null)
        setError('视图列表加载失败——需要本地后端在线')
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
    fetch(`/api/repos/${backendRepo}/views/${encodeURIComponent(slug)}`, { method: 'DELETE' })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        setConfirming(null)
        load()
      })
      .catch(() => toast('删除视图失败', 'error'))
      .finally(() => setDeleting(null))
  }

  const open = (v: ViewItem) => {
    const ids = (v.view.nodes ?? []).map((n) => n.ref.replace(/^module:/, '')).filter(Boolean)
    if (ids.length === 0) return
    // R6 清债：视图与主地图漂移——失效引用不静默跳过，提醒用户图已失真
    if (validModuleIds) {
      const stale = ids.filter((id) => !validModuleIds.has(id))
      if (stale.length > 0) {
        toast(`视图有 ${stale.length} 个模块引用已失效（地图已更新）：${stale.slice(0, 3).join('、')}${stale.length > 3 ? '…' : ''}——已按现存模块打开`, 'error')
      }
      const alive = ids.filter((id) => validModuleIds.has(id))
      if (alive.length === 0) {
        toast('视图引用的模块已全部失效（建议删除重建）', 'error')
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
    return <p className="py-8 text-center text-[11.5px] text-slate-400">需要本地后端在线</p>
  }
  if (error) {
    return (
      <div className="py-8 text-center">
        <p className="text-[11.5px] text-red-500">{error}</p>
        <button onClick={load} className="mt-2 rounded-lg border border-slate-200 px-3 py-1 text-[11px] text-slate-600 hover:bg-slate-50">
          重试
        </button>
      </div>
    )
  }
  if (views === null) {
    return (
      <p className="flex items-center justify-center gap-2 py-8 text-[11.5px] text-slate-400">
        <Loader2 size={13} className="animate-spin" /> 加载视图…
      </p>
    )
  }
  if (views.length === 0) {
    return (
      <p className="py-8 text-center text-[11.5px] leading-5 text-slate-400">
        还没有保存的视图。
        <br />
        在<span className="text-slate-500">对话</span>页签提问后，点回答下方的
        <br />
        <span className="text-slate-500">"存为视图"</span>即可创建可复用的模块集合。
      </p>
    )
  }

  return (
    <div className="space-y-2.5">
      <p className="text-[10.5px] leading-4 text-slate-400">
        共 {views.length} 个视图 · 引用式存储（.easyvibe/views/，随仓库走）
      </p>
      {views.map((v) => {
        const moduleIds = (v.view.nodes ?? []).map((n) => n.ref.replace(/^module:/, '')).filter(Boolean)
        const note = v.view.annotations?.find((a) => a.note)?.note
        const mermaid = v.view.annotations?.find((a) => a.type === 'mermaid' && a.content)?.content
        return (
          <div key={v.slug} className="rounded-lg border border-slate-200 p-3">
            <div className="flex items-center gap-1.5">
              <Bookmark size={11} className="shrink-0 text-emerald-500" />
              <span className="truncate text-[12px] font-semibold text-slate-800">{v.name}</span>
              <span className="ml-auto shrink-0 rounded-full bg-slate-100 px-1.5 py-px text-[9px] text-slate-400">
                {v.nodes === 0 && v.view.annotations?.some((a) => a.type === 'mermaid') ? '纯图视图' : `${v.nodes} 个模块`}
              </span>
            </div>
            {note && <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-400">{note}</p>}
            {moduleIds.length > 0 && (
              <div className="mt-1.5 flex flex-wrap gap-1">
                {moduleIds.slice(0, 6).map((id) => (
                  <span key={id} className="rounded-full bg-slate-50 px-1.5 py-px font-mono text-[9px] text-slate-500">
                    {id}
                  </span>
                ))}
                {moduleIds.length > 6 && (
                  <span className="rounded-full bg-slate-50 px-1.5 py-px text-[9px] text-slate-400">
                    +{moduleIds.length - 6}
                  </span>
                )}
              </div>
            )}
            {mermaid && (
              <details className="mt-1.5 rounded-md border border-slate-100 bg-white p-1.5">
                <summary className="cursor-pointer text-[10px] font-semibold text-slate-500 hover:text-slate-700">
                  附：对话生成的流程图
                </summary>
                <div className="mt-1 max-h-72 overflow-auto">
                  <MarkdownMessage content={mermaid} />
                </div>
                <button
                  onClick={() => setZoomed({ name: v.name, content: mermaid })}
                  className="mt-1 rounded-full border border-slate-200 px-2 py-0.5 text-[9.5px] text-slate-500 hover:bg-slate-50"
                >
                  放大查看
                </button>
              </details>
            )}
            <div className="mt-2 flex items-center gap-2">
              <button
                onClick={() => open(v)}
                disabled={moduleIds.length === 0}
                className="flex items-center gap-1 rounded-lg bg-blue-600 px-2.5 py-1 text-[10.5px] font-bold text-white hover:bg-blue-700 disabled:opacity-40"
                title="在画布中定位该视图的模块"
              >
                {openedSlug === v.slug ? <Check size={10} /> : <ExternalLink size={10} />}
                {openedSlug === v.slug ? '已定位' : '打开视图'}
              </button>
              <button
                onClick={() => download(v)}
                className="flex items-center gap-1 rounded-lg border border-slate-200 px-2 py-1 text-[10.5px] text-slate-500 hover:bg-slate-50"
                title="下载视图文件（含流程图，资产可外带）"
              >
                <Download size={10} />
              </button>
              {confirming === v.slug ? (
                <>
                  <button
                    onClick={() => remove(v.slug)}
                    disabled={deleting !== null}
                    className="flex items-center gap-1 rounded-lg bg-red-600 px-2.5 py-1 text-[10.5px] font-bold text-white hover:bg-red-700 disabled:opacity-40"
                  >
                    {deleting === v.slug ? <Loader2 size={10} className="animate-spin" /> : <Trash2 size={10} />}
                    确认删除
                  </button>
                  <button
                    onClick={() => setConfirming(null)}
                    className="rounded-lg px-2 py-1 text-[10.5px] text-slate-400 hover:text-slate-600"
                  >
                    取消
                  </button>
                </>
              ) : (
                <button
                  onClick={() => setConfirming(v.slug)}
                  className="flex items-center gap-1 rounded-lg border border-slate-200 px-2.5 py-1 text-[10.5px] text-slate-500 hover:bg-slate-50 hover:text-red-600"
                  title="删除该视图文件"
                >
                  <Trash2 size={10} />
                  删除
                </button>
              )}
              <span className="ml-auto text-[9px] text-slate-300">{v.createdAt?.slice(0, 10)}</span>
            </div>
          </div>
        )
      })}
      {/* 深挖#B/E：流程图大图查看——340px 卡片里字小看不清，放大到全屏 */}
      {zoomed && (
        <div className="fixed inset-0 z-50 flex flex-col bg-slate-900/80 p-6" onClick={() => setZoomed(null)}>
          <div className="flex flex-1 flex-col overflow-hidden rounded-xl bg-white p-4" onClick={(e) => e.stopPropagation()}>
            <div className="mb-2 flex items-center justify-between">
              <span className="text-[13px] font-bold text-slate-800">{zoomed.name}</span>
              <button onClick={() => setZoomed(null)} className="rounded-full bg-slate-100 px-3 py-1 text-[11px] text-slate-500 hover:bg-slate-200">
                关闭
              </button>
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
