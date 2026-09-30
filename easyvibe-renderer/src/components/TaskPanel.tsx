import { useCallback, useEffect, useState } from 'react'
import { Loader2, CheckCircle2, XCircle, Clock, ShieldCheck } from 'lucide-react'
import { onTaskEvent } from '@/lib/growthBus'

interface TaskResult {
  result?: { summary?: string; changed_modules?: string[] }
  diffStat?: string
  archivedPath?: string | null
  collectedAt?: string
}

interface TaskItem {
  id: string
  title: string
  description: string
  modules: string[]
  source: string
  status: string // pending/awaiting_approval/running/succeeded/failed/rejected/done
  trust: string
  gate?: string
  result?: TaskResult | null
  createdAt: string
}

interface Props {
  backendRepo: string | null
}

const GATE_LABEL: Record<string, string> = { plan: '① 计划审批', diff: '② Diff 审批', report: '③ 审查报告' }

const STATUS_STYLE: Record<string, { cls: string; label: string }> = {
  pending: { cls: 'bg-slate-100 text-slate-500', label: '排队中' },
  awaiting_approval: { cls: 'bg-amber-100 text-amber-700', label: '待审批' },
  running: { cls: 'bg-blue-100 text-blue-700', label: '执行中' },
  succeeded: { cls: 'bg-emerald-100 text-emerald-700', label: '执行成功' },
  failed: { cls: 'bg-red-100 text-red-700', label: '失败' },
  rejected: { cls: 'bg-red-100 text-red-600', label: '已驳回' },
  done: { cls: 'bg-emerald-100 text-emerald-700', label: '已完成' },
}

