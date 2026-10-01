import { useEffect, useRef, useState } from 'react'
import { toast } from '@/lib/toast'
import { MarkdownMessage } from '@/components/MarkdownMessage'
import type { TaskDraft } from '@/lib/taskContext'
import type { CodeMap } from '@/types/map'
import { Send, Loader2, BookmarkPlus, Check, Crosshair, Shrink, RotateCcw, Wrench, Square, Paperclip, X as XIcon, Download, AtSign} from 'lucide-react'

interface ChatMessage {
  role: 'user' | 'assistant' | 'system'
  content: string
  refs: string[]
  /** 库消息 id（R1 分页游标） */
  id?: number
  /** 图片附件（dataURL；仅当前会话内存，刷新后不还原） */
  images?: { name: string; dataUrl: string }[]
  /** D9 @模块：本条用户消息显式钉住的模块 */
  mentions?: { id: string; name: string }[]
}

interface Clarify {
  question: string
  options: { label: string; desc?: string }[]
  why?: string
}

interface Props {
  backendRepo: string | null
  map: CodeMap | null
  onLocateModule: (moduleId: string) => void
  /** S1：对话升级任务入口——把本轮对话组织成 TaskDraft 交给任务表单 */
  onCreateTask: (draft: TaskDraft) => void
}

interface ChatRestore {
  summary: string | null
  messages: { id: number; role: string; content: string }[]
  hasMore: boolean
  usage: { promptTokens: number; completionTokens: number }
}

