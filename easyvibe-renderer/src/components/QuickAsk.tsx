import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { toast } from '@/lib/toast'
import { AnswerCards } from '@/components/AnswerCards'
import { type ConversationSummary } from '@/components/ChatPanel'
import type { TaskDraft } from '@/lib/taskContext'
import type { CodeMap } from '@/types/map'
import type { Selection } from '@/components/DetailPanel'
import { buildTaskDraftFromChat } from '@/lib/chatUpgrade'
import { onTaskEvent } from '@/lib/growthBus'
import { ONBOARDING_COPY } from '@/lib/onboardingCopy'
import { Send, Loader2, BookmarkPlus, Check, Crosshair, Shrink, Wrench, Square, Paperclip, X as XIcon, Download, AtSign, Plus, Pencil, AlertTriangle, Copy, MoreHorizontal, MessagesSquare, ArrowRight } from 'lucide-react'

// QuickAsk（2026-10-05 右栏对话 Redesign-A 检查器文档流）：
// Linear 的克制 × Xcode Inspector 的读出感——ContextBar（状态/会话/审批/上下文）+
// ThreadStream（回合文档流：右对齐问句、全宽回答、裸 mono 引用芯片）+ ComposerDock（glass 输入坞）。
// 数据契约与工作台全功能 ChatPanel 完全镜像（同一服务端会话库），差异只在形态：
// 审批只出口（去工作台裁决，不在右栏内联决策）、无 token 统计常驻行（收纳进「…」）。
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

interface PendingApproval {
  taskId: string
  title: string
  gate: string | null
}

const GATE_LABEL: Record<string, string> = { plan: '① 计划审批', diff: '② Diff 审批', report: '③ 审查报告' }

interface ChatRestore {
  summary: string | null
  conversation?: { id: string; title: string | null }
  messages: { id: number; role: string; content: string }[]
  hasMore: boolean
  pendingApprovals?: PendingApproval[]
  usage: { promptTokens: number; completionTokens: number }
}

interface Props {
  backendRepo: string | null
  map: CodeMap | null
  selection: Selection
  onCreateTask: (draft: TaskDraft) => void
  onLocateModule: (moduleId: string) => void
  /** 审批出口：跳工作台「任务对话」页裁决（形态分裂裁决=右栏只出口，不做内联决策） */
  onGoWorkbench?: () => void
  /** 选中对象变化时父级投递 @提及（nonce 一次性消费，去重追加） */
  pendingMention?: { id: string; name: string; nonce: number } | null
  /** 「新建会话」时的默认标题（如「模块 · 桌面端界面」）；null 走后端自动命名 */
  defaultConvTitle?: string | null
}

