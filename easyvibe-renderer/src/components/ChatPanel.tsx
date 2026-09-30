import { useEffect, useRef, useState } from 'react'
import { MarkdownMessage } from '@/components/MarkdownMessage'
import type { TaskDraft } from '@/lib/taskContext'
import { Send, Loader2, BookmarkPlus, Check, Crosshair, Shrink, RotateCcw, Wrench, Square, Paperclip, X as XIcon} from 'lucide-react'

interface ChatMessage {
  role: 'user' | 'assistant' | 'system'
  content: string
  refs: string[]
  /** 图片附件（dataURL；仅当前会话内存，刷新后不还原） */
  images?: { name: string; dataUrl: string }[]
}

interface Clarify {
  question: string
  options: { label: string; desc?: string }[]
  why?: string
}

interface Props {
  backendRepo: string | null
  onLocateModule: (moduleId: string) => void
  /** S1：对话升级任务入口——把本轮对话组织成 TaskDraft 交给任务表单 */
  onCreateTask: (draft: TaskDraft) => void
}

interface ChatRestore {
  summary: string | null
  messages: { id: number; role: string; content: string }[]
  usage: { promptTokens: number; completionTokens: number }
}

// 入口对话（F2 + M3-5 会话持久化）：服务端 SQLite 是会话事实源——
// 切换页签/刷新/后端重启均从库恢复（不再只活在前端 state）；支持手动压缩与 auto-compact 留痕
export function ChatPanel({ backendRepo, onLocateModule, onCreateTask }: Props) {
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [usage, setUsage] = useState({ promptTokens: 0, completionTokens: 0 })
  const [input, setInput] = useState('')
  const [sending, setSending] = useState(false)
  const [savedIdx, setSavedIdx] = useState<number | null>(null)
  const [compacting, setCompacting] = useState(false)
  const [clarify, setClarify] = useState<Clarify | null>(null)
  const abortRef = useRef<AbortController | null>(null)
  // 附件：文本类内容随消息注入（50KB×3）；图片走 images 通道（1.5MB×2，视觉模型消费）
  const [attachments, setAttachments] = useState<{ name: string; size: number; content: string }[]>([])
  const [images, setImages] = useState<{ name: string; size: number; dataUrl: string }[]>([])
  const fileRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)

  // 恢复会话（M3-5：历史从域 2 读，完整原文含压缩留痕）
  // stale 保护：快速切换仓库时，旧 fetch 返回不得覆盖新会话（审查 🟡2）
  useEffect(() => {
    if (!backendRepo) return
    let stale = false
    setMessages([])
    fetch(`/api/repos/${backendRepo}/chat`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: ChatRestore }) => {
        if (stale) return
        setUsage(d.data.usage)
        setMessages(
          d.data.messages.map((m) => ({
            role: m.role as ChatMessage['role'],
            content: m.content,
            refs: [],
          })),
        )
        setTimeout(() => listRef.current?.scrollTo({ top: listRef.current.scrollHeight }), 50)
      })
      .catch(() => {
        /* 后端不在线：保持空会话，发送时会有错误提示 */
      })
    return () => {
      stale = true
    }
  }, [backendRepo])

  const addFiles = (files: FileList | null) => {
    if (!files) return
    for (const f of Array.from(files)) {
      if (f.type.startsWith('image/')) {
        if (images.length >= 2) {
          alert('图片最多 2 张')
          continue
        }
        if (f.size > 1.5 * 1024 * 1024) {
          alert(`图片 ${f.name} 超过 1.5MB 上限（当前 ${(f.size / 1024 / 1024).toFixed(1)}MB）`)
          continue
        }
        const reader = new FileReader()
        reader.onload = () => setImages((prev) => [...prev, { name: f.name, size: f.size, dataUrl: String(reader.result ?? '') }])
        reader.readAsDataURL(f)
      } else {
        if (attachments.length >= 3) {
          alert('文本附件最多 3 个')
          continue
        }
        if (f.size > 50 * 1024) {
          alert(`附件 ${f.name} 超过 50KB 上限（当前 ${(f.size / 1024).toFixed(0)}KB）——请贴关键片段`)
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

  const send = () => {
    const q = input.trim()
    if (!q || sending || !backendRepo) return
    setInput('')
    setSending(true)
    // 服务端为事实源：只发当前消息，历史由后端从库装配（摘要+未压缩窗口）
    setClarify(null)
    const withAttach =
      attachments.length > 0
        ? q +
          '\n\n' +
          attachments.map((a) => `--- 附件：${a.name}（${(a.size / 1024).toFixed(1)}KB）---\n\`\`\`\n${a.content}\n\`\`\``).join('\n\n')
        : q
    setMessages((prev) => [
      ...prev,
      { role: 'user', content: withAttach, refs: [], images: images.map((i) => ({ name: i.name, dataUrl: i.dataUrl })) },
    ])
    setAttachments([])
    const sendImages = images.map((i) => i.dataUrl)
    setImages([])
    abortRef.current?.abort()
    const ac = new AbortController()
    abortRef.current = ac
    fetch(`/api/repos/${backendRepo}/chat`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: withAttach, images: sendImages }),
      signal: ac.signal,
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.json()
      })
      .then((d: { data: { reply: string; refs: string[]; compaction: string | null; clarify: Clarify | null; usage: { promptTokens: number; completionTokens: number } } }) => {
        setUsage(d.data.usage)
        setClarify(d.data.clarify)
        setMessages((prev) => [
          ...prev,
          { role: 'assistant', content: d.data.reply, refs: d.data.refs },
          // auto-compact 留痕（§10a："上下文已压缩：82%→34%"系统消息）
          ...(d.data.compaction ? [{ role: 'system' as const, content: d.data.compaction, refs: [] }] : []),
        ])
        setTimeout(() => listRef.current?.scrollTo({ top: listRef.current.scrollHeight }), 50)
      })
      .catch((e) => {
        if (ac.signal.aborted) {
          setMessages((prev) => [...prev, { role: 'system', content: '已停止生成（后端调用已发出，token 已计费）', refs: [] }])
        } else {
          setMessages((prev) => [...prev, { role: 'assistant', content: `（对话服务不可用：${String(e).slice(0, 80)}）`, refs: [] }])
        }
      })
      .finally(() => {
        if (abortRef.current === ac) {
          abortRef.current = null
          setSending(false)
        }
      })
  }

  // 手动压缩（§10a：压缩上下文按钮；自动阈值兜底之外的主动手段）
  const compact = () => {
    if (!backendRepo || compacting) return
    setCompacting(true)
    fetch(`/api/repos/${backendRepo}/chat/compact`, { method: 'POST' })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.json()
      })
      .then((d: { data: { compacted: boolean; trace: string | null } }) => {
        if (d.data.compacted && d.data.trace) {
          setMessages((prev) => [...prev, { role: 'system', content: d.data.trace!, refs: [] }])
        }
        setTimeout(() => listRef.current?.scrollTo({ top: listRef.current.scrollHeight }), 50)
      })
      .catch(() => alert('压缩失败（需要本地后端在线）'))
      .finally(() => setCompacting(false))
  }

  // 新对话：清空服务端会话（消息/摘要/计数归零）
  const reset = () => {
    if (!backendRepo || sending) return
    fetch(`/api/repos/${backendRepo}/chat/reset`, { method: 'POST' })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        setMessages([])
        setUsage({ promptTokens: 0, completionTokens: 0 })
      })
      .catch(() => alert('重置失败（需要本地后端在线）'))
  }

  // 澄清卡点选：选择即回答（grill-me 选择题形态的产品化；选择带建议理由回传）
  const answerClarify = (label: string, desc?: string) => {
    setClarify(null)
    setInput(`选择：${label}${desc ? `（${desc}）` : ''}`)
    setTimeout(() => {
      const btn = document.querySelector<HTMLTextAreaElement>('textarea[placeholder^="问点什么"]')
      btn?.focus()
    }, 50)
  }

  // S1：对话升级任务——最近用户问题 + 对话摘要 + 引用模块 → TaskDraft（表单可再编辑）
  const upgradeToTask = () => {
    const turns = messages.filter((m) => m.role !== 'system')
    const lastUser = [...turns].reverse().find((m) => m.role === 'user')
    if (!lastUser) return
    const refs = [...new Set(turns.flatMap((m) => m.refs))].slice(0, 5)
    const summary = turns
      .slice(-6)
      .map((m) => `${m.role === 'user' ? '问' : '答'}：${m.content.slice(0, 120)}`)
      .join('\n')
    onCreateTask({
      title: `对话：${lastUser.content.slice(0, 16)}`,
      description: `${lastUser.content}\n\n—— 来自对话的已澄清需求，见上下文中的对话摘要。`,
      modules: refs,
      acceptance: '',
      source: 'manual',
      context: { inject: { conversation: summary, refs } },
    })
  }

  const saveAsView = (idx: number) => {
    const m = messages[idx]
    const q = messages.slice(0, idx).reverse().find((x) => x.role === 'user')?.content ?? '对话视图'
    const name = q.slice(0, 24)
    fetch(`/api/repos/${backendRepo}/views`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        name,
        nodes: m.refs.map((id) => `module:${id}`),
        edges: [],
        annotations: [
          { ref: 'conversation', note: q },
          // 试用反馈#5：回答里的流程图（mermaid）随视图保存——视图成为"用户创建的图"资产，
          // 打开视图可直接看到流程图，而不只是定位已有模块
          ...(m.content.match(/```mermaid[\s\S]*?```/g) ?? []).map((content) => ({ type: 'mermaid', content, note: q })),
        ],
      }),
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        setSavedIdx(idx)
        setTimeout(() => setSavedIdx(null), 2500)
      })
      .catch(() => alert('存视图失败（需要本地后端在线）'))
  }

  return (
    <div className="flex h-full flex-col">
      <div className="mb-2 flex items-center justify-between">
        <span className="text-[10px] text-slate-400">
          会话已持久化 · 累计 {usage.promptTokens.toLocaleString()} / {usage.completionTokens.toLocaleString()} tokens
        </span>
        <div className="flex items-center gap-1">
          <button
            onClick={upgradeToTask}
            disabled={!backendRepo || sending || !messages.some((m) => m.role === 'user')}
            className="flex items-center gap-1 rounded-full border border-slate-200 bg-white px-2 py-0.5 text-[10px] text-slate-500 shadow-sm hover:bg-blue-50 hover:text-blue-600 disabled:opacity-40"
            title="把本轮对话（已澄清的需求与引用模块）组织成修复任务"
          >
            <Wrench size={10} />
            转为任务
          </button>
          <button
            onClick={compact}
            disabled={!backendRepo || compacting}
            className="flex items-center gap-1 rounded-full border border-slate-200 bg-white px-2 py-0.5 text-[10px] text-slate-500 shadow-sm hover:bg-slate-50 disabled:opacity-40"
            title="压缩上下文：早期历史折叠为结构化摘要（原文保留在库中可回放）"
          >
            <Shrink size={10} />
            {compacting ? '压缩中…' : '压缩上下文'}
          </button>
          <button
            onClick={reset}
            disabled={!backendRepo || sending}
            className="flex items-center gap-1 rounded-full border border-slate-200 bg-white px-2 py-0.5 text-[10px] text-slate-500 shadow-sm hover:bg-slate-50 disabled:opacity-40"
            title="新对话：清空本会话（消息、摘要、token 计数归零）"
          >
            <RotateCcw size={10} />
            新对话
          </button>
        </div>
      </div>

      <div ref={listRef} className="flex-1 space-y-3 overflow-y-auto">
        {messages.length === 0 && (
          <p className="pt-8 text-center text-[11.5px] leading-5 text-slate-400">
            基于语义代码地图提问：
            <br />
            "评测提交流程是谁负责的？"
            <br />
            "哪些模块耦合最重？"
            <br />
            <span className="text-[10px]">（回答可存为视图，引用模块可定位到画布）</span>
          </p>
        )}
        {messages.map((m, i) =>
          m.role === 'system' ? (
            <div key={i} className="flex justify-center">
              <span className="rounded-full bg-slate-100 px-2.5 py-0.5 text-[10px] text-slate-400">{m.content}</span>
            </div>
          ) : (
            <div key={i} className={`flex ${m.role === 'user' ? 'justify-end' : 'justify-start'}`}>
              <div
                className={`max-w-[92%] rounded-lg px-3 py-2 text-[11.5px] leading-5 ${
                  m.role === 'user' ? 'bg-blue-600 text-white' : 'border border-slate-200 bg-slate-50 text-slate-700'
                }`}
              >
                {m.images?.map((im, j) => (
                  <img key={j} src={im.dataUrl} alt={im.name} className="mb-1.5 max-h-40 rounded-lg" />
                ))}
                <MarkdownMessage content={m.content} />
                {m.role === 'assistant' && m.refs.length > 0 && (
                  <div className="mt-2 flex flex-wrap items-center gap-1 border-t border-slate-200 pt-2">
                    {m.refs.map((id) => (
                      <button
                        key={id}
                        onClick={() => onLocateModule(id)}
                        className="flex items-center gap-0.5 rounded-full bg-white px-2 py-0.5 font-mono text-[10px] text-blue-600 shadow-sm hover:bg-blue-50"
                        title="定位到画布"
                      >
                        <Crosshair size={9} />
                        {id}
                      </button>
                    ))}
                    <button
                      onClick={() => saveAsView(i)}
                      className="ml-auto flex items-center gap-1 rounded-full border border-emerald-200 bg-emerald-50 px-2 py-0.5 text-[10px] font-semibold text-emerald-700 hover:bg-emerald-100"
                      title="把本回答引用的实体存为可复用视图（.easyvibe/views/）"
                    >
                      {savedIdx === i ? <Check size={10} /> : <BookmarkPlus size={10} />}
                      {savedIdx === i ? '已存视图' : '存为视图'}
                    </button>
                  </div>
                )}
              </div>
            </div>
          ),
        )}
        {/* S1 grill-me 澄清卡：选择题形态，点选即回答 */}
        {clarify && (
          <div className="flex justify-start">
            <div className="max-w-[92%] rounded-lg border border-amber-200 bg-amber-50 px-3 py-2">
              <p className="text-[11px] font-semibold leading-4 text-amber-800">
                {clarify.question}
                {clarify.why && (
                  <span className="ml-1 font-normal text-amber-500" title={clarify.why}>
                    ⓘ
                  </span>
                )}
              </p>
              <div className="mt-1.5 space-y-1">
                {clarify.options.map((o) => (
                  <button
                    key={o.label}
                    onClick={() => answerClarify(o.label, o.desc)}
                    className="flex w-full items-center justify-between gap-2 rounded-md border border-amber-200 bg-white px-2 py-1 text-left text-[11px] text-slate-700 hover:border-blue-300 hover:bg-blue-50"
                  >
                    <span className="font-medium">{o.label}</span>
                    {o.desc && <span className="shrink-0 text-[9.5px] text-slate-400">{o.desc}</span>}
                  </button>
                ))}
              </div>
            </div>
          </div>
        )}
        {sending && (
          <div className="flex justify-start">
            <div className="flex items-center gap-1.5 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-[11px] text-slate-400">
              <Loader2 size={12} className="animate-spin" /> 正在查询地图…
            </div>
          </div>
        )}
      </div>

      {(attachments.length > 0 || images.length > 0) && (
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          {images.map((im, i) => (
            <span key={`img-${i}`} className="relative flex items-center gap-1 rounded-lg border border-slate-200 bg-white p-1">
              <img src={im.dataUrl} alt={im.name} className="h-8 w-8 rounded object-cover" />
              <span className="max-w-[90px] truncate text-[9.5px] text-slate-500">{im.name}</span>
              <button onClick={() => setImages((prev) => prev.filter((_, j) => j !== i))} className="text-slate-300 hover:text-red-500">
                <XIcon size={9} />
              </button>
            </span>
          ))}
          {attachments.map((a, i) => (
            <span key={i} className="flex items-center gap-1 rounded-full bg-blue-50 px-2 py-0.5 text-[10px] text-blue-700">
              <Paperclip size={9} />
              {a.name}（{(a.size / 1024).toFixed(0)}KB）
              <button onClick={() => setAttachments((prev) => prev.filter((_, j) => j !== i))} className="text-blue-300 hover:text-red-500">
                <XIcon size={9} />
              </button>
            </span>
          ))}
        </div>
      )}
      <div className="mt-3 flex items-end gap-2 border-t border-slate-100 pt-3">
        <input
          ref={fileRef}
          type="file"
          multiple
          className="hidden"
          onChange={(e) => addFiles(e.target.files)}
        />
        <button
          onClick={() => fileRef.current?.click()}
          disabled={!backendRepo || attachments.length >= 3}
          className="rounded-lg border border-slate-200 p-2.5 text-slate-400 transition-colors hover:text-blue-600 disabled:opacity-40"
          title="添加附件（代码/日志/文档文本，≤50KB×3）——内容随消息一起发给 AI"
        >
          <Paperclip size={14} />
        </button>
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault()
              send()
            }
          }}
          rows={2}
          placeholder={backendRepo ? '问点什么…（Enter 发送，Shift+Enter 换行）' : '需要本地后端在线'}
          disabled={!backendRepo || sending}
          className="flex-1 resize-none rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-[12px] leading-5 text-slate-700 outline-none focus:border-blue-300 disabled:opacity-50"
        />
        {sending ? (
          <button
            onClick={() => abortRef.current?.abort()}
            className="rounded-lg border border-red-200 p-2.5 text-red-600 transition-colors hover:bg-red-50"
            title="停止等待本次回答"
          >
            <Square size={13} />
          </button>
        ) : (
          <button
            onClick={send}
            disabled={!backendRepo || !input.trim()}
            className="rounded-lg bg-blue-600 p-2.5 text-white transition-colors hover:bg-blue-700 disabled:opacity-40"
          >
            <Send size={14} />
          </button>
        )}
      </div>
    </div>
  )
}