// 任务列表 + 审批操作（审批中心的数据面；Diff 可视化按审批中心原型是 M3-4 后续增强）
export function TaskPanel({ backendRepo }: Props) {
  const [tasks, setTasks] = useState<TaskItem[] | null>(null)
  const [loading, setLoading] = useState(false)
  const [deciding, setDeciding] = useState<string | null>(null)

  const load = useCallback(() => {
    if (!backendRepo) return
    setLoading(true)
    fetch(`/api/repos/${backendRepo}/tasks`)
      .then((r) => r.json())
      .then((d: { data: TaskItem[] }) => setTasks(d.data))
      .catch(() => setTasks([]))
      .finally(() => setLoading(false))
  }, [backendRepo])

  useEffect(() => {
    load()
  }, [load])

  // WS 任务事件驱动刷新
  useEffect(() => onTaskEvent(() => load()), [load])

  const decide = (id: string, decision: 'approved' | 'rejected') => {
    if (!backendRepo) return
    setDeciding(id + decision)
    fetch(`/api/repos/${backendRepo}/tasks/${id}/decide`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ decision }),
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        setTimeout(load, 500) // 等看门任务回写后再刷一次
        load()
      })
      .catch(() => alert('审批操作失败'))
      .finally(() => setDeciding(null))
  }

  return (
    <div className="space-y-3">
      <div>
        <h2 className="flex items-center gap-1.5 text-[15px] font-bold text-slate-800">
          <ShieldCheck size={15} className="text-blue-500" /> 任务与审批
        </h2>
        <p className="mt-0.5 text-[10.5px] text-slate-400">三道关：计划审批 → Diff 审批 → 审查报告；自动模式直通留痕</p>
      </div>

      {!backendRepo && <p className="py-6 text-center text-[11.5px] text-slate-400">需要本地后端在线</p>}
      {loading && !tasks && (
        <div className="flex items-center justify-center gap-2 py-8 text-[12px] text-slate-400">
          <Loader2 size={14} className="animate-spin" /> 加载任务…
        </div>
      )}
      {tasks?.length === 0 && <p className="py-6 text-center text-[11.5px] text-slate-400">暂无任务——从地图/问题/建议发起一个</p>}

      {tasks?.map((t) => {
        const st = STATUS_STYLE[t.status] ?? STATUS_STYLE.pending
        const needDecision = t.status === 'awaiting_approval' && t.gate && t.gate !== 'done' && t.gate !== 'rejected'
        return (
          <div key={t.id} className="rounded-lg border border-slate-200 p-3">
            <div className="flex items-center gap-2">
              <span className={`rounded-full px-2 py-px text-[9.5px] font-bold ${st.cls}`}>{st.label}</span>
              {t.status === 'awaiting_approval' && t.gate && (
                <span className="rounded-full bg-amber-50 px-2 py-px text-[9.5px] font-semibold text-amber-600">
                  {GATE_LABEL[t.gate] ?? t.gate}
                </span>
              )}
              <span className={`ml-auto rounded-full px-1.5 py-px text-[9px] ${t.trust === 'auto' ? 'bg-slate-100 text-slate-400' : 'bg-blue-50 text-blue-600'}`}>
                {t.trust === 'auto' ? '自动' : '手动'}
              </span>
            </div>
            <div className="mt-1.5 text-[12px] font-semibold leading-5 text-slate-800">{t.title}</div>
            <p className="mt-0.5 line-clamp-2 text-[10.5px] leading-4 text-slate-500">{t.description}</p>
            {/* M4-1：终态采集产物——agent 总结 + 变更摘要（diff 关审批的"改了什么"） */}
            {t.result && (
              <div className="mt-2 rounded-md border border-slate-100 bg-slate-50 p-2">
                {t.result.result?.summary && (
                  <p className="text-[10.5px] leading-4 text-slate-700">
                    <span className="font-semibold text-slate-500">agent 总结：</span>
                    {t.result.result.summary}
                  </p>
                )}
                {t.result.result?.changed_modules && t.result.result.changed_modules.length > 0 && (
                  <div className="mt-1 flex flex-wrap gap-1">
                    {t.result.result.changed_modules.map((m) => (
                      <span key={m} className="rounded-full bg-white px-1.5 py-px font-mono text-[9px] text-blue-600 shadow-sm">
                        {m}
                      </span>
                    ))}
                  </div>
                )}
                {t.result.diffStat && (
                  <details className="mt-1.5">
                    <summary className="cursor-pointer text-[10px] font-semibold text-slate-500 hover:text-slate-700">
                      变更摘要（git）
                    </summary>
                    <pre className="mt-1 max-h-36 overflow-auto whitespace-pre-wrap rounded bg-white p-1.5 font-mono text-[9.5px] leading-4 text-slate-600">
                      {t.result.diffStat}
                    </pre>
                  </details>
                )}
                {t.result.archivedPath && (
                  <p className="mt-1 truncate text-[9px] text-slate-400" title={t.result.archivedPath}>
                    已归档：{t.result.archivedPath}
                  </p>
                )}
              </div>
            )}
            {needDecision && (
              <div className="mt-2 flex gap-2">
                <button
                  onClick={() => decide(t.id, 'approved')}
                  disabled={deciding !== null}
                  className="flex flex-1 items-center justify-center gap-1 rounded-lg bg-blue-600 py-1.5 text-[11px] font-bold text-white hover:bg-blue-700 disabled:opacity-50"
                >
                  {deciding === t.id + 'approved' ? <Loader2 size={11} className="animate-spin" /> : <CheckCircle2 size={11} />}
                  通过
                </button>
                <button
                  onClick={() => decide(t.id, 'rejected')}
                  disabled={deciding !== null}
                  className="flex flex-1 items-center justify-center gap-1 rounded-lg border border-red-200 py-1.5 text-[11px] font-bold text-red-600 hover:bg-red-50 disabled:opacity-50"
                >
                  {deciding === t.id + 'rejected' ? <Loader2 size={11} className="animate-spin" /> : <XCircle size={11} />}
                  驳回
                </button>
              </div>
            )}
            {t.status === 'running' && (
              <p className="mt-1.5 flex items-center gap-1 text-[10px] text-blue-500">
                <Clock size={9} className="animate-pulse" /> agent 执行中，完成后进入下一关
              </p>
            )}
          </div>
        )
      })}
    </div>
  )
}
