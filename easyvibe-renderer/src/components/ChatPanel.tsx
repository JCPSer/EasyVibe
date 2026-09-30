import { useEffect, useRef, useState } from 'react'
import { MarkdownMessage } from '@/components/MarkdownMessage'
import { Send, Loader2, BookmarkPlus, Check, Crosshair, Shrink, RotateCcw } from 'lucide-react'

interface ChatMessage {
  role: 'user' | 'assistant' | 'system'
  content: string
  refs: string[]
}

interface Props {
  backendRepo: string | null
  onLocateModule: (moduleId: string) => void
}

interface ChatRestore {
  summary: string | null
  messages: { id: number; role: string; content: string }[]
  usage: { promptTokens: number; completionTokens: number }
}

// 入口对话（F2 + M3-5 会话持久化）：服务端 SQLite 是会话事实源——
// 切换页签/刷新/后端重启均从库恢复（不再只活在前端 state）；支持手动压缩与 auto-compact 留痕
export function ChatPanel({ backendRepo, onLocateModule }: Props) {
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [usage, setUsage] = useState({ promptTokens: 0, completionTokens: 0 })
  const [input, setInput] = useState('')
  const [sending, setSending] = useState(false)
  const [savedIdx, setSavedIdx] = useState<number | null>(null)
  const [compacting, setCompacting] = useState(false)
  const listRef = useRef<HTMLDivElement>(null)

  // 恢复会话（M3-5：历史从域 2 读，完整原文含压缩留痕）
  useEffect(() => {
    if (!backendRepo) return
    setMessages([])
    fetch(`/api/repos/${backendRepo}/chat`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: ChatRestore }) => {
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
  }, [backendRepo])

  const send = () => {
    const q = input.trim()
    if (!q || sending || !backendRepo) return
    setInput('')
    setSending(true)
    // 服务端为事实源：只发当前消息，历史由后端从库装配（摘要+未压缩窗口）
    setMessages((prev) => [...prev, { role: 'user', content: q, refs: [] }])
    fetch(`/api/repos/${backendRepo}/chat`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: q }),
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.json()
      })
      .then((d: { data: { reply: string; refs: string[]; compaction: string | null; usage: { promptTokens: number; completionTokens: number } } }) => {
        setUsage(d.data.usage)
        setMessages((prev) => [
          ...prev,
          { role: 'assistant', content: d.data.reply, refs: d.data.refs },
          // auto-compact 留痕（§10a："上下文已压缩：82%→34%"系统消息）
          ...(d.data.compaction ? [{ role: 'system' as const, content: d.data.compaction, refs: [] }] : []),
        ])
        setTimeout(() => listRef.current?.scrollTo({ top: listRef.current.scrollHeight }), 50)
      })
      .catch(() => {
        setMessages((prev) => [...prev, { role: 'assistant', content: '（对话服务不可用：需要本地后端在线）', refs: [] }])
      })
      .finally(() => setSending(false))
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
        annotations: [{ ref: 'conversation', note: q }],
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
        {sending && (
          <div className="flex justify-start">
            <div className="flex items-center gap-1.5 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-[11px] text-slate-400">
              <Loader2 size={12} className="animate-spin" /> 正在查询地图…
            </div>
          </div>
        )}
      </div>

      <div className="mt-3 flex items-end gap-2 border-t border-slate-100 pt-3">
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
        <button
          onClick={send}
          disabled={!backendRepo || sending || !input.trim()}
          className="rounded-lg bg-blue-600 p-2.5 text-white transition-colors hover:bg-blue-700 disabled:opacity-40"
        >
          <Send size={14} />
        </button>
      </div>
    </div>
  )
}
