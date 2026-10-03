import { useState } from 'react'
import { Ban, Hammer, RotateCcw, Trash2, X } from 'lucide-react'
import { toast } from '@/lib/toast'
import { deleteTask, killTask, remediateTask, retryTask } from '@/lib/taskAdmin'

// 任务管理按钮组（2026-10-03 现状重审 P0 + 复审闭环）：按任务状态自动出现——
//   running              → 终止（两步确认；awaiting_approval 不在此列——它没在执行，
//                          正确出路是审批/打回/删除，kill 对无会话或已终态会话只会报错）
//   rejected             → 修改并复审（注入子 agent 审查意见直达实施重跑，完成自动复审）
//   failed / interrupted → 重试（服务端状态机白名单兜底，无需确认）
//   非 running            → 删除（两步确认；若真有活动会话后端会先杀后删）
// 两步确认走行内态（与 ViewsPanel 删除同风格），不用 window.confirm。

type ConfirmKind = 'kill' | 'delete'

export function TaskAdminButtons({
  repo,
  taskId,
  status,
  onDone,
  onDeleted,
}: {
  repo: string
  taskId: string
  status: string
  /** 操作成功后的本地回调（父组件一般再 load() 一次兜底） */
  onDone?: () => void
  /** 删除成功的专门回调——删后选中态要清掉（onDone 无法区分删与停） */
  onDeleted?: () => void
}) {
  const [confirming, setConfirming] = useState<ConfirmKind | null>(null)
  const [busy, setBusy] = useState(false)

  const run = async (kind: ConfirmKind | 'retry' | 'remediate') => {
    if (busy) return
    setBusy(true)
    try {
      if (kind === 'kill') await killTask(repo, taskId)
      else if (kind === 'retry') await retryTask(repo, taskId)
      else if (kind === 'remediate') await remediateTask(repo, taskId)
      else await deleteTask(repo, taskId)
      toast(
        kind === 'kill' ? '已终止执行'
          : kind === 'retry' ? '已重新入队'
          : kind === 'remediate' ? '已带审查意见进入修改复审——完成后自动复审'
          : '任务已删除',
      )
      setConfirming(null)
      if (kind === 'delete') onDeleted?.()
      onDone?.()
    } catch (e) {
      toast(e instanceof Error ? e.message : '操作失败', 'error')
    } finally {
      setBusy(false)
    }
  }

  const stoppable = status === 'running'
  const retryable = status === 'failed' || status === 'interrupted'
  // 2026-10-03 用户裁定：子 agent 审查打回（rejected）→ 修改并复审闭环
  const remediable = status === 'rejected'
  const deletable = status !== 'running' // pending/终态/awaiting_approval 都可删（后端会处理活动会话）

  if (confirming) {
    return (
      <span className="flex items-center gap-1" onClick={(e) => e.stopPropagation()}>
        <button
          onClick={() => void run(confirming)}
          disabled={busy}
          className={`rounded px-1.5 py-px text-[9px] font-bold text-white disabled:opacity-40 ${
            confirming === 'kill' ? 'bg-amber-500 hover:bg-amber-600' : 'bg-red-500 hover:bg-red-600'
          }`}
        >
          {busy ? '…' : confirming === 'kill' ? '确认终止' : '确认删除'}
        </button>
        <button onClick={() => setConfirming(null)} className="rounded p-px text-slate-400 hover:text-slate-600" title="取消">
          <X size={10} />
        </button>
      </span>
    )
  }

  return (
    <span className="flex items-center gap-0.5" onClick={(e) => e.stopPropagation()}>
      {stoppable && (
        <button
          onClick={() => setConfirming('kill')}
          title="终止执行（agent 进程被停止，可重试）"
          className="rounded p-0.5 text-slate-400 hover:bg-amber-50 hover:text-amber-600"
        >
          <Ban size={11} />
        </button>
      )}
      {remediable && (
        <button
          onClick={() => void run('remediate')}
          disabled={busy}
          title="修改并复审：注入子 agent 审查意见，直达实施阶段重跑，完成后自动复审"
          className="rounded p-0.5 text-slate-400 hover:bg-violet-50 hover:text-violet-600 disabled:opacity-40"
        >
          <Hammer size={11} />
        </button>
      )}
      {retryable && (
        <button
          onClick={() => void run('retry')}
          disabled={busy}
          title="就地重试（从失败处重新入队）"
          className="rounded p-0.5 text-slate-400 hover:bg-blue-50 hover:text-blue-600 disabled:opacity-40"
        >
          <RotateCcw size={11} />
        </button>
      )}
      {deletable && (
        <button
          onClick={() => setConfirming('delete')}
          title="删除该任务（running 请先终止）"
          className="rounded p-0.5 text-slate-400 hover:bg-red-50 hover:text-red-500"
        >
          <Trash2 size={11} />
        </button>
      )}
    </span>
  )
}
