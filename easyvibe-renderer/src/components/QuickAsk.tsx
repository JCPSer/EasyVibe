import { useCallback, useEffect, useRef, useState } from 'react'
import { toast } from '@/lib/toast'
import type { TaskDraft } from '@/lib/taskContext'
import type { CodeMap } from '@/types/map'
import type { Selection } from '@/components/DetailPanel'
import { buildTaskDraftFromChat } from '@/lib/chatUpgrade'
import { Download, Loader2, MoreHorizontal, Pencil, Plus, Shrink, Wrench, X as XIcon } from 'lucide-react'
import { type ChatMessage, type Clarify, type PendingApproval } from './chat/types'
import { useConversations } from './chat/useConversations'
import { QuickAskStream } from './chat/QuickAskStream'
import { QuickAskComposer } from './chat/QuickAskComposer'

// QuickAsk（2026-10-05 右栏对话 Redesign-A 检查器文档流）：
// ContextBar（状态/会话/审批/上下文）+ ThreadStream + ComposerDock。
// 本文件为壳：会话状态装配 + 数据副作用；渲染归 ./chat/*（2026-10-05 防膨胀拆分，行为零改动）。
// 数据契约与工作台全功能 ChatPanel 完全镜像（同一服务端会话库），差异只在形态：
// 审批只出口（去工作台裁决，不在右栏内联决策）、无 token 统计常驻行（收纳进「…」）。
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
  const [compacting, setCompacting] = useState(false)
  const [clarify, setClarify] = useState<Clarify | null>(null)
  // R1 回放分页：首屏最近 50 条，"加载更早"再翻页
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
  const [moreOpen, setMoreOpen] = useState(false)
  // ContextChip 的一次性移除（仅对本个选中对象；重新选中同对象 → PanelChat 重新投递 nonce，chip 回归）
  const [chipDismissed, setChipDismissed] = useState(false)

  const {
    convs, convId, setConvId, convQ, convMenuOpen, setConvMenuOpen,
    renaming, setRenaming, renameVal, setRenameVal, pendingApprovals, setPendingApprovals,
    loadConvs, createConv, renameConv, deleteConv, currentConv, displayTitle,
  } = useConversations({ backendRepo, defaultConvTitle, onCreated: () => setMoreOpen(false) })

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
    // 切会话清空输入框与 @提及——消除"上一会话的半成品文本串到下一会话"
    setInput('')
    setMentions([])
    setSuggest(null)
  }, [setConvId, setConvMenuOpen, setPendingApprovals])

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
  // stale 保护：快速切换仓库/会话时，旧 fetch 返回不得覆盖新会话；
  // 旧会话消息在 cleanup 中同步清空（set-state-in-effect 纪律：effect 体内不直接 setState）
  useEffect(() => {
    if (!backendRepo) return
    let stale = false
    loadConvs()
    fetch(`/api/repos/${backendRepo}/chat${convQ ? convQ + '&' : '?'}limit=50`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: { usage: { promptTokens: number; completionTokens: number }; hasMore: boolean; pendingApprovals?: PendingApproval[]; conversation?: { id: string; title: string | null }; messages: { id: number; role: string; content: string }[] } }) => {
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
  }, [backendRepo, convId, convQ, loadConvs, setConvId, setPendingApprovals])

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

  const exportAndClear = () => {
    exportChat()
    setTimeout(() => {
      if (!window.confirm('已导出。清空当前会话？（清空后不可恢复，视图文件不受影响）')) return
      reset()
    }, 400)
  }

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

  const upgradeToTask = () => {
    const draft = buildTaskDraftFromChat(messages, convId)
    if (draft) onCreateTask(draft)
  }

  const pendingCount = currentConv?.runtime.pendingConfirmations ?? 0

  // 选中模块（ContextChip 数据源；子模块选中落到其父模块）
  const selMod =
    selection?.kind === 'module'
      ? (map?.modules.find((m) => m.id === selection.id) ?? null)
      : selection?.kind === 'submodule'
        ? (map?.modules.find((m) => m.id === selection.parentId) ?? null)
        : null

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

      <QuickAskStream
        listRef={listRef}
        hasMore={hasMore}
        loadingMore={loadingMore}
        loadEarlier={loadEarlier}
        messages={messages}
        pendingApprovals={pendingApprovals}
        onGoWorkbench={onGoWorkbench}
        map={map}
        setInput={setInput}
        setClarify={setClarify}
        textareaRef={textareaRef}
        onLocateModule={onLocateModule}
        clarify={clarify}
        sending={sending}
        backendRepo={backendRepo}
      />

      <QuickAskComposer
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
