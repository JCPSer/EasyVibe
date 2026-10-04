import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { toast } from '@/lib/toast'
import { MarkdownMessage } from '@/components/MarkdownMessage'
import { AnswerCards } from '@/components/AnswerCards'
import type { TaskDraft } from '@/lib/taskContext'
import type { CodeMap } from '@/types/map'
import { onTaskEvent } from '@/lib/growthBus'
import { ONBOARDING_COPY } from '@/lib/onboardingCopy'
import { Send, Loader2, BookmarkPlus, Check, Crosshair, Shrink, RotateCcw, Wrench, Square, Paperclip, X as XIcon, Download, AtSign, ChevronDown, Plus, Pencil, CheckCircle2, AlertTriangle, Copy} from 'lucide-react'

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

// M4-2 多会话：AionUI 运行时摘要精简版（后端 conversation_summary 产出）
export interface ConversationSummary {
  id: string
  title: string | null
  repo: string
  updatedAt: string
  messageCount: number
  usage: { promptTokens: number; completionTokens: number }
  runtime: { state: 'idle' | 'running' | 'waiting_confirmation'; pendingConfirmations: number; runningTasks: number }
}

interface PendingApproval {
  taskId: string
  title: string
  gate: string | null
}

const GATE_LABEL: Record<string, string> = { plan: '① 计划审批', diff: '② Diff 审批', report: '③ 审查报告' }

interface Props {
  backendRepo: string | null
  map: CodeMap | null
  onLocateModule: (moduleId: string) => void
  /** S1：对话升级任务入口——把本轮对话组织成 TaskDraft 交给任务表单 */
  onCreateTask: (draft: TaskDraft) => void
  /** M4-2 工作台嵌入模式：隐藏会话切换器（左侧栏自带）与部分头部按钮 */
  embedded?: boolean
  /** M4-2：当前会话变化通知（工作台据此加载该会话的任务/影响面） */
  onConvChange?: (id: string | null) => void
  /** R3 B1：受控会话 id——传入后组件进入受控模式（工作台左栏驱动中栏联动），
   * 内部切换只经 onConvChange 上报父级；不传则维持内部自治 */
  activeConvId?: string | null
  /** v0.2 跨页预填：携带目标会话 + nonce，会话匹配且未消费时写入输入框/mentions。
   * 契约如此而非"挂载时写一次"的原因：本组件常驻挂载、切会话不重挂载（曾致跨会话串话） */
  pendingDraft?: { convId: string; text: string; mention?: { id: string; name: string }; nonce: number } | null
}

interface ChatRestore {
  summary: string | null
  conversation?: { id: string; title: string | null }
  messages: { id: number; role: string; content: string }[]
  hasMore: boolean
  pendingApprovals?: PendingApproval[]
  usage: { promptTokens: number; completionTokens: number }
}

