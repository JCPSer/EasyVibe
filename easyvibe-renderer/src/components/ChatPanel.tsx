import { useRef, useState } from 'react'
import { Send, Loader2, BookmarkPlus, Check, Crosshair } from 'lucide-react'

interface ChatMessage {
  role: 'user' | 'assistant'
  content: string
  refs: string[]
}

interface Props {
  backendRepo: string | null
  onLocateModule: (moduleId: string) => void
}

// 入口对话（F2）：基于语义地图问答；回答可一键存为视图（F1b）
export function ChatPanel({ backendRepo, onLocateModule }: Props) {
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [input, setInput] = useState('')
  const [sending, setSending] = useState(false)
  const [savedIdx, setSavedIdx] = useState<number | null>(null)
  const listRef = useRef<HTMLDivElement>(null)

  const send = () => {
    const q = input.trim()
    if (!q || sending || !backendRepo) return
    setInput('')
    setSending(true)
    const history = messages.flatMap((m) => [
      { role: m.role, content: m.content },
    ])
    setMessages((prev) => [...prev, { role: 'user', content: q, refs: [] }])
    fetch(`/api/repos/${backendRepo}/chat`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: q, history }),
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.json()
      })
      .then((d: { data: { reply: string; refs: string[] } }) => {
        setMessages((prev) => [...prev, { role: 'assistant', content: d.data.reply, refs: d.data.refs }])
        setTimeout(() => listRef.current?.scrollTo({ top: listRef.current.scrollHeight }), 50)
      })
      .catch(() => {
        setMessages((prev) => [...prev, { role: 'assistant', content: '（对话服务不可用：需要本地后端在线）', refs: [] }])
      })
      .finally(() => setSending(false))
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
        {messages.map((m, i) => (
          <div key={i} className={`flex ${m.role === 'user' ? 'justify-end' : 'justify-start'}`}>
            <div
              className={`max-w-[92%] rounded-lg px-3 py-2 text-[11.5px] leading-5 ${
                m.role === 'user' ? 'bg-blue-600 text-white' : 'border border-slate-200 bg-slate-50 text-slate-700'
              }`}
            >
              <div className="whitespace-pre-wrap">{m.content}</div>
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
        ))}
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
