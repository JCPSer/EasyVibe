import { useCallback, useEffect, useRef, useState } from 'react'
import { toast } from '@/lib/toast'
import type { TaskDraft } from '@/lib/taskContext'
import type { CodeMap } from '@/types/map'
import { type ChatMessage, type Clarify, type PendingApproval } from './chat/types'
import { useConversations } from './chat/useConversations'
import { ConversationSwitcher } from './chat/ConversationSwitcher'
import { HeaderActions } from './chat/HeaderActions'
import { MessageStream } from './chat/MessageStream'
import { ComposerDock } from './chat/ComposerDock'

// M4-2 兼容：ConversationSummary 原从本文件导出，多个消费方沿用该路径
export type { ConversationSummary } from './chat/types'

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
  /** 2026-10-04 右栏就地对话：选中对象变化时父级投递 @提及（nonce 一次性消费，去重追加） */
  pendingMention?: { id: string; name: string; nonce: number } | null
  /** 2026-10-04 右栏就地对话：「新建会话」时的默认标题（如「模块 · 桌面端界面」）；null 走后端自动命名 */
  defaultConvTitle?: string | null
}

// 入口对话（F2 + M3-5 会话持久化 + M4-2 多会话）：服务端 SQLite 是会话事实源——
// 本文件为壳：会话状态装配 + 数据副作用；会话 CRUD 走 useConversations，
// 渲染归 ./chat/*（2026-10-05 防膨胀拆分，行为零改动）。
export function ChatPanel({ backendRepo, map, onLocateModule, onCreateTask, embedded, onConvChange, activeConvId, pendingDraft, pendingMention, defaultConvTitle }: Props) {
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [usage, setUsage] = useState({ promptTokens: 0, completionTokens: 0 })
  const [input, setInput] = useState('')
  const [sending, setSending] = useState(false)
  const [compacting, setCompacting] = useState(false)
  const [clarify, setClarify] = useState<Clarify | null>(null)
  // R1 清债：回放分页——首屏最近 50 条，"加载更早"再翻页（不再全表读）
  const [hasMore, setHasMore] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const abortRef = useRef<AbortController | null>(null)
  // 附件：文本类内容随消息注入（50KB×3）；图片走 images 通道（1.5MB×2，视觉模型消费）
  const [attachments, setAttachments] = useState<{ name: string; size: number; content: string }[]>([])
  const [images, setImages] = useState<{ name: string; size: number; dataUrl: string }[]>([])
  const listRef = useRef<HTMLDivElement>(null)
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  // D9 @模块：显式钉住的模块（随本条消息发送，后端拼成聚焦块前置给 LLM）
  const [mentions, setMentions] = useState<{ id: string; name: string }[]>([])
  // @ 触发建议浮层：query=@ 后的输入，start=@ 在输入框中的下标（用于选中后删除原文 token）
  const [suggest, setSuggest] = useState<{ query: string; start: number } | null>(null)
  const [suggestIdx, setSuggestIdx] = useState(0)
  // 内联审批：决策留痕 + 驳回理由
  const [decided, setDecided] = useState<Record<string, 'approved' | 'rejected'>>({})
  const [rejectingApproval, setRejectingApproval] = useState<PendingApproval | null>(null)
  const [rejectNote, setRejectNote] = useState('')

  const {
    convs, convId, setConvId, convQ, convMenuOpen, setConvMenuOpen,
    renaming, setRenaming, renameVal, setRenameVal, pendingApprovals, setPendingApprovals,
    loadConvs, createConv, renameConv, deleteConv, currentConv, displayTitle,
  } = useConversations({ backendRepo, activeConvId, onConvChange, defaultConvTitle })

  const convBody = convId ? { conv: convId } : {}
  // R3 #5：会话归属守卫——发送后立刻切会话，回复不得追加进新会话的消息列表（串台污染）
  const convIdRef = useRef<string | null>(convId)
  useEffect(() => {
    convIdRef.current = convId
  }, [convId])

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
  }, [setConvId, setConvMenuOpen, setPendingApprovals])

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

  // 2026-10-04 右栏就地对话：@提及投递消费（nonce 一次性；同 id 去重追加不重复钉）
  const consumedMentions = useRef<Set<number>>(new Set())
  useEffect(() => {
    if (!pendingMention || consumedMentions.current.has(pendingMention.nonce)) return
    consumedMentions.current.add(pendingMention.nonce)
    setMentions((prev) => (prev.some((m) => m.id === pendingMention.id) ? prev : [...prev, { id: pendingMention.id, name: pendingMention.name }]))
    requestAnimationFrame(() => textareaRef.current?.focus())
  }, [pendingMention])

  // R1 清债：加载更早一页（prepend 到消息头部）
  const loadEarlier = () => {
    if (!backendRepo || !messages.length || loadingMore) return
    setLoadingMore(true)
    fetch(`/api/repos/${backendRepo}/chat${convQ ? convQ + '&' : '?'}before=${messages[0].id}&limit=50`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: { hasMore: boolean; messages: { id: number; role: string; content: string }[] } }) => {
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
      .then((d: { data: { usage: { promptTokens: number; completionTokens: number }; hasMore: boolean; pendingApprovals?: PendingApproval[]; conversation?: { id: string; title: string | null }; messages: { id: number; role: string; content: string }[] } }) => {
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
  }, [backendRepo, convId, convQ, loadConvs, setConvId, setPendingApprovals])

  // 内联审批：选项即按钮，提交后原地变"✓ 已回复"（AionUI PermissionRequestPanel 模式）
  // R3 C4：驳回必须带理由（后端 400 强制）——卡内展开理由输入，失败 toast 带后端归因
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
    [backendRepo, loadConvs, setPendingApprovals],
  )

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
        setDecided({})
        loadConvs()
      })
      .catch(() => toast('重置失败（需要本地后端在线）', 'error'))
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

  return (
    <div className="flex h-full flex-col">
      {!embedded && (
        <ConversationSwitcher
          currentConv={currentConv}
          displayTitle={displayTitle}
          convs={convs}
          convId={convId}
          convMenuOpen={convMenuOpen}
          setConvMenuOpen={setConvMenuOpen}
          switchConv={switchConv}
          renaming={renaming}
          renameVal={renameVal}
          setRenameVal={setRenameVal}
          renameConv={renameConv}
          setRenaming={setRenaming}
          createConv={createConv}
        />
      )}

      <HeaderActions
        usage={usage}
        convId={convId}
        embedded={embedded}
        backendRepo={backendRepo}
        sending={sending}
        compacting={compacting}
        messageCount={messages.length}
        hasUserMessage={messages.some((m) => m.role === 'user')}
        onDelete={deleteConv}
        onExportClear={exportAndClear}
        onUpgrade={upgradeToTask}
        onCompress={compressCtx}
        onCreate={createConv}
      />

      <MessageStream
        listRef={listRef}
        pendingApprovals={pendingApprovals}
        decided={decided}
        rejectingApproval={rejectingApproval}
        setRejectingApproval={setRejectingApproval}
        rejectNote={rejectNote}
        setRejectNote={setRejectNote}
        decide={decide}
        hasMore={hasMore}
        loadingMore={loadingMore}
        loadEarlier={loadEarlier}
        messages={messages}
        map={map}
        setInput={setInput}
        setClarify={setClarify}
        textareaRef={textareaRef}
        onLocateModule={onLocateModule}
        clarify={clarify}
        sending={sending}
        backendRepo={backendRepo}
      />

      <ComposerDock
        backendRepo={backendRepo}
        map={map}
        input={input}
        setInput={setInput}
        mentions={mentions}
        setMentions={setMentions}
        attachments={attachments}
        setAttachments={setAttachments}
        images={images}
        setImages={setImages}
        suggest={suggest}
        setSuggest={setSuggest}
        suggestIdx={suggestIdx}
        setSuggestIdx={setSuggestIdx}
        sending={sending}
        send={send}
        abortRef={abortRef}
        textareaRef={textareaRef}
      />
    </div>
  )
}
