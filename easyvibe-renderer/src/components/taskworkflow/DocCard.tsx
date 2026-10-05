// 产物文档卡：点击展开全文（拉 /dev-doc），两步确认删除。
// 拆自 TaskWorkflowPage.tsx（2026-10-05 防膨胀）。
import { useEffect, useState } from 'react'
import { ChevronRight, Loader2, Trash2 } from 'lucide-react'
import { toast } from '@/lib/toast'

/** 产物文档卡（审计 P2：此前纯只读死胡同）——点击标题展开全文（拉 /dev-doc），
 *  再点收起；展开态本地缓存避免重复请求。删除两步确认（审计 P1：归档只进不出收口） */
export function DocCard({ backendRepo, doc, onDeleted }: { backendRepo: string; doc: { path: string; name: string; excerpt?: string }; onDeleted?: () => void }) {
  const [open, setOpen] = useState(false)
  const [full, setFull] = useState<string | null>(null)
  const [err, setErr] = useState(false)
  const [confirmDel, setConfirmDel] = useState(false)
  const [deleting, setDeleting] = useState(false)
  useEffect(() => {
    if (!open || full !== null || err) return
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/dev-doc?path=${encodeURIComponent(doc.path)}`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data?: { content?: string } }) => setFull(d?.data?.content ?? ''))
      .catch(() => setErr(true))
  }, [open, full, err, backendRepo, doc.path])
  const remove = () => {
    setDeleting(true)
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/dev-doc`, {
      method: 'DELETE',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: doc.path }),
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        toast('产物文档已删除', 'info')
        onDeleted?.()
      })
      .catch(() => toast('删除失败', 'error'))
      .finally(() => {
        setDeleting(false)
        setConfirmDel(false)
      })
  }
  return (
    <div className="rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-2">
      <div className="flex items-center gap-1">
        <button
          onClick={() => setOpen((v) => !v)}
          className="flex min-w-0 flex-1 items-center gap-1 text-left"
          title={open ? '收起全文' : '打开全文'}
        >
          <ChevronRight size={10} className={`shrink-0 text-slate-300 dark:text-slate-600 transition-transform ${open ? 'rotate-90' : ''}`} />
          <span className="min-w-0 flex-1 truncate text-[11px] font-semibold text-slate-700 dark:text-slate-200">{doc.name}</span>
        </button>
        {confirmDel ? (
          <button
            onClick={remove}
            disabled={deleting}
            className="shrink-0 rounded bg-red-500 px-1.5 py-0.5 text-[9px] font-bold text-white disabled:opacity-40"
            title="确认删除该产物文档（不可恢复）"
          >
            {deleting ? '…' : '确认'}
          </button>
        ) : (
          <button
            onClick={() => setConfirmDel(true)}
            className="shrink-0 rounded p-0.5 text-slate-300 dark:text-slate-600 hover:bg-red-50 dark:hover:bg-red-950/40 hover:text-red-500"
            title="删除该产物文档"
            onMouseLeave={() => setConfirmDel(false)}
          >
            <Trash2 size={10} />
          </button>
        )}
      </div>
      <p className="mono mt-0.5 truncate text-[9px] text-slate-400 dark:text-slate-500" title={doc.path}>{doc.path}</p>
      {!open && <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-500 dark:text-slate-400">{doc.excerpt || '（空文档）'}</p>}
      {open && (
        <div className="mt-1.5 max-h-64 overflow-y-auto rounded-md bg-slate-50 dark:bg-slate-950/70 p-2">
          {full === null && !err && <p className="text-[10px] text-slate-400 dark:text-slate-500"><Loader2 size={10} className="mr-1 inline animate-spin" />加载全文…</p>}
          {err && <p className="text-[10px] text-red-500">全文加载失败</p>}
          {full !== null && <pre className="select-text whitespace-pre-wrap break-all font-mono text-[10px] leading-4 text-slate-600 dark:text-slate-300">{full || '（空文档）'}</pre>}
        </div>
      )}
    </div>
  )
}
