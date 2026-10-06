// 多会话状态与 CRUD（ChatPanel / QuickAsk 共用；对话消息流由各自壳持有）。
// 拆自 ChatPanel.tsx / QuickAsk.tsx（2026-10-05 防膨胀）。
import { useCallback, useEffect, useState } from 'react'
import { toast } from '@/runtime/toast'
import { onTaskEvent } from '@/runtime/growthBus'
import { chatHistory, conversations as fetchConversations, createConversation, deleteConversation, renameConversation } from '@/api/chat'
import type { ChatRestore, ConversationSummary, PendingApproval } from './types'

export function useConversations({
  backendRepo, activeConvId, onConvChange, defaultConvTitle, onCreated,
}: {
  backendRepo: string | null
  /** 受控会话 id（工作台左栏驱动）。undefined = 内部自治 */
  activeConvId?: string | null
  onConvChange?: (id: string | null) => void
  defaultConvTitle?: string | null
  /** 新建会话成功后的附加动作（如收起「…」菜单） */
  onCreated?: () => void
}) {
  const [convs, setConvs] = useState<ConversationSummary[]>([])
  const [internalConvId, setInternalConvId] = useState<string | null>(null)
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

  const convQ = convId ? `?conv=${encodeURIComponent(convId)}` : ''

  const loadConvs = useCallback(() => {
    if (!backendRepo) return
    fetchConversations(backendRepo)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: ConversationSummary[] } | null) => {
        if (d?.data) setConvs(d.data)
      })
      .catch(() => {})
  }, [backendRepo])

  const refreshPending = useCallback(() => {
    if (!backendRepo) return
    chatHistory(backendRepo, convId ? `?conv=${encodeURIComponent(convId)}&limit=1` : '?limit=1')
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
    createConversation(backendRepo, defaultConvTitle ? { title: defaultConvTitle } : {})
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: ConversationSummary }) => {
        setConvId(d.data.id)
        setConvMenuOpen(false)
        onCreated?.()
        loadConvs()
      })
      .catch(() => toast('新建会话失败', 'error'))
  }, [backendRepo, loadConvs, setConvId, defaultConvTitle, onCreated])

  const renameConv = useCallback(() => {
    if (!backendRepo || !convId || !renameVal.trim()) return
    renameConversation(backendRepo, convId, renameVal.trim())
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
    deleteConversation(backendRepo, convId)
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        setConvId(null)
        loadConvs()
      })
      .catch((e) => toast(String(e).includes('400') ? '每个仓库至少保留一个会话' : '删除失败', 'error'))
  }, [backendRepo, convId, loadConvs, setConvId])

  const currentConv = convs.find((c) => c.id === convId)
  const displayTitle = currentConv?.title ?? (convId ? '会话' : '默认会话')

  return {
    convs, convId, setConvId, convQ, convMenuOpen, setConvMenuOpen,
    renaming, setRenaming, renameVal, setRenameVal, pendingApprovals, setPendingApprovals,
    loadConvs, refreshPending, createConv, renameConv, deleteConv, currentConv, displayTitle,
  }
}
