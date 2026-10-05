// 输入坞：@模块建议浮层 + 附件/图片 chips + 回形针 + textarea + 发送/停止。
// 拆自 ChatPanel.tsx（2026-10-05 防膨胀）。
import { useRef } from 'react'
import { AtSign, Paperclip, Send, Square, X as XIcon } from 'lucide-react'
import { toast } from '@/lib/toast'
import type { CodeMap } from '@/types/map'

export function ComposerDock({
  backendRepo, map, input, setInput, mentions, setMentions, attachments, setAttachments, images, setImages,
  suggest, setSuggest, suggestIdx, setSuggestIdx, sending, send, abortRef, textareaRef,
}: {
  backendRepo: string | null
  map: CodeMap | null
  input: string
  setInput: (v: string) => void
  mentions: { id: string; name: string }[]
  setMentions: (updater: (prev: { id: string; name: string }[]) => { id: string; name: string }[]) => void
  attachments: { name: string; size: number; content: string }[]
  setAttachments: (updater: (prev: { name: string; size: number; content: string }[]) => { name: string; size: number; content: string }[]) => void
  images: { name: string; size: number; dataUrl: string }[]
  setImages: (updater: (prev: { name: string; size: number; dataUrl: string }[]) => { name: string; size: number; dataUrl: string }[]) => void
  suggest: { query: string; start: number } | null
  setSuggest: (s: { query: string; start: number } | null) => void
  suggestIdx: number
  setSuggestIdx: (value: number | ((i: number) => number)) => void
  sending: boolean
  send: () => void
  abortRef: React.RefObject<AbortController | null>
  textareaRef: React.RefObject<HTMLTextAreaElement | null>
}) {
  const fileRef = useRef<HTMLInputElement>(null)

  const addFiles = (files: FileList | null) => {
    if (!files) return
    for (const f of Array.from(files)) {
      if (f.type.startsWith('image/')) {
        if (images.length >= 2) {
          toast('图片最多 2 张', 'error')
          continue
        }
        if (f.size > 1.5 * 1024 * 1024) {
          toast(`图片 ${f.name} 超过 1.5MB 上限（当前 ${(f.size / 1024 / 1024).toFixed(1)}MB）`, 'error')
          continue
        }
        const reader = new FileReader()
        reader.onload = () => setImages((prev) => [...prev, { name: f.name, size: f.size, dataUrl: String(reader.result ?? '') }])
        reader.readAsDataURL(f)
      } else {
        if (attachments.length >= 3) {
          toast('文本附件最多 3 个', 'error')
          continue
        }
        if (f.size > 50 * 1024) {
          toast(`附件 ${f.name} 超过 50KB 上限（当前 ${(f.size / 1024).toFixed(0)}KB）——请贴关键片段`, 'error')
          continue
        }
        const reader = new FileReader()
        reader.onload = () => {
          const content = String(reader.result ?? '')
          setAttachments((prev) => [...prev, { name: f.name, size: f.size, content }])
        }
        reader.readAsText(f)
      }
    }
    if (fileRef.current) fileRef.current.value = ''
  }

  const mentionCandidates = (query: string) => {
    if (!map) return []
    const q = query.toLowerCase()
    return map.modules
      .filter((m) => !mentions.some((x) => x.id === m.id))
      .filter((m) => !q || m.id.toLowerCase().includes(q) || m.name.toLowerCase().includes(q))
      .slice(0, 6)
  }

  const handleInputChange = (value: string, caret: number) => {
    setInput(value)
    const before = value.slice(0, caret)
    const m = before.match(/@([^\s@]{0,20})$/)
    if (m && map) {
      setSuggest({ query: m[1], start: caret - m[1].length - 1 })
      setSuggestIdx(() => 0)
    } else {
      setSuggest(null)
    }
  }

  const pickMention = (id: string, name: string) => {
    if (!suggest) return
    const ta = textareaRef.current
    const caret = ta?.selectionStart ?? input.length
    const next = input.slice(0, suggest.start) + input.slice(caret)
    setInput(next)
    setMentions((prev) => [...prev, { id, name }])
    setSuggest(null)
    requestAnimationFrame(() => {
      ta?.focus()
      ta?.setSelectionRange(suggest.start, suggest.start)
    })
  }

  return (
    <>
    {mentions.length > 0 && (
      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        {mentions.map((m) => (
          <span key={m.id} className="flex items-center gap-1 rounded-full bg-blue-600 px-2 py-0.5 text-micro text-white">
            <AtSign size={9} />
            {m.name}
            <button onClick={() => setMentions((prev) => prev.filter((x) => x.id !== m.id))} className="text-blue-200 hover:text-white">
              <XIcon size={9} />
            </button>
          </span>
        ))}
      </div>
    )}
    {(attachments.length > 0 || images.length > 0) && (
      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        {images.map((im, i) => (
          <span key={`img-${i}`} className="relative flex items-center gap-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-1">
            <img src={im.dataUrl} alt={im.name} className="h-8 w-8 rounded object-cover" />
            <span className="max-w-[90px] truncate text-micro text-slate-500 dark:text-slate-400">{im.name}</span>
            <button onClick={() => setImages((prev) => prev.filter((_, j) => j !== i))} className="text-slate-300 dark:text-slate-600 hover:text-red-500">
              <XIcon size={9} />
            </button>
          </span>
        ))}
        {attachments.map((a, i) => (
          <span key={i} className="flex items-center gap-1 rounded-full bg-blue-50 dark:bg-blue-950/40 px-2 py-0.5 text-micro text-blue-700">
            <Paperclip size={9} />
            {a.name}（{(a.size / 1024).toFixed(0)}KB）
            <button onClick={() => setAttachments((prev) => prev.filter((_, j) => j !== i))} className="text-blue-300 hover:text-red-500">
              <XIcon size={9} />
            </button>
          </span>
        ))}
      </div>
    )}
    <div className="mt-3 flex items-end gap-1.5 border-t border-slate-100 dark:border-slate-800 pt-3">
      <input
        ref={fileRef}
        type="file"
        multiple
        className="hidden"
        onChange={(e) => addFiles(e.target.files)}
      />
      {/* 回形针 ghost 化：次要动作不再与发送按钮同视觉重量（设计师评审#7） */}
      <button
        onClick={() => fileRef.current?.click()}
        disabled={!backendRepo || attachments.length >= 3}
        className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg text-slate-400 transition-colors hover:bg-slate-100 hover:text-slate-600 dark:text-slate-500 dark:hover:bg-slate-800/70 dark:hover:text-slate-300 disabled:opacity-40"
        title="添加附件（代码/日志/文档文本，≤50KB×3）——内容随消息一起发给 AI"
      >
        <Paperclip size={14} />
      </button>
      <div className="relative flex-1">
        {suggest && mentionCandidates(suggest.query).length > 0 && (
          /* D9 @模块建议浮层：@ 后输入即过滤，↑↓ 选择，Enter/Tab 选中，Esc 关闭 */
          <div className="absolute bottom-full left-0 z-20 mb-1 w-64 overflow-hidden rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 shadow-lg">
            {mentionCandidates(suggest.query).map((m, i) => (
              <button
                key={m.id}
                onMouseDown={(e) => {
                  e.preventDefault()
                  pickMention(m.id, m.name)
                }}
                onMouseEnter={() => setSuggestIdx(i)}
                className={`flex w-full items-center gap-2 px-2.5 py-1.5 text-left ${i === suggestIdx ? 'bg-blue-50 dark:bg-blue-950/40' : 'bg-white dark:bg-slate-900'}`}
              >
                <AtSign size={10} className="shrink-0 text-blue-400" />
                <span className="truncate text-[12px] font-medium text-slate-700 dark:text-slate-200">{m.name}</span>
                <span className="ml-auto shrink-0 font-mono text-micro text-slate-400 dark:text-slate-500">{m.id}</span>
              </button>
            ))}
          </div>
        )}
        {/* 新手引导：空态示例 chip 已移至上方邀请卡（设计师评审#8），此处不再常驻 */}
        <textarea
          ref={textareaRef}
          value={input}
          onChange={(e) => handleInputChange(e.target.value, e.target.selectionStart)}
          onKeyDown={(e) => {
            const cands = suggest ? mentionCandidates(suggest.query) : []
            if (suggest && cands.length > 0) {
              if (e.key === 'ArrowDown') {
                e.preventDefault()
                setSuggestIdx((i) => (i + 1) % cands.length)
                return
              }
              if (e.key === 'ArrowUp') {
                e.preventDefault()
                setSuggestIdx((i) => (i - 1 + cands.length) % cands.length)
                return
              }
              if (e.key === 'Enter' || e.key === 'Tab') {
                e.preventDefault()
                pickMention(cands[suggestIdx].id, cands[suggestIdx].name)
                return
              }
              if (e.key === 'Escape') {
                setSuggest(null)
                return
              }
            }
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault()
              send()
            }
          }}
          rows={2}
          placeholder={backendRepo ? '问点什么，@ 可引用模块' : '需要本地后端在线'}
          title="Enter 发送，Shift+Enter 换行"
          disabled={!backendRepo || sending}
          className="w-full flex-1 resize-none rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-950/70 px-3 py-2 text-[12px] leading-5 text-slate-700 dark:text-slate-200 outline-none transition-colors focus:border-blue-300 focus:ring-2 focus:ring-blue-100 disabled:opacity-50"
        />
      </div>
      {sending ? (
        <button
          onClick={() => abortRef.current?.abort()}
          className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-red-200 text-red-600 transition-colors hover:bg-red-50 dark:border-red-900/60 dark:hover:bg-red-950/40"
          title="停止等待本次回答"
        >
          <Square size={13} />
        </button>
      ) : (
        /* 禁用态换材质而非变淡（设计师评审#7）：空输入=灰底灰字，有输入=实蓝——
            状态对比由颜色承担，不再靠透明度 */
        <button
          onClick={send}
          disabled={!backendRepo}
          className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-lg transition-colors ${
            input.trim() && backendRepo
              ? 'bg-blue-600 text-white hover:bg-blue-700'
              : 'bg-slate-100 text-slate-400 dark:bg-slate-800 dark:text-slate-600'
          }`}
          title="发送（Enter）"
        >
          <Send size={14} />
        </button>
      )}
    </div>
    </>
  )
}