// 入口对话（F2 + M3-5 会话持久化）：服务端 SQLite 是会话事实源——
// 切换页签/刷新/后端重启均从库恢复（不再只活在前端 state）；支持手动压缩与 auto-compact 留痕
export function ChatPanel({ backendRepo, map, onLocateModule, onCreateTask }: Props) {
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [usage, setUsage] = useState({ promptTokens: 0, completionTokens: 0 })
  const [input, setInput] = useState('')
  const [sending, setSending] = useState(false)
  const [savedIdx, setSavedIdx] = useState<number | null>(null)
  const [namingIdx, setNamingIdx] = useState<number | null>(null)
  const [viewName, setViewName] = useState('')
  const [compacting, setCompacting] = useState(false)
  const [clarify, setClarify] = useState<Clarify | null>(null)
  // R1 清债：回放分页——首屏最近 50 条，"加载更早"再翻页（不再全表读）
  const [hasMore, setHasMore] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const abortRef = useRef<AbortController | null>(null)
  // 附件：文本类内容随消息注入（50KB×3）；图片走 images 通道（1.5MB×2，视觉模型消费）
  const [attachments, setAttachments] = useState<{ name: string; size: number; content: string }[]>([])
  const [images, setImages] = useState<{ name: string; size: number; dataUrl: string }[]>([])
  const fileRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  // D9 @模块：显式钉住的模块（随本条消息发送，后端拼成聚焦块前置给 LLM）
  const [mentions, setMentions] = useState<{ id: string; name: string }[]>([])
  // @ 触发建议浮层：query=@ 后的输入，start=@ 在输入框中的下标（用于选中后删除原文 token）
  const [suggest, setSuggest] = useState<{ query: string; start: number } | null>(null)
  const [suggestIdx, setSuggestIdx] = useState(0)

  const mentionCandidates = (query: string) => {
    if (!map) return []
    const q = query.toLowerCase()
    return map.modules
      .filter((m) => !mentions.some((x) => x.id === m.id))
      .filter((m) => !q || m.id.toLowerCase().includes(q) || m.name.toLowerCase().includes(q))
      .slice(0, 6)
  }

  // 输入变化：检测光标前最近的 @token（中间无空格才触发）
  const handleInputChange = (value: string, caret: number) => {
    setInput(value)
    const before = value.slice(0, caret)
    const m = before.match(/@([^\s@]{0,20})$/)
    if (m && map) {
      setSuggest({ query: m[1], start: caret - m[1].length - 1 })
      setSuggestIdx(0)
    } else {
      setSuggest(null)
    }
  }

  const pickMention = (id: string, name: string) => {
    if (!suggest) return
    const ta = textareaRef.current
    const caret = ta?.selectionStart ?? input.length
    // 删除已输入的 @token 原文，模块以芯片形式存在（不污染消息文本）
    const next = input.slice(0, suggest.start) + input.slice(caret)
    setInput(next)
    setMentions((prev) => [...prev, { id, name }])
    setSuggest(null)
    requestAnimationFrame(() => {
      ta?.focus()
      ta?.setSelectionRange(suggest.start, suggest.start)
    })
  }

  // R1 清债：加载更早一页（prepend 到消息头部）
  const loadEarlier = () => {
    if (!backendRepo || !messages.length || loadingMore) return
    setLoadingMore(true)
    fetch(`/api/repos/${backendRepo}/chat?before=${messages[0].id}&limit=50`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: ChatRestore }) => {
        setHasMore(d.data.hasMore)
        setMessages((prev) => [
          ...d.data.messages.map((m) => ({ id: m.id, role: m.role as ChatMessage['role'], content: m.content, refs: [] })),
          ...prev,
        ])
      })
      .catch(() => {})
      .finally(() => setLoadingMore(false))
  }

  // 恢复会话（M3-5：历史从域 2 读；R1 起分页——首屏最近一页）
  // stale 保护：快速切换仓库时，旧 fetch 返回不得覆盖新会话（审查 🟡2）
  useEffect(() => {
    if (!backendRepo) return
    let stale = false
    setMessages([])
    fetch(`/api/repos/${backendRepo}/chat?limit=50`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: ChatRestore }) => {
        if (stale) return
        setUsage(d.data.usage)
        setHasMore(d.data.hasMore)
        setMessages(
          d.data.messages.map((m) => ({
            id: m.id,
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
      { role: 'user', content: withAttach, refs: [], images: images.map((i) => ({ name: i.name, dataUrl: i.dataUrl })), mentions },
    ])
    setAttachments([])
    const sendImages = images.map((i) => i.dataUrl)
    setImages([])
    const sendMentions = mentions.map((m) => m.id)
    setMentions([])
    setSuggest(null)
    abortRef.current?.abort()
    const ac = new AbortController()
    abortRef.current = ac
    fetch(`/api/repos/${backendRepo}/chat`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: withAttach, images: sendImages, moduleRefs: sendMentions }),
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
      .catch(() => toast('压缩失败（需要本地后端在线）', 'error'))
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
      .catch(() => toast('重置失败（需要本地后端在线）', 'error'))
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

  // D2 拍板：导出并清空——留痕承诺不自动清，由用户显式"导出归档 → 清空"
  const exportAndClear = () => {
    exportChat()
    setTimeout(() => {
      if (!window.confirm('已导出。清空当前对话？（清空后不可恢复，视图文件不受影响）')) return
      fetch(`/api/repos/${backendRepo}/chat/reset`, { method: 'POST' })
        .then((r) => {
          if (!r.ok) throw new Error(String(r.status))
          setMessages([])
          setUsage({ promptTokens: 0, completionTokens: 0 })
          toast('对话已清空（归档文件已下载）')
        })
        .catch(() => toast('清空失败（需要本地后端在线）', 'error'))
    }, 400)
  }

  // 导出对话为 Markdown（§12d 兑现之一：对话含 mermaid 图，可直接评审/沉淀）
  const exportChat = () => {
    const md = [
      `# EasyVibe 对话导出 · ${backendRepo ?? ''}`,
      `> ${new Date().toLocaleString()}`,
      '',
      ...messages.flatMap((m) => {
        if (m.role === 'system') return [`> ${m.content}`, '']
        if (m.role === 'user') return [`## 问`, '', m.content, '']
        return [`## 答`, '', m.content, '']
      }),
    ].join('\n')
    const blob = new Blob([md], { type: 'text/markdown;charset=utf-8' })
    const a = document.createElement('a')
    a.href = URL.createObjectURL(blob)
    a.download = `easyvibe-chat-${backendRepo ?? 'export'}-${Date.now()}.md`
    a.click()
    URL.revokeObjectURL(a.href)
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
    const name = (viewName.trim() || q).slice(0, 40)
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
        setNamingIdx(null)
        setTimeout(() => setSavedIdx(null), 2500)
      })
      .catch(() => toast('存视图失败（需要本地后端在线）', 'error'))
  }

  return (
    <div className="flex h-full flex-col">
      <div className="mb-2 flex items-center justify-between">
        <span className="text-[10px] text-slate-400">
          会话已持久化 · 累计 {usage.promptTokens.toLocaleString()} / {usage.completionTokens.toLocaleString()} tokens
        </span>
        <div className="flex flex-wrap items-center justify-end gap-1">
          <button
            onClick={exportAndClear}
            disabled={!backendRepo || messages.length === 0}
            className="flex items-center gap-1 rounded-full border border-slate-200 bg-white px-2 py-0.5 text-[10px] text-slate-500 shadow-sm hover:bg-slate-50 disabled:opacity-40"
            title="导出为 Markdown 后可清空会话（D2 拍板：留痕照旧，清理显式）"
          >
            <Download size={10} />
            导出/清空
          </button>
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
        {hasMore && (
          <div className="flex justify-center">
            <button
              onClick={loadEarlier}
              disabled={loadingMore}
              className="rounded-full border border-slate-200 bg-white px-3 py-1 text-[10px] text-slate-500 shadow-sm hover:bg-slate-50 disabled:opacity-40"
            >
              {loadingMore ? '加载中…' : '↑ 加载更早的消息'}
            </button>
          </div>
        )}
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
                {m.mentions && m.mentions.length > 0 && (
                  <div className="mb-1 flex flex-wrap gap-1">
                    {m.mentions.map((mm) => (
                      <span key={mm.id} className="flex items-center gap-0.5 rounded-full bg-white/20 px-1.5 py-0.5 text-[9.5px]">
                        <AtSign size={8} />
                        {mm.name}
                      </span>
                    ))}
                  </div>
                )}
                <MarkdownMessage content={m.content} />
                {(m.role === 'assistant' || m.role === 'user') && (m.refs.length > 0 || /```mermaid/.test(m.content)) && (
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
                    {namingIdx === i ? (
                      /* 深挖#C：存图命名——多张视图靠问题截断无法区分，存前给命名框 */
                      <span className="ml-auto flex items-center gap-1">
                        <input
                          autoFocus
                          value={viewName}
                          onChange={(e) => setViewName(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') saveAsView(i)
                            if (e.key === 'Escape') setNamingIdx(null)
                          }}
                          placeholder="视图名…"
                          className="w-36 rounded-full border border-emerald-300 bg-white px-2 py-0.5 text-[10.5px] outline-none"
                        />
                        <button
                          onClick={() => saveAsView(i)}
                          disabled={!viewName.trim()}
                          className="rounded-full bg-emerald-600 px-2 py-0.5 text-[10px] font-bold text-white disabled:opacity-40"
                        >
                          存
                        </button>
                      </span>
                    ) : (
                      <button
                        onClick={() => {
                          const q = messages.slice(0, i).reverse().find((x) => x.role === 'user')?.content ?? '对话视图'
                          setViewName(q.replace(/\s+/g, ' ').slice(0, 24))
                          setNamingIdx(i)
                        }}
                        className="ml-auto flex items-center gap-1 rounded-full border border-emerald-200 bg-emerald-50 px-2 py-0.5 text-[10px] font-semibold text-emerald-700 hover:bg-emerald-100"
                        title="把本回答（含流程图）存为可复用视图（.easyvibe/views/）"
                      >
                        {savedIdx === i ? <Check size={10} /> : <BookmarkPlus size={10} />}
                        {savedIdx === i ? '已存视图' : '存为视图'}
                      </button>
                    )}
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
                  /* 改进#3：选项纵排（横排挤压曾把"仅桌面端"断成 4 行）——标签一行、理由一行 */
                  <button
                    key={o.label}
                    onClick={() => answerClarify(o.label, o.desc)}
                    className="flex w-full flex-col items-start gap-0.5 rounded-md border border-amber-200 bg-white px-2 py-1.5 text-left hover:border-blue-300 hover:bg-blue-50"
                  >
                    <span className="text-[11px] font-medium leading-4 text-slate-700">{o.label}</span>
                    {o.desc && <span className="text-[9.5px] leading-4 text-slate-400">{o.desc}</span>}
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

      {mentions.length > 0 && (
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          {mentions.map((m) => (
            <span key={m.id} className="flex items-center gap-1 rounded-full bg-blue-600 px-2 py-0.5 text-[10px] text-white">
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
        <div className="relative flex-1">
          {suggest && mentionCandidates(suggest.query).length > 0 && (
            /* D9 @模块建议浮层：@ 后输入即过滤，↑↓ 选择，Enter/Tab 选中，Esc 关闭 */
            <div className="absolute bottom-full left-0 z-20 mb-1 w-64 overflow-hidden rounded-lg border border-slate-200 bg-white shadow-lg">
              {mentionCandidates(suggest.query).map((m, i) => (
                <button
                  key={m.id}
                  onMouseDown={(e) => {
                    e.preventDefault()
                    pickMention(m.id, m.name)
                  }}
                  onMouseEnter={() => setSuggestIdx(i)}
                  className={`flex w-full items-center gap-2 px-2.5 py-1.5 text-left ${i === suggestIdx ? 'bg-blue-50' : 'bg-white'}`}
                >
                  <AtSign size={10} className="shrink-0 text-blue-400" />
                  <span className="truncate text-[11.5px] font-medium text-slate-700">{m.name}</span>
                  <span className="ml-auto shrink-0 font-mono text-[9.5px] text-slate-400">{m.id}</span>
                </button>
              ))}
            </div>
          )}
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
            placeholder={backendRepo ? '问点什么…（@ 引用模块，Enter 发送，Shift+Enter 换行）' : '需要本地后端在线'}
            disabled={!backendRepo || sending}
            className="w-full flex-1 resize-none rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-[12px] leading-5 text-slate-700 outline-none focus:border-blue-300 disabled:opacity-50"
          />
        </div>
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