// 入口对话（F2 + M3-5 会话持久化 + M4-2 多会话）：服务端 SQLite 是会话事实源——
// 多会话（每仓库 N 个，会话=任务的上位容器）+ 内联审批卡（AionUI 模式）+
// 切换页签/刷新/后端重启均从库恢复（不再只活在前端 state）；支持手动压缩与 auto-compact 留痕
export function ChatPanel({ backendRepo, map, onLocateModule, onCreateTask, embedded, onConvChange, activeConvId, pendingDraft }: Props) {
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

  // ---- M4-2 多会话状态 ----
  const [convs, setConvs] = useState<ConversationSummary[]>([])
  // R3 B1：受控/非受控双模式。受控（activeConvId 传入）时内部态不生效，
  // 一切切换经 setConvId → onConvChange 上报父级（工作台左栏与中栏联动的契约）
  const [internalConvId, setInternalConvId] = useState<string | null>(null) // null = 后端回落（最近活跃）
  const convId = activeConvId !== undefined ? activeConvId : internalConvId
  const setConvId = useCallback(
    (id: string | null) => {
      if (activeConvId === undefined) setInternalConvId(id)
      onConvChange?.(id)
    },
    [activeConvId, onConvChange],
  )
  const [convMenuOpen, setConvMenuOpen] = useState(false)
  const [renaming, setRenaming] = useState(false)
  const [renameVal, setRenameVal] = useState('')
  const [pendingApprovals, setPendingApprovals] = useState<PendingApproval[]>([])
  const [decided, setDecided] = useState<Record<string, 'approved' | 'rejected'>>({})

  const convQ = convId ? `?conv=${encodeURIComponent(convId)}` : ''
  const convBody = convId ? { conv: convId } : {}
  // R3 #5：会话归属守卫——发送后立刻切会话，回复不得追加进新会话的消息列表（串台污染）
  const convIdRef = useRef<string | null>(convId)
  useEffect(() => {
    convIdRef.current = convId
  }, [convId])

  const loadConvs = useCallback(() => {
    if (!backendRepo) return
    fetch(`/api/repos/${backendRepo}/conversations`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: ConversationSummary[] } | null) => {
        if (d?.data) setConvs(d.data)
      })
      .catch(() => {})
  }, [backendRepo])

  const refreshPending = useCallback(() => {
    if (!backendRepo) return
    fetch(`/api/repos/${backendRepo}/chat${convId ? `?conv=${encodeURIComponent(convId)}&limit=1` : '?limit=1'}`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: ChatRestore } | null) => {
        if (d?.data?.pendingApprovals) setPendingApprovals(d.data.pendingApprovals)
      })
      .catch(() => {})
  }, [backendRepo, convId])

  // 会话列表 + 审批数随任务事件实时刷新（AionUI：页签上永远看得到"谁在等你批准"）
  useEffect(() => onTaskEvent(() => { loadConvs(); refreshPending() }), [loadConvs, refreshPending])

  const createConv = useCallback(() => {
    if (!backendRepo) return
    fetch(`/api/repos/${backendRepo}/conversations`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({}),
    })
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: ConversationSummary }) => {
        setConvId(d.data.id)
        setConvMenuOpen(false)
        loadConvs()
      })
      .catch(() => toast('新建会话失败', 'error'))
  }, [backendRepo, loadConvs, setConvId])

  const switchConv = useCallback((id: string | null) => {
    setConvId(id)
    setConvMenuOpen(false)
    setClarify(null)
    setPendingApprovals([])
    setDecided({})
    setRejectingApproval(null)
    setRejectNote('')
    // v0.2 同批修复：切会话清空输入框与 @提及——消除"上一会话的半成品文本串到下一会话"
    setInput('')
    setMentions([])
    setSuggest(null)
  }, [setConvId])

  // v0.2 跨页预填消费：目标会话匹配且 nonce 未消费才写入（防跨会话串话；消费即弃）
  const consumedDrafts = useRef<Set<number>>(new Set())
  useEffect(() => {
    if (!pendingDraft || !convId) return
    if (pendingDraft.convId !== convId || consumedDrafts.current.has(pendingDraft.nonce)) return
    consumedDrafts.current.add(pendingDraft.nonce)
    setInput(pendingDraft.text)
    setMentions(pendingDraft.mention ? [pendingDraft.mention] : [])
    requestAnimationFrame(() => textareaRef.current?.focus())
  }, [pendingDraft, convId])

  const renameConv = useCallback(() => {
    if (!backendRepo || !convId || !renameVal.trim()) return
    fetch(`/api/repos/${backendRepo}/conversations/${encodeURIComponent(convId)}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ title: renameVal.trim() }),
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        setRenaming(false)
        loadConvs()
      })
      .catch(() => toast('重命名失败', 'error'))
  }, [backendRepo, convId, renameVal, loadConvs])

  const deleteConv = useCallback(() => {
    if (!backendRepo || !convId) return
    if (!window.confirm('删除该会话？（其消息与关联任务留痕一并删除，不可恢复）')) return
    fetch(`/api/repos/${backendRepo}/conversations/${encodeURIComponent(convId)}`, { method: 'DELETE' })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        setConvId(null)
        loadConvs()
      })
      .catch((e) => toast(String(e).includes('400') ? '每个仓库至少保留一个会话' : '删除失败', 'error'))
  }, [backendRepo, convId, loadConvs])

  // 内联审批：选项即按钮，提交后原地变"✓ 已回复"（AionUI PermissionRequestPanel 模式）
  // R3 C4：驳回必须带理由（后端 400 强制）——卡内展开理由输入，失败 toast 带后端归因
  const [rejectingApproval, setRejectingApproval] = useState<PendingApproval | null>(null)
  const [rejectNote, setRejectNote] = useState('')
  const decide = useCallback(
    (p: PendingApproval, decision: 'approved' | 'rejected', note?: string) => {
      if (!backendRepo) return
      fetch(`/api/repos/${backendRepo}/tasks/${encodeURIComponent(p.taskId)}/decide`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ decision, note }),
      })
        .then((r) =>
          r.ok ? r.json() : r.json().then((e: { error?: string }) => Promise.reject(new Error(e?.error ?? `HTTP ${r.status}`))),
        )
        .then(() => {
          setDecided((prev) => ({ ...prev, [p.taskId + ':' + (p.gate ?? '')]: decision }))
          setPendingApprovals((prev) => prev.filter((x) => x.taskId !== p.taskId))
          setRejectingApproval(null)
          setRejectNote('')
          loadConvs()
        })
        .catch((e) => toast(`审批操作失败：${String(e).replace(/^Error:\s*/, '').slice(0, 60)}`, 'error'))
    },
    [backendRepo, loadConvs],
  )

  // R1 清债：加载更早一页（prepend 到消息头部）
  const loadEarlier = () => {
    if (!backendRepo || !messages.length || loadingMore) return
    setLoadingMore(true)
    fetch(`/api/repos/${backendRepo}/chat${convQ ? convQ + '&' : '?'}before=${messages[0].id}&limit=50`)
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

  // 恢复会话（历史从域 2 读；R1 起分页——首屏最近一页）
  // stale 保护：快速切换仓库/会话时，旧 fetch 返回不得覆盖新会话
  useEffect(() => {
    if (!backendRepo) return
    let stale = false
    setMessages([])
    loadConvs()
    fetch(`/api/repos/${backendRepo}/chat${convQ ? convQ + '&' : '?'}limit=50`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: ChatRestore }) => {
        if (stale) return
        setUsage(d.data.usage)
        setHasMore(d.data.hasMore)
        setPendingApprovals(d.data.pendingApprovals ?? [])
        setDecided({})
        if (d.data.conversation?.id) {
          setConvId(d.data.conversation.id)
        }
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
  }, [backendRepo, convId])

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
    const sentConvId = convId // 归属锚点：响应到达时比对当前会话
    fetch(`/api/repos/${backendRepo}/chat`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: withAttach, images: sendImages, moduleRefs: sendMentions, ...convBody }),
      signal: ac.signal,
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.json()
      })
      .then((d: { data: { reply: string; refs: string[]; compaction: string | null; clarify: Clarify | null; usage: { promptTokens: number; completionTokens: number } } }) => {
        if (convIdRef.current !== sentConvId) {
          // 会话已切换：回复属于旧会话（服务端已落库），不污染当前列表
          loadConvs()
          return
        }
        setUsage(d.data.usage)
        setClarify(d.data.clarify)
        setMessages((prev) => [
          ...prev,
          { role: 'assistant', content: d.data.reply.trim() || '（未获得回答——请重试）', refs: d.data.refs },
          // auto-compact 留痕（§10a："上下文已压缩：82%→34%"系统消息）
          ...(d.data.compaction ? [{ role: 'system' as const, content: d.data.compaction, refs: [] }] : []),
        ])
        setTimeout(() => listRef.current?.scrollTo({ top: listRef.current.scrollHeight }), 50)
        loadConvs()
      })
      .catch((e) => {
        if (convIdRef.current !== sentConvId) return // 会话已切换：错误也不得追加进新会话
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
    fetch(`/api/repos/${backendRepo}/chat/compact${convQ}`, { method: 'POST' })
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

  // 清空当前会话（消息/摘要/计数归零，会话行保留）
  const reset = () => {
    if (!backendRepo || sending) return
    fetch(`/api/repos/${backendRepo}/chat/reset${convQ}`, { method: 'POST' })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        setMessages([])
        setUsage({ promptTokens: 0, completionTokens: 0 })
        setPendingApprovals([])
        setDecided({})
        loadConvs()
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
      if (!window.confirm('已导出。清空当前会话？（清空后不可恢复，视图文件不受影响）')) return
      reset()
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
      conversation_id: convId ?? undefined,
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
      .then(async (r) => {
        if (r.status === 409) {
          // 审计 P1：同名冲突改为用户裁决——确认后带 force 覆盖（此前后端静默另存后缀，数据去向不明）
          if (window.confirm(`已存在同名视图「${name}」。覆盖它？（取消则放弃保存）`)) {
            const again = await fetch(`/api/repos/${backendRepo}/views?force=true`, {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({
                name,
                nodes: m.refs.map((id) => `module:${id}`),
                edges: [],
                annotations: [
                  { ref: 'conversation', note: q },
                  ...(m.content.match(/```mermaid[\s\S]*?```/g) ?? []).map((content) => ({ type: 'mermaid', content, note: q })),
                ],
              }),
            })
            if (!again.ok) throw new Error(String(again.status))
          } else {
            return
          }
        } else if (!r.ok) {
          throw new Error(String(r.status))
        }
        setSavedIdx(idx)
        setNamingIdx(null)
        setTimeout(() => setSavedIdx(null), 2500)
      })
      .catch(() => toast('存视图失败（需要本地后端在线）', 'error'))
  }

  const currentConv = convs.find((c) => c.id === convId)
  const displayTitle = currentConv?.title ?? (convId ? '会话' : '默认会话')
  // 示例 prompt：模块感知（首个模块入句）+ 通用两条；仅在空对话渲染
  const samplePrompts = useMemo(() => {
    const sp = ONBOARDING_COPY.samplePrompts
    const first = map?.modules[0]
    return [
      ...(first ? [sp.withModule.replace('{module}', first.name)] : []),
      ...sp.generic,
    ]
  }, [map])

  return (
    <div className="flex h-full flex-col">
      {/* M4-2 会话切换器（AionUI 模式：三态行 + 待审批角标；embedded 模式由工作台左栏承担） */}
      {!embedded && (
        <div className="relative mb-2">
          <button
            onClick={() => setConvMenuOpen((v) => !v)}
            className="flex w-full items-center gap-2 rounded-lg border border-slate-200 bg-white px-2.5 py-1.5 text-left hover:border-blue-300"
          >
            {currentConv?.runtime.state === 'running' ? (
              <Loader2 size={12} className="shrink-0 animate-spin text-amber-500" />
            ) : currentConv && currentConv.runtime.pendingConfirmations > 0 ? (
              <AlertTriangle size={12} className="shrink-0 text-amber-500" />
            ) : (
              <span className="h-2 w-2 shrink-0 rounded-full bg-slate-300" />
            )}
            <span className="min-w-0 flex-1 truncate text-[12px] font-semibold text-slate-700">{displayTitle}</span>
            {currentConv && currentConv.runtime.pendingConfirmations > 0 && (
              <span className="rounded-full bg-red-500 px-1.5 text-micro font-bold leading-4 text-white">
                {currentConv.runtime.pendingConfirmations}
              </span>
            )}
            <ChevronDown size={12} className="shrink-0 text-slate-400" />
          </button>
          {convMenuOpen && (
            <>
              <div className="fixed inset-0 z-30" onClick={() => setConvMenuOpen(false)} />
              <div className="glass absolute left-0 right-0 top-full z-40 mt-1 max-h-72 overflow-y-auto rounded-xl border border-slate-200 p-1.5 shadow-xl anim-scale-in">
                {convs.map((c) => (
                  <div key={c.id} className="flex items-center gap-1.5 rounded-lg px-2 py-1.5 hover:bg-slate-50">
                    <button
                      className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
                      onClick={() => switchConv(c.id === convId ? null : c.id)}
                    >
                      {c.runtime.state === 'running' ? (
                        <Loader2 size={11} className="shrink-0 animate-spin text-amber-500" />
                      ) : c.runtime.pendingConfirmations > 0 ? (
                        <AlertTriangle size={11} className="shrink-0 text-amber-500" />
                      ) : (
                        <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-slate-300" />
                      )}
                      <span className={`min-w-0 flex-1 truncate text-[12px] ${c.id === convId ? 'font-bold text-blue-700' : 'text-slate-600'}`}>
                        {c.title ?? '未命名会话'}
                      </span>
                      {c.runtime.pendingConfirmations > 0 && (
                        <span className="rounded-full bg-red-500 px-1.5 text-micro font-bold leading-4 text-white">{c.runtime.pendingConfirmations}</span>
                      )}
                      <span className="tnum shrink-0 text-micro text-slate-300">{c.messageCount}条</span>
                    </button>
                    {c.id === convId && (
                      <button onClick={() => { setRenaming(true); setRenameVal(c.title ?? '') }} className="shrink-0 rounded p-0.5 text-slate-300 hover:text-blue-500" title="重命名">
                        <Pencil size={10} />
                      </button>
                    )}
                  </div>
                ))}
                {convs.length === 0 && <p className="px-2 py-1.5 text-[11px] text-slate-400">还没有会话，发第一条消息即创建</p>}
                <button
                  onClick={createConv}
                  className="mt-1 flex w-full items-center justify-center gap-1 rounded-lg bg-blue-600 px-2 py-1.5 text-[11px] font-semibold text-white hover:bg-blue-700"
                >
                  <Plus size={11} /> 新建会话
                </button>
              </div>
            </>
          )}
          {renaming && (
            <div className="absolute left-0 right-0 top-full z-50 mt-1 flex items-center gap-1 rounded-xl border border-slate-200 bg-white p-2 shadow-xl">
              <input
                autoFocus
                value={renameVal}
                onChange={(e) => setRenameVal(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') renameConv()
                  if (e.key === 'Escape') setRenaming(false)
                }}
                placeholder="会话名…"
                className="flex-1 rounded-lg border border-slate-200 px-2 py-1 text-[12px] outline-none focus:border-blue-300"
              />
              <button onClick={renameConv} className="rounded-lg bg-blue-600 px-2.5 py-1 text-[11px] font-bold text-white">存</button>
            </div>
          )}
        </div>
      )}

      <div className="mb-2 flex items-center justify-between">
        <span className="tnum text-micro text-slate-400">
          会话已持久化 · 累计 {usage.promptTokens.toLocaleString()} / {usage.completionTokens.toLocaleString()} tokens
        </span>
        <div className="flex flex-wrap items-center justify-end gap-1">
          {convId && !embedded && (
            <button
              onClick={deleteConv}
              disabled={!backendRepo}
              className="flex items-center gap-1 rounded-full border border-slate-200 bg-white px-2 py-0.5 text-micro text-slate-500 shadow-sm hover:bg-red-50 hover:text-red-600 disabled:opacity-40"
              title="删除当前会话"
            >
              <XIcon size={10} />
              删会话
            </button>
          )}
          <button
            onClick={exportAndClear}
            disabled={!backendRepo || messages.length === 0}
            className="flex items-center gap-1 rounded-full border border-slate-200 bg-white px-2 py-0.5 text-micro text-slate-500 shadow-sm hover:bg-slate-50 disabled:opacity-40"
            title="导出为 Markdown 后可清空会话（D2 拍板：留痕照旧，清理显式）"
          >
            <Download size={10} />
            导出/清空
          </button>
          <button
            onClick={upgradeToTask}
            disabled={!backendRepo || sending || !messages.some((m) => m.role === 'user')}
            className="flex items-center gap-1 rounded-full border border-slate-200 bg-white px-2 py-0.5 text-micro text-slate-500 shadow-sm hover:bg-blue-50 hover:text-blue-600 disabled:opacity-40"
            title="把本轮对话（已澄清的需求与引用模块）组织成修复任务"
          >
            <Wrench size={10} />
            转为任务
          </button>
          <button
            onClick={compact}
            disabled={!backendRepo || compacting}
            className="flex items-center gap-1 rounded-full border border-slate-200 bg-white px-2 py-0.5 text-micro text-slate-500 shadow-sm hover:bg-slate-50 disabled:opacity-40"
            title="压缩上下文：早期历史折叠为结构化摘要（原文保留在库中可回放）"
          >
            <Shrink size={10} />
            {compacting ? '压缩中…' : '压缩上下文'}
          </button>
          {/* v0.2：embedded 模式隐藏"新会话"——会话创建统一走工作台左栏（列表即管理），
              消除"预填/新会话落在哪"的双入口解释成本 */}
          {!embedded && (
            <button
              onClick={createConv}
              disabled={!backendRepo || sending}
              className="flex items-center gap-1 rounded-full border border-slate-200 bg-white px-2 py-0.5 text-micro text-slate-500 shadow-sm hover:bg-slate-50 disabled:opacity-40"
              title="新会话：开一个全新的对话容器（旧会话保留在列表中）"
            >
              <RotateCcw size={10} />
              新会话
            </button>
          )}
        </div>
      </div>

      <div ref={listRef} className="flex-1 space-y-3 overflow-y-auto">
        {/* M4-2 内联审批卡：等待中的审批门（AionUI：选项即按钮，决策后原地留痕） */}
        {pendingApprovals.map((p) => (
          <div key={p.taskId + ':' + (p.gate ?? '')} className="anim-msg-in flex justify-start">
            <div className="max-w-[92%] rounded-lg border border-amber-200 bg-amber-50 px-3 py-2">
              <p className="flex items-center gap-1.5 text-[11px] font-semibold leading-4 text-amber-800">
                <AlertTriangle size={11} />
                审批请求 <span className="rounded-full bg-white/80 px-1.5 text-micro font-bold text-amber-600">{GATE_LABEL[p.gate ?? ''] ?? p.gate ?? '审批'}</span>
              </p>
              <p className="mt-1 text-[12px] leading-5 text-slate-700">{p.title}</p>
              {decided[p.taskId + ':' + (p.gate ?? '')] ? (
                <p className="mt-1.5 flex items-center gap-1 text-cap font-semibold text-emerald-600">
                  <CheckCircle2 size={11} /> 已{decided[p.taskId + ':' + (p.gate ?? '')] === 'approved' ? '通过' : '驳回'}
                </p>
              ) : rejectingApproval?.taskId === p.taskId && rejectingApproval.gate === p.gate ? (
                <div className="mt-1.5 space-y-1.5">
                  <textarea
                    value={rejectNote}
                    onChange={(e) => setRejectNote(e.target.value)}
                    rows={2}
                    placeholder="驳回理由（必填，留痕可追溯）"
                    className="w-full resize-none rounded-lg border border-red-200 bg-white px-2 py-1.5 text-[11px] leading-4 text-slate-700 outline-none focus:border-red-400"
                  />
                  <div className="flex gap-2">
                    <button
                      onClick={() => decide(p, 'rejected', rejectNote.trim())}
                      disabled={!rejectNote.trim()}
                      className="flex-1 rounded-lg bg-red-600 px-3 py-1.5 text-[11px] font-bold text-white hover:bg-red-700 disabled:opacity-40"
                    >
                      确认驳回
                    </button>
                    <button
                      onClick={() => {
                        setRejectingApproval(null)
                        setRejectNote('')
                      }}
                      className="flex-1 rounded-lg border border-slate-200 bg-white px-3 py-1.5 text-[11px] font-bold text-slate-500 hover:bg-slate-50"
                    >
                      取消
                    </button>
                  </div>
                </div>
              ) : (
                <div className="mt-1.5 flex gap-2">
                  <button
                    onClick={() => decide(p, 'approved')}
                    className="flex-1 rounded-lg bg-blue-600 px-3 py-1.5 text-[11px] font-bold text-white hover:bg-blue-700"
                  >
                    通过
                  </button>
                  <button
                    onClick={() => {
                      setRejectingApproval(p)
                      setRejectNote('')
                    }}
                    className="flex-1 rounded-lg border border-red-200 bg-white px-3 py-1.5 text-[11px] font-bold text-red-600 hover:bg-red-50"
                  >
                    驳回
                  </button>
                </div>
              )}
            </div>
          </div>
        ))}
        {hasMore && (
          <div className="flex justify-center">
            <button
              onClick={loadEarlier}
              disabled={loadingMore}
              className="rounded-full border border-slate-200 bg-white px-3 py-1 text-micro text-slate-500 shadow-sm hover:bg-slate-50 disabled:opacity-40"
            >
              {loadingMore ? '加载中…' : '↑ 加载更早的消息'}
            </button>
          </div>
        )}
        {messages.length === 0 && pendingApprovals.length === 0 && (
          <p className="pt-8 text-center text-[12px] leading-5 text-slate-400">
            基于语义代码地图提问：
            <br />
            "评测提交流程是谁负责的？"
            <br />
            "哪些模块耦合最重？"
            <br />
            <span className="text-micro">（回答可存为视图，引用模块可定位到画布）</span>
          </p>
        )}
        {messages.map((m, i) =>
          m.role === 'system' ? (
            <div key={i} className="anim-msg-in flex justify-center">
              <span className="rounded-full bg-slate-100 px-2.5 py-0.5 text-micro text-slate-400">{m.content}</span>
            </div>
          ) : (
            <div key={i} className={`anim-msg-in group/msg flex ${m.role === 'user' ? 'justify-end' : 'justify-start'}`}>
              <div
                className={`max-w-[92%] rounded-lg px-3 py-2 text-[12px] leading-5 ${
                  m.role === 'user' ? 'bg-blue-600 text-white' : 'border border-slate-200 bg-slate-50 text-slate-700'
                }`}
              >
                {m.images?.map((im, j) => (
                  <img key={j} src={im.dataUrl} alt={im.name} className="mb-1.5 max-h-40 rounded-lg" />
                ))}
                {m.mentions && m.mentions.length > 0 && (
                  <div className="mb-1 flex flex-wrap gap-1">
                    {m.mentions.map((mm) => (
                      <span key={mm.id} className="flex items-center gap-0.5 rounded-full bg-white/20 px-1.5 py-0.5 text-micro">
                        <AtSign size={8} />
                        {mm.name}
                      </span>
                    ))}
                  </div>
                )}
                {m.role === 'assistant' && !m.content.trim() ? (
                  <span className="text-slate-400">（此条未获得回答）</span>
                ) : m.role === 'assistant' ? (
                  /* M4-2 答案卡片三型（对话面板原型）：有章节结构的回答拆卡渲染 */
                  <AnswerCards content={m.content} />
                ) : (
                  <MarkdownMessage content={m.content} />
                )}
                {/* 审计 P2：消息单条复制（此前只能导出全文或手选）——悬停浮现，组内免打扰 */}
                <span className="mt-1 flex justify-end opacity-0 transition-opacity group-hover/msg:opacity-100">
                  <button
                    onClick={() => {
                      void navigator.clipboard?.writeText(m.content).then(
                        () => toast('已复制该条消息', 'info'),
                        () => toast('复制失败（剪贴板不可用）', 'error'),
                      )
                    }}
                    className={`flex items-center gap-0.5 rounded px-1.5 py-0.5 text-[9px] font-semibold ${
                      m.role === 'user' ? 'text-white/70 hover:text-white' : 'text-slate-300 hover:text-slate-500'
                    }`}
                  >
                    <Copy size={9} /> 复制
                  </button>
                </span>
                {(m.role === 'assistant' || m.role === 'user') && (m.refs.length > 0 || /```mermaid/.test(m.content)) && (
                  <div className="mt-2 flex flex-wrap items-center gap-1 border-t border-slate-200 pt-2">
                    {m.refs.map((id) => (
                      <button
                        key={id}
                        onClick={() => onLocateModule(id)}
                        className="flex items-center gap-0.5 rounded-full bg-white px-2 py-0.5 font-mono text-micro text-blue-600 shadow-sm hover:bg-blue-50"
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
                          className="w-36 rounded-full border border-emerald-300 bg-white px-2 py-0.5 text-cap outline-none"
                        />
                        <button
                          onClick={() => saveAsView(i)}
                          disabled={!viewName.trim()}
                          className="rounded-full bg-emerald-600 px-2 py-0.5 text-micro font-bold text-white disabled:opacity-40"
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
                        className="ml-auto flex items-center gap-1 rounded-full border border-emerald-200 bg-emerald-50 px-2 py-0.5 text-micro font-semibold text-emerald-700 hover:bg-emerald-100"
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
          <div className="anim-msg-in flex justify-start">
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
                    {o.desc && <span className="text-micro leading-4 text-slate-400">{o.desc}</span>}
                  </button>
                ))}
              </div>
            </div>
          </div>
        )}
        {sending && (
          <div className="anim-msg-in flex justify-start">
            <div className="flex items-center gap-1.5 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2 text-[11px] text-slate-400">
              <Loader2 size={12} className="animate-spin" /> 正在查询地图…
            </div>
          </div>
        )}
      </div>

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
            <span key={`img-${i}`} className="relative flex items-center gap-1 rounded-lg border border-slate-200 bg-white p-1">
              <img src={im.dataUrl} alt={im.name} className="h-8 w-8 rounded object-cover" />
              <span className="max-w-[90px] truncate text-micro text-slate-500">{im.name}</span>
              <button onClick={() => setImages((prev) => prev.filter((_, j) => j !== i))} className="text-slate-300 hover:text-red-500">
                <XIcon size={9} />
              </button>
            </span>
          ))}
          {attachments.map((a, i) => (
            <span key={i} className="flex items-center gap-1 rounded-full bg-blue-50 px-2 py-0.5 text-micro text-blue-700">
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
                  <span className="truncate text-[12px] font-medium text-slate-700">{m.name}</span>
                  <span className="ml-auto shrink-0 font-mono text-micro text-slate-400">{m.id}</span>
                </button>
              ))}
            </div>
          )}
          {/* 新手引导（2026-10-04）：空对话时给示例 prompt（AionUi 式"活的引导"——点一下即进入真实工作流） */}
          {messages.length === 0 && !sending && !suggest && (
            <div className="mb-1.5 flex flex-wrap gap-1.5">
              {samplePrompts.map((p) => (
                <button
                  key={p}
                  onClick={() => setInput(p)}
                  className="rounded-full border border-slate-200 bg-white px-2.5 py-1 text-[11px] text-slate-500 shadow-sm transition-colors hover:border-blue-300 hover:text-blue-600"
                >
                  {p}
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
