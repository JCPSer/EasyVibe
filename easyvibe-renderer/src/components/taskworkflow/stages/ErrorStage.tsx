// 未启动/终态灰态：error + 重试/复制入口 + 删除 + 修改并复审。
// 拆自 TaskWorkflowPage.tsx（2026-10-05 防膨胀）。
import { Copy, Hammer, XCircle } from 'lucide-react'
import { toast } from '@/runtime/toast'
import { TaskAdminButtons } from '@/components/taskworkflow/TaskAdminButtons'
import { remediateTask } from '@/components/taskworkflow/taskAdmin'
import type { TaskDraft } from '@/shared/logic/taskContext'
import { statusWording, type TaskItem } from '../types'
import { useLang } from '@/runtime/i18n'

export function ErrorStage({ sel, backendRepo, onCreateTask, onReload, onClearedSelection }: {
  sel: TaskItem
  backendRepo: string
  onCreateTask: (d: TaskDraft) => void
  onReload: () => void
  onClearedSelection: () => void
}) {
  const { t } = useLang()
  return (
    <div className="m-4 rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 p-4">
      <p className="flex items-center gap-1 text-micro font-bold uppercase tracking-wider text-slate-400 dark:text-slate-500">
        <XCircle size={10} /> {statusWording(sel.status)}
      </p>
      {sel.error && <p className="mt-2 rounded-lg bg-red-50 dark:bg-red-950/40 px-3 py-2 text-[12px] leading-5 text-red-600">{sel.error}</p>}
      {/* 重审 P0：无 error 详情的终态卡不再是死胡同——把可走的路明说 */}
      {!sel.error && (
        <p className="mt-2 rounded-lg bg-slate-50 dark:bg-slate-950/70 px-3 py-2 text-[12px] leading-5 text-slate-500 dark:text-slate-400">
          {t('task.noErrorDetail')}
        </p>
      )}
      <div className="mt-3 flex flex-wrap items-center gap-2">
        {/* 修改并复审（用户裁定 2026-10-03）：子 agent 审查打回 → 带意见重跑实施，完成自动复审。
            比"复制为新任务"更优的路径——上下文/血缘不断裂 */}
        {sel.status === 'rejected' && (
          <button
            onClick={() => {
              remediateTask(backendRepo, sel.id)
                .then(() => {
                  toast(t('task.remediateToast'))
                  onReload()
                })
                .catch((e) => toast(e instanceof Error ? e.message : t('task.opFail'), 'error'))
            }}
            className="flex items-center gap-1 rounded-lg bg-violet-600 px-3 py-1.5 text-micro font-bold text-white transition-colors hover:bg-violet-700"
            title={t('task.remediateErrTip')}
          >
            <Hammer size={11} /> {t('task.remediate')}
          </button>
        )}
        {/* 管理按钮组（重审 P0 + 复审闭环）：终止/重试/删除——重试也可点上方标题栏的循环箭头 */}
        <TaskAdminButtons repo={backendRepo} taskId={sel.id} status={sel.status} onDone={onReload} onDeleted={() => { onClearedSelection(); onReload() }} />
        <button
          onClick={() =>
            onCreateTask({
              title: `${sel.title}${t('task.reworkSuffix')}`,
              description: sel.description,
              modules: sel.modules ?? [],
              acceptance: sel.acceptance ?? '',
              source: 'manual',
              context: { origin_task_id: sel.id },
            })
          }
          className="flex items-center gap-1 rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-1.5 text-micro font-semibold text-slate-500 dark:text-slate-400 transition-colors hover:border-blue-300 hover:text-blue-600"
        >
          <Copy size={9} /> {t('task.copyNew')}
        </button>
      </div>
    </div>
  )
}