export function QuickAsk({ backendRepo, map, selection, onCreateTask, onLocateModule, onGoWorkbench, pendingMention, defaultConvTitle }: Props) {
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [usage, setUsage] = useState({ promptTokens: 0, completionTokens: 0 })
  const [input, setInput] = useState('')
  const [sending, setSending] = useState(false)
  const [savedIdx, setSavedIdx] = useState<number | null>(null)
  const [namingIdx, setNamingIdx] = useState<number | null>(null)
  const [viewName, setViewName] = useState('')
  const [compacting, setCompacting] = useState(false)
  const [clarify, setClarify] = useState<Clarify | null>(null)
  // R1 回放分页：首屏最近 50 条，"加载更早"再翻页
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

  // ---- 多会话状态（与 ChatPanel 同契约：内部自治，无受控模式） ----
  const [convs, setConvs] = useState<ConversationSummary[]>([])
  const [convId, setConvId] = useState<string | null>(null) // null = 后端回落（最近活跃）
  const [convMenuOpen, setConvMenuOpen] = useState(false)
  const [moreOpen, setMoreOpen] = useState(false)
  const [renaming, setRenaming] = useState(false)
  const [renameVal, setRenameVal] = useState('')
  const [pendingApprovals, setPendingApprovals] = useState<PendingApproval[]>([])
  // ContextChip 的一次性移除（仅对本个选中对象；重新选中同对象 → PanelChat 重新投递 nonce，chip 回归）
  const [chipDismissed, setChipDismissed] = useState(false)

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

  // 会话列表 + 审批数随任务事件实时刷新（页签上永远看得到"谁在等你批准"）
  useEffect(() => onTaskEvent(() => { loadConvs(); refreshPending() }), [loadConvs, refreshPending])

  const createConv = useCallback(() => {
    if (!backendRepo) return
    fetch(`/api/repos/${backendRepo}/conversations`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      // 右栏就地对话传入默认标题（如「模块 · 桌面端界面」）；未传走后端自动命名
      body: JSON.stringify(defaultConvTitle ? { title: defaultConvTitle } : {}),
    })
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: ConversationSummary }) => {
        setConvId(d.data.id)
        setConvMenuOpen(false)
        setMoreOpen(false)
        loadConvs()
      })
      .catch(() => toast('新建会话失败', 'error'))
  }, [backendRepo, loadConvs, defaultConvTitle])

  const switchConv = useCallback((id: string | null) => {
    setConvId(id)
    setConvMenuOpen(false)
    setClarify(null)
    setPendingApprovals([])
    // 切会话清空输入框与 @提及——消除"上一会话的半成品文本串到下一会话"
    setInput('')
    setMentions([])
    setSuggest(null)
  }, [])

  // @提及投递消费（渲染期派生态：与 PanelChat 同款的 adjust-state-during-render 模式；
  // nonce 一次性；同 id 去重追加不重复钉）
  const [consumedNonce, setConsumedNonce] = useState<number | null>(null)
  if (pendingMention && consumedNonce !== pendingMention.nonce) {
    setConsumedNonce(pendingMention.nonce)
    setMentions((prev) => (prev.some((m) => m.id === pendingMention.id) ? prev : [...prev, { id: pendingMention.id, name: pendingMention.name }]))
  }
  // 聚焦走 effect（渲染期不做 DOM 副作用）：nonce 已被本渲染消费即聚焦输入框
  useEffect(() => {
    if (pendingMention && consumedNonce === pendingMention.nonce) {
      requestAnimationFrame(() => textareaRef.current?.focus())
    }
  }, [pendingMention, consumedNonce])

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
  // stale 保护：快速切换仓库/会话时，旧 fetch 返回不得覆盖新会话；
  // 旧会话消息在 cleanup 中同步清空（set-state-in-effect 纪律：effect 体内不直接 setState）
  useEffect(() => {
    if (!backendRepo) return
    let stale = false
    loadConvs()
    fetch(`/api/repos/${backendRepo}/chat${convQ ? convQ + '&' : '?'}limit=50`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: ChatRestore }) => {
        if (stale) return
        setUsage(d.data.usage)
        setHasMore(d.data.hasMore)
        setPendingApprovals(d.data.pendingApprovals ?? [])
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
      // 切换仓库/会话的瞬间清空旧消息列表——避免加载间隙展示上一会话内容（串台观感）
      setMessages([])
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
          // auto-compact 留痕（"上下文已压缩：82%→34%"系统消息）
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

  // 手动压缩（压缩上下文按钮；自动阈值兜底之外的主动手段）
  const compressCtx = () => {
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
        loadConvs()
      })
      .catch(() => toast('重置失败（需要本地后端在线）', 'error'))
  }

  // 澄清卡点选：选择即回答（选择带建议理由回传）
  const answerClarify = (label: string, desc?: string) => {
    setClarify(null)
    setInput(`选择：${label}${desc ? `（${desc}）` : ''}`)
    requestAnimationFrame(() => textareaRef.current?.focus())
  }

  // D2 拍板：导出并清空——留痕承诺不自动清，由用户显式"导出归档 → 清空"
  const exportAndClear = () => {
    exportChat()
    setTimeout(() => {
      if (!window.confirm('已导出。清空当前会话？（清空后不可恢复，视图文件不受影响）')) return
      reset()
    }, 400)
  }

  // 导出对话为 Markdown（对话含 mermaid 图，可直接评审/沉淀）
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

  // S1：对话升级任务——最近用户问题 + 近 6 轮摘要 + 引用模块 → TaskDraft（表单可再编辑）
  const upgradeToTask = () => {
    const draft = buildTaskDraftFromChat(messages, convId)
    if (draft) onCreateTask(draft)
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
          // 回答里的流程图（mermaid）随视图保存——视图成为"用户创建的图"资产
          ...(m.content.match(/```mermaid[\s\S]*?```/g) ?? []).map((content) => ({ type: 'mermaid', content, note: q })),
        ],
      }),
    })
      .then(async (r) => {
        if (r.status === 409) {
          // 同名冲突用户裁决——确认后带 force 覆盖
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
  const pendingCount = currentConv?.runtime.pendingConfirmations ?? 0

  // 选中模块（ContextChip 数据源；子模块选中落到其父模块）
  const selMod =
    selection?.kind === 'module'
      ? (map?.modules.find((m) => m.id === selection.id) ?? null)
      : selection?.kind === 'submodule'
        ? (map?.modules.find((m) => m.id === selection.parentId) ?? null)
        : null
  // 每条回答的 TurnFooter 是否存在的判定 + "流内最后一条带 footer 的回答"（其复制按钮常显）
  const hasFooter = (m: ChatMessage) => m.role === 'assistant' && (m.refs.length > 0 || /```mermaid/.test(m.content))
  const lastFooterIdx = useMemo(() => {
    for (let i = messages.length - 1; i >= 0; i--) {
      const m = messages[i]
      if (m.role === 'assistant' && (m.refs.length > 0 || /```mermaid/.test(m.content))) return i
    }
    return -1
  }, [messages])

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
    <div className="flex h-full flex-col px-3">
      {/* ---- ContextBar：状态点 + 会话标题（点开历史）+ 审批角标 + @上下文 chip + 「…」收纳 ---- */}
      <div className="relative">
        <div className="flex h-8 items-center gap-1.5 border-b border-slate-100 px-1 dark:border-slate-800">
          {/* StateDot：running=琥珀转圈 / 待审批=红点 / idle=灰点 */}
          {currentConv?.runtime.state === 'running' ? (
            <Loader2 size={10} className="shrink-0 animate-spin text-amber-500" />
          ) : pendingCount > 0 ? (
            <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-red-500" />
          ) : (
            <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-slate-300 dark:bg-slate-600" />
          )}
          <button
            onClick={() => setConvMenuOpen((v) => !v)}
            className="min-w-0 flex-1 rounded-md px-1 py-0.5 text-left transition-colors hover:bg-slate-100 dark:hover:bg-slate-800/70"
            title="切换 / 新建会话"
          >
            <span className="block truncate text-[12px] font-semibold text-slate-700 dark:text-slate-200">{displayTitle}</span>
          </button>
          {/* 审批角标：右栏只出口——点击跳工作台裁决 */}
          {pendingCount > 0 && (
            <button
              onClick={() => onGoWorkbench?.()}
              className="flex shrink-0 items-center gap-1 rounded-full px-1.5 py-0.5 transition-colors hover:bg-red-50 dark:hover:bg-red-950/40"
              title="有待审批任务：去工作台裁决"
            >
              <span className="h-1.5 w-1.5 rounded-full bg-red-500" />
              <span className="text-micro font-bold leading-3 text-red-600 dark:text-red-400">{pendingCount}</span>
            </button>
          )}
          {/* ContextChip：选中模块的只读上下文（× 仅对本对象一次性移除） */}
          {selMod && !chipDismissed && (
            <span className="flex shrink-0 items-center gap-1 rounded-md bg-slate-100 px-1.5 py-0.5 font-mono text-micro text-slate-600 dark:bg-slate-800 dark:text-slate-300">
              @{selMod.name}
              <button
                onClick={() => setChipDismissed(true)}
                className="text-slate-400 transition-colors hover:text-slate-600 dark:hover:text-slate-200"
                title="移除上下文标记（不影响 @提及）"
              >
                <XIcon size={9} />
              </button>
            </span>
          )}
          <div className="relative">
            <button
              onClick={() => setMoreOpen((v) => !v)}
              className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-slate-400 transition-colors hover:bg-slate-100 hover:text-slate-600 dark:text-slate-500 dark:hover:bg-slate-800/70 dark:hover:text-slate-300"
              title="更多操作"
            >
              <MoreHorizontal size={13} />
            </button>
            {moreOpen && (
              <>
                <div className="fixed inset-0 z-30" onClick={() => setMoreOpen(false)} />
                <div className="glass anim-scale-in absolute right-0 top-full z-40 mt-1 w-48 rounded-xl border border-slate-200 p-1 shadow-xl dark:border-slate-700">
                  {/* 信息头：带标签的 token 统计 */}
                  <p className="tnum flex items-baseline justify-between px-2 pb-1 pt-1.5 text-micro text-slate-400 dark:text-slate-500">
                    <span>输入 {usage.promptTokens.toLocaleString()}</span>
                    <span>输出 {usage.completionTokens.toLocaleString()}</span>
                  </p>
                  <button
                    onClick={() => { setMoreOpen(false); upgradeToTask() }}
                    disabled={!backendRepo || sending || !messages.some((m) => m.role === 'user')}
                    className="flex w-full items-center gap-1.5 rounded-lg px-2 py-1.5 text-[12px] text-slate-600 transition-colors hover:bg-slate-50 disabled:opacity-40 dark:text-slate-300 dark:hover:bg-slate-800/70"
                  >
                    <Wrench size={11} /> 转为任务
                  </button>
                  <div className="mx-2 my-0.5 border-t border-slate-100 dark:border-slate-800" />
                  <button
                    onClick={() => { setMoreOpen(false); exportAndClear() }}
                    disabled={!backendRepo || messages.length === 0}
                    className="flex w-full items-center gap-1.5 rounded-lg px-2 py-1.5 text-[12px] text-slate-600 transition-colors hover:bg-slate-50 disabled:opacity-40 dark:text-slate-300 dark:hover:bg-slate-800/70"
                  >
                    <Download size={11} /> 导出/清空
                  </button>
                  <button
                    onClick={() => { setMoreOpen(false); compressCtx() }}
                    disabled={!backendRepo || compacting}
                    className="flex w-full items-center gap-1.5 rounded-lg px-2 py-1.5 text-[12px] text-slate-600 transition-colors hover:bg-slate-50 disabled:opacity-40 dark:text-slate-300 dark:hover:bg-slate-800/70"
                  >
                    <Shrink size={11} /> {compacting ? '压缩中…' : '压缩上下文'}
                  </button>
                  {convId && (
                    <>
                      <div className="mx-2 my-0.5 border-t border-slate-100 dark:border-slate-800" />
                      <button
                        onClick={() => { setMoreOpen(false); setRenaming(true); setRenameVal(currentConv?.title ?? '') }}
                        disabled={!backendRepo}
                        className="flex w-full items-center gap-1.5 rounded-lg px-2 py-1.5 text-[12px] text-slate-600 transition-colors hover:bg-slate-50 disabled:opacity-40 dark:text-slate-300 dark:hover:bg-slate-800/70"
                      >
                        <Pencil size={11} /> 重命名会话
                      </button>
                      <button
                        onClick={() => { setMoreOpen(false); deleteConv() }}
                        disabled={!backendRepo}
                        className="flex w-full items-center gap-1.5 rounded-lg px-2 py-1.5 text-[12px] text-red-500 transition-colors hover:bg-red-50 disabled:opacity-40 dark:hover:bg-red-950/40"
                      >
                        <XIcon size={11} /> 删除会话
                      </button>
                    </>
                  )}
                </div>
              </>
            )}
          </div>
        </div>
        {/* 历史下拉：切换 / 新建会话（与工作台同一服务端会话库） */}
        {convMenuOpen && (
          <>
            <div className="fixed inset-0 z-30" onClick={() => setConvMenuOpen(false)} />
            <div className="glass anim-scale-in absolute left-0 right-0 top-full z-40 mt-1 max-h-72 overflow-y-auto rounded-xl border border-slate-200 p-1.5 shadow-xl dark:border-slate-700">
              {convs.map((c) => (
                <div key={c.id} className="flex items-center gap-1.5 rounded-lg px-2 py-1.5 hover:bg-slate-50 dark:hover:bg-slate-800/70">
                  <button
                    className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
                    onClick={() => switchConv(c.id === convId ? null : c.id)}
                  >
                    {c.runtime.state === 'running' ? (
                      <Loader2 size={11} className="shrink-0 animate-spin text-amber-500" />
                    ) : c.runtime.pendingConfirmations > 0 ? (
                      <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-red-500" />
                    ) : (
                      <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-slate-300 dark:bg-slate-600" />
                    )}
                    <span className={`min-w-0 flex-1 truncate text-[12px] ${c.id === convId ? 'font-bold text-blue-700 dark:text-blue-400' : 'text-slate-600 dark:text-slate-300'}`}>
                      {c.title ?? '未命名会话'}
                    </span>
                    {c.runtime.pendingConfirmations > 0 && (
                      <span className="rounded-full bg-red-500 px-1.5 text-micro font-bold leading-4 text-white">{c.runtime.pendingConfirmations}</span>
                    )}
                    <span className="tnum shrink-0 text-micro text-slate-300 dark:text-slate-600">{c.messageCount}条</span>
                  </button>
                  {c.id === convId && (
                    <button onClick={() => { setRenaming(true); setRenameVal(c.title ?? '') }} className="shrink-0 rounded p-0.5 text-slate-300 hover:text-blue-500 dark:text-slate-600 dark:hover:text-blue-400" title="重命名">
                      <Pencil size={10} />
                    </button>
                  )}
                </div>
              ))}
              {convs.length === 0 && <p className="px-2 py-1.5 text-cap text-slate-400 dark:text-slate-500">还没有会话，发第一条消息即创建</p>}
              <button
                onClick={createConv}
                className="mt-1 flex w-full items-center justify-center gap-1 rounded-lg bg-blue-600 px-2 py-1.5 text-cap font-semibold text-white hover:bg-blue-700"
              >
                <Plus size={11} /> 新建会话
              </button>
            </div>
          </>
        )}
        {renaming && (
          <div className="absolute left-0 right-0 top-full z-50 mt-1 flex items-center gap-1 rounded-xl border border-slate-200 bg-white p-2 shadow-xl dark:border-slate-700 dark:bg-slate-900">
            <input
              autoFocus
              value={renameVal}
              onChange={(e) => setRenameVal(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') renameConv()
                if (e.key === 'Escape') setRenaming(false)
              }}
              placeholder="会话名…"
              className="flex-1 rounded-lg border border-slate-200 px-2 py-1 text-[12px] outline-none focus:border-blue-300 dark:border-slate-700 dark:bg-slate-950/70 dark:text-slate-200"
            />
            <button onClick={renameConv} className="rounded-lg bg-blue-600 px-2.5 py-1 text-cap font-bold text-white">存</button>
          </div>
        )}
      </div>

      {/* ---- ThreadStream：回合文档流（检查器读出感——问句右对齐无气泡，回答全宽顶线分隔） ---- */}
      <div ref={listRef} className="flex-1 space-y-5 overflow-y-auto pt-3">
        {hasMore && (
          <div className="flex justify-center">
            <button
              onClick={loadEarlier}
              disabled={loadingMore}
              className="text-cap text-slate-400 transition-colors hover:text-slate-600 disabled:opacity-40 dark:text-slate-500 dark:hover:text-slate-300"
            >
              {loadingMore ? '加载中…' : '↑ 加载更早的消息'}
            </button>
          </div>
        )}
        {messages.length === 0 && pendingApprovals.length === 0 && (
          /* 空态邀请卡：图标 + 一句主张 + 示例 chip 一处说完 */
          <div className="flex flex-col items-center gap-3 px-4 pt-12 text-center">
            <span className="flex h-10 w-10 items-center justify-center rounded-2xl bg-blue-50 text-blue-500 dark:bg-blue-950/40 dark:text-blue-400">
              <MessagesSquare size={18} />
            </span>
            <div>
              <p className="text-[12px] font-semibold text-slate-600 dark:text-slate-300">基于语义代码地图提问</p>
              <p className="mt-0.5 text-micro text-slate-400 dark:text-slate-500">回答可存为视图，引用模块可定位到画布</p>
            </div>
            <div className="flex flex-wrap justify-center gap-1.5">
              {samplePrompts.map((p) => (
                <button
                  key={p}
                  onClick={() => { setInput(p); textareaRef.current?.focus() }}
                  className="rounded-full bg-slate-100 px-2.5 py-1 text-cap text-slate-600 transition-colors hover:bg-blue-50 hover:text-blue-600 dark:bg-slate-800 dark:text-slate-300 dark:hover:bg-blue-950/40 dark:hover:text-blue-400"
                >
                  {p}
                </button>
              ))}
            </div>
          </div>
        )}
        {/* 审批卡：只出口——「去工作台审批」，不做内联决策（形态分裂裁决） */}
        {pendingApprovals.map((p) => (
          <div key={p.taskId + ':' + (p.gate ?? '')} className="anim-msg-in rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 dark:border-amber-900/60 dark:bg-amber-950/40">
            <p className="flex items-center gap-1.5 text-cap font-semibold leading-4 text-amber-800 dark:text-amber-200">
              <AlertTriangle size={11} />
              审批请求 <span className="rounded-full bg-white/80 px-1.5 text-micro font-bold text-amber-600 dark:bg-slate-900/80 dark:text-amber-400">{GATE_LABEL[p.gate ?? ''] ?? p.gate ?? '审批'}</span>
            </p>
            <p className="mt-1 text-[12px] leading-5 text-slate-700 dark:text-slate-200">{p.title}</p>
            {onGoWorkbench && (
              <button
                onClick={onGoWorkbench}
                className="mt-1.5 flex items-center gap-1 rounded-lg bg-white px-2 py-1 text-micro font-semibold text-amber-700 shadow-sm transition-colors hover:bg-amber-100 dark:bg-slate-900 dark:text-amber-400 dark:hover:bg-amber-900/40"
              >
                去工作台审批 <ArrowRight size={10} />
              </button>
            )}
          </div>
        ))}
        {messages.map((m, i) =>
          m.role === 'system' ? (
            <div key={m.id ?? i} className="anim-msg-in flex justify-center">
              <span className="rounded-full bg-slate-100 px-2.5 py-0.5 text-micro text-slate-400 dark:bg-slate-800 dark:text-slate-500">{m.content}</span>
            </div>
          ) : m.role === 'user' ? (
            /* 用户问句：右对齐、无气泡、零蓝色（颜色宪法：蓝=交互对象） */
            <div key={m.id ?? i} className="anim-msg-in flex justify-end">
              <div className="max-w-[92%] select-text text-[12px] font-semibold leading-5 text-slate-800 dark:text-slate-100">
                {m.images?.map((im, j) => (
                  <img key={j} src={im.dataUrl} alt={im.name} className="mb-1.5 max-h-40 rounded-lg" />
                ))}
                {m.mentions && m.mentions.length > 0 && (
                  <span className="mb-1 flex flex-wrap justify-end gap-1">
                    {m.mentions.map((mm) => (
                      <span key={mm.id} className="flex items-center gap-0.5 rounded-full bg-slate-100 px-1.5 py-0.5 font-normal text-micro text-slate-600 dark:bg-slate-800 dark:text-slate-300">
                        <AtSign size={8} />
                        {mm.name}
                      </span>
                    ))}
                  </span>
                )}
                <span className="block whitespace-pre-wrap break-words">{m.content}</span>
              </div>
            </div>
          ) : (
            /* 助手回答：全宽文档流，顶线分隔；有章节结构时 AnswerCards 拆卡 */
            <div key={m.id ?? i} className="anim-msg-in group/msg border-t border-slate-100 pt-2 dark:border-slate-800">
              <div className="select-text text-[12px] leading-5 text-slate-700 dark:text-slate-200">
                {!m.content.trim() ? (
                  <span className="text-slate-400 dark:text-slate-500">（此条未获得回答）</span>
                ) : (
                  <AnswerCards content={m.content} />
                )}
              </div>
              {hasFooter(m) && (
                <div className="mt-2 flex items-center gap-1 border-t border-slate-100 pt-1.5 dark:border-slate-800">
                  {/* 引用芯片：裸 mono，hover 定位（蓝=交互对象） */}
                  {m.refs.map((id) => (
                    <button
                      key={id}
                      onClick={() => onLocateModule(id)}
                      className="flex items-center gap-0.5 font-mono text-micro text-slate-500 transition-colors hover:text-blue-600 dark:text-slate-400 dark:hover:text-blue-400"
                      title="定位到画布"
                    >
                      <Crosshair size={9} />
                      {id}
                    </button>
                  ))}
                  <span className="ml-auto flex items-center gap-1">
                    {/* 复制：悬停浮现；流内最后一条回答常显（opacity-40） */}
                    <button
                      onClick={() => {
                        void navigator.clipboard?.writeText(m.content).then(
                          () => toast('已复制该条消息', 'info'),
                          () => toast('复制失败（剪贴板不可用）', 'error'),
                        )
                      }}
                      className={`rounded p-1 text-slate-300 transition-opacity hover:text-slate-500 dark:text-slate-600 dark:hover:text-slate-300 ${i === lastFooterIdx ? 'opacity-40' : 'opacity-0 group-hover/msg:opacity-100'}`}
                      title="复制该条回答"
                    >
                      <Copy size={10} />
                    </button>
                    {namingIdx === i ? (
                      /* 存图命名——多张视图靠问题截断无法区分，存前给命名框 */
                      <span className="flex items-center gap-1">
                        <input
                          autoFocus
                          value={viewName}
                          onChange={(e) => setViewName(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') saveAsView(i)
                            if (e.key === 'Escape') setNamingIdx(null)
                          }}
                          placeholder="视图名…"
                          className="w-32 rounded-full border border-emerald-300 bg-white px-2 py-0.5 text-cap outline-none dark:border-emerald-800 dark:bg-slate-900 dark:text-slate-200"
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
                        className="flex items-center gap-1 rounded-full bg-emerald-50 px-2 py-0.5 text-micro font-semibold text-emerald-700 transition-colors hover:bg-emerald-100 dark:bg-emerald-950/40 dark:text-emerald-400 dark:hover:bg-emerald-900/40"
                        title="把本回答（含流程图）存为可复用视图（.easyvibe/views/）"
                      >
                        {savedIdx === i ? <Check size={10} /> : <BookmarkPlus size={10} />}
                        {savedIdx === i ? '已存视图' : '存为视图'}
                      </button>
                    )}
                  </span>
                </div>
              )}
            </div>
          ),
        )}
        {/* S1 grill-me 澄清卡：选择题形态，点选即填入输入框 */}
        {clarify && (
          <div className="anim-msg-in rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 dark:border-amber-900/60 dark:bg-amber-950/40">
            <p className="text-cap font-semibold leading-4 text-amber-800 dark:text-amber-200">
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
                  className="flex w-full flex-col items-start gap-0.5 rounded-md border border-amber-200 bg-white px-2 py-1.5 text-left transition-colors hover:border-blue-300 hover:bg-blue-50 dark:border-amber-900/60 dark:bg-slate-900 dark:hover:bg-blue-950/40"
                >
                  <span className="text-cap font-medium leading-4 text-slate-700 dark:text-slate-200">{o.label}</span>
                  {o.desc && <span className="text-micro leading-4 text-slate-400 dark:text-slate-500">{o.desc}</span>}
                </button>
              ))}
            </div>
          </div>
        )}
        {sending && (
          <p className="anim-msg-in flex items-center gap-1.5 text-micro text-slate-400 dark:text-slate-500">
            <Loader2 size={10} className="animate-spin" /> 正在查询地图…
          </p>
        )}
      </div>

      {/* ---- ComposerDock：glass 输入坞（@建议浮层贴坞顶部长出） ---- */}
      <div className="relative">
        {suggest && mentionCandidates(suggest.query).length > 0 && (
          /* D9 @模块建议浮层：@ 后输入即过滤，↑↓ 选择，Enter/Tab 选中，Esc 关闭 */
          <div className="anim-scale-in absolute bottom-full left-0 z-20 mb-1.5 w-64 overflow-hidden rounded-lg border border-slate-200 bg-white shadow-lg dark:border-slate-700 dark:bg-slate-900">
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
        <div className="glass elev-2 m-3 mt-2 rounded-[10px] p-2.5">
          {(mentions.length > 0 || attachments.length > 0 || images.length > 0) && (
            <div className="mb-1.5 flex flex-wrap items-center gap-1.5">
              {mentions.map((m) => (
                <span key={m.id} className="flex items-center gap-1 rounded-full bg-slate-100 px-2 py-0.5 text-micro text-slate-600 dark:bg-slate-800 dark:text-slate-300">
                  <AtSign size={9} />
                  {m.name}
                  <button onClick={() => setMentions((prev) => prev.filter((x) => x.id !== m.id))} className="text-slate-400 transition-colors hover:text-slate-700 dark:hover:text-slate-200">
                    <XIcon size={9} />
                  </button>
                </span>
              ))}
              {images.map((im, i) => (
                <span key={`img-${i}`} className="relative flex items-center gap-1 rounded-lg bg-slate-100 p-1 dark:bg-slate-800">
                  <img src={im.dataUrl} alt={im.name} className="h-6 w-6 rounded object-cover" />
                  <span className="max-w-[90px] truncate text-micro text-slate-500 dark:text-slate-400">{im.name}</span>
                  <button onClick={() => setImages((prev) => prev.filter((_, j) => j !== i))} className="text-slate-400 transition-colors hover:text-red-500">
                    <XIcon size={9} />
                  </button>
                </span>
              ))}
              {attachments.map((a, i) => (
                <span key={i} className="flex items-center gap-1 rounded-full bg-slate-100 px-2 py-0.5 text-micro text-slate-500 dark:bg-slate-800 dark:text-slate-400">
                  <Paperclip size={9} />
                  {a.name}（{(a.size / 1024).toFixed(0)}KB）
                  <button onClick={() => setAttachments((prev) => prev.filter((_, j) => j !== i))} className="text-slate-400 transition-colors hover:text-red-500">
                    <XIcon size={9} />
                  </button>
                </span>
              ))}
            </div>
          )}
          <div className="flex items-end gap-1.5">
            <input
              ref={fileRef}
              type="file"
              multiple
              className="hidden"
              onChange={(e) => addFiles(e.target.files)}
            />
            {/* 回形针 ghost 化：次要动作不与发送按钮同视觉重量 */}
            <button
              onClick={() => fileRef.current?.click()}
              disabled={!backendRepo || attachments.length >= 3}
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-slate-400 transition-colors hover:bg-slate-100 hover:text-slate-600 disabled:opacity-40 dark:text-slate-500 dark:hover:bg-slate-800/70 dark:hover:text-slate-300"
              title="添加附件（代码/日志/文档文本，≤50KB×3；图片 ≤1.5MB×2）——内容随消息一起发给 AI"
            >
              <Paperclip size={14} />
            </button>
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
              rows={1}
              placeholder={backendRepo ? '就这张架构地图提问…' : '需要本地后端在线'}
              title="Enter 发送，Shift+Enter 换行"
              disabled={!backendRepo || sending}
              className="w-full flex-1 resize-none border-0 bg-transparent px-1 py-1.5 text-[12px] leading-5 text-slate-700 outline-none ring-0 placeholder:text-slate-400 focus:border-0 focus:ring-0 disabled:opacity-50 dark:text-slate-200 dark:placeholder:text-slate-500"
            />
            {sending ? (
              <button
                onClick={() => abortRef.current?.abort()}
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-red-200 text-red-600 transition-colors hover:bg-red-50 dark:border-red-900/60 dark:hover:bg-red-950/40"
                title="停止等待本次回答"
              >
                <Square size={11} />
              </button>
            ) : (
              /* 禁用态换材质而非变淡：空输入=灰底灰字，有输入=实蓝——状态对比由颜色承担 */
              <button
                onClick={send}
                disabled={!backendRepo}
                className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-full transition-colors ${
                  input.trim() && backendRepo
                    ? 'bg-blue-600 text-white hover:bg-blue-700'
                    : 'bg-slate-100 text-slate-400 dark:bg-slate-800 dark:text-slate-600'
                }`}
                title="发送（Enter）"
              >
                <Send size={13} />
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}
