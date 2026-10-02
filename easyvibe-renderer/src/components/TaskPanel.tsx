import { useCallback, useEffect, useState } from 'react'
import { toast } from '@/lib/toast'
import { Loader2, CheckCircle2, XCircle, Clock, ShieldCheck, FileDiff, RefreshCw, Copy} from 'lucide-react'
import { onTaskEvent, onSessionOutput } from '@/lib/growthBus'
import type { TaskDraft } from '@/lib/taskContext'

interface TaskResult {
  result?: { summary?: string; changed_modules?: string[] } | null
  diffStat?: string
  archivedPath?: string | null
  collectedAt?: string
  warnings?: string[]
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
  sessionId?: string
  acceptance?: string
  result?: TaskResult | null
  createdAt: string
}

interface Props {
  backendRepo: string | null
  /** D3 拍板：驳回即终局 + "复制为新任务"（不重开旧任务，守一任务一条河） */
  onCreateTask?: (draft: TaskDraft) => void
  /** M4-1 真人测试建议#3：空态主行动（如"去地图看看"） */
  emptyAction?: { label: string; onClick: () => void }
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
export function TaskPanel({ backendRepo, onCreateTask, emptyAction }: Props) {
  const [tasks, setTasks] = useState<TaskItem[] | null>(null)
  const [loadError, setLoadError] = useState(false)
  const [loading, setLoading] = useState(false)
  const [deciding, setDeciding] = useState<string | null>(null)
  const [rejectingId, setRejectingId] = useState<string | null>(null)
  const [rejectReason, setRejectReason] = useState('')
  const [diffs, setDiffs] = useState<Record<string, string | null>>({})
  const [rechecking, setRechecking] = useState(false)
  // 失败可诊断：任务会话的输出流（含 [err] 行）——失败时能看到 agent 的死亡原因
  const [taskLines, setTaskLines] = useState<Record<string, string[]>>({})
  useEffect(
    () =>
      onSessionOutput((e) => {
        setTaskLines((prev) => {
          const cur = prev[e.sessionId] ?? []
          return { ...prev, [e.sessionId]: [...cur.slice(-3), e.line] }
        })
      }),
    [],
  )

  // 改进#7：plan 关的 flagged 风险理由（supervised 高危时由后端留痕）
  const [riskNotes, setRiskNotes] = useState<Record<string, string>>({})
  const [diffLoading, setDiffLoading] = useState<string | null>(null)

  const load = useCallback(() => {
    if (!backendRepo) return
    setLoading(true)
    fetch(`/api/repos/${backendRepo}/tasks`)
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status)) // R3 #8：5xx 时不得静默空白（四态失守）
        return r.json()
      })
      .then((d: { data: TaskItem[] }) => {
        setTasks(d.data)
        setLoadError(false)
      })
      // R4 清债：加载失败不再静默成"暂无任务"（四态齐全，§0 标准）
      .catch(() => {
        setTasks([])
        setLoadError(true)
      })
      .finally(() => setLoading(false))
  }, [backendRepo])

  useEffect(() => {
    load()
  }, [load])

  // WS 任务事件驱动刷新
  useEffect(() => onTaskEvent(() => load()), [load])

  const decide = (id: string, decision: 'approved' | 'rejected', note?: string) => {
    if (!backendRepo) return
    setDeciding(id + decision)
    fetch(`/api/repos/${backendRepo}/tasks/${id}/decide`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ decision, note }),
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        setRejectingId(null)
        setRejectReason('')
        setTimeout(load, 500) // 等看门任务回写后再刷一次
        load()
      })
      .catch(() => toast(decision === 'rejected' ? '驳回失败（理由必填）' : '审批操作失败', 'error'))
      .finally(() => setDeciding(null))
  }

  // S2 复检闭环：任务 done 且 agent 声明了改动模块 → 一键触发巡检，
  // 该模块健康分变化会在详情页趋势条中显现（任务成功 ≠ 架构变好，复检让闭环成环）
  const recheck = () => {
    if (!backendRepo || rechecking) return
    setRechecking(true)
    fetch(`/api/repos/${backendRepo}/patrol`, { method: 'POST' })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
      })
      .catch(() => toast('触发巡检失败（需要本地后端在线）', 'error'))
      .finally(() => setTimeout(() => setRechecking(false), 3000))
  }

  // M4-3：完整 diff 按需懒加载（不随任务列表载荷；归档文件按任务读取）
  const loadDiff = (id: string) => {
    if (!backendRepo || id in diffs || diffLoading === id) return
    setDiffLoading(id)
    fetch(`/api/repos/${backendRepo}/tasks/${id}/diff`)
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.json()
      })
      .then((d: { data: { diff: string | null } }) => {
        setDiffs((prev) => ({ ...prev, [id]: d.data.diff ?? null }))
      })
      .catch(() => setDiffs((prev) => ({ ...prev, [id]: null })))
      .finally(() => setDiffLoading(null))
  }

  return (
    <div className="space-y-3">
      <div>
        <h2 className="flex items-center gap-1.5 text-[15px] font-bold text-slate-800">
          <ShieldCheck size={15} className="text-blue-500" /> 任务与审批
        </h2>
        <p className="mt-0.5 text-cap text-slate-400">三道关：计划审批 → Diff 审批 → 审查报告；自动模式直通留痕</p>
      </div>

      {!backendRepo && <p className="py-6 text-center text-[12px] text-slate-400">需要本地后端在线</p>}
      {loading && !tasks && (
        <div className="flex items-center justify-center gap-2 py-8 text-[12px] text-slate-400">
          <Loader2 size={14} className="animate-spin" /> 加载任务…
        </div>
      )}
      {/* 空态 = 说明 + 主行动（验收清单⑪） */}
      {tasks?.length === 0 && !loadError && (
        <div className="flex flex-col items-center gap-2 py-6">
          <p className="text-[12px] text-slate-400">暂无任务——从地图/问题/建议发起一个</p>
          {emptyAction && (
            <button
              onClick={emptyAction.onClick}
              className="rounded-lg border border-slate-200 bg-white px-3 py-1.5 text-[11px] font-semibold text-slate-600 hover:bg-slate-50"
            >
              {emptyAction.label}
            </button>
          )}
        </div>
      )}
      {tasks?.length === 0 && loadError && (
        <div className="py-6 text-center">
          <p className="text-[12px] text-red-500">任务列表加载失败（需要本地后端在线）</p>
          <button onClick={load} className="mt-2 rounded-lg border border-slate-200 px-3 py-1 text-[11px] text-slate-600 hover:bg-slate-50">
            重试
          </button>
        </div>
      )}

      {tasks?.map((t) => {
        const st = STATUS_STYLE[t.status] ?? STATUS_STYLE.pending
        const needDecision = t.status === 'awaiting_approval' && t.gate && t.gate !== 'done' && t.gate !== 'rejected'
        return (
          <div key={t.id} className="rounded-lg border border-slate-200 p-3">
            <div className="flex items-center gap-2">
              <span className={`rounded-full px-2 py-px text-micro font-bold ${st.cls}`}>{st.label}</span>
              {t.status === 'awaiting_approval' && t.gate && (
                <span className="rounded-full bg-amber-50 px-2 py-px text-micro font-semibold text-amber-600">
                  {GATE_LABEL[t.gate] ?? t.gate}
                </span>
              )}
              <span className={`ml-auto rounded-full px-1.5 py-px text-micro ${t.trust === 'auto' ? 'bg-slate-100 text-slate-400' : 'bg-blue-50 text-blue-600'}`}>
                {t.trust === 'auto' ? '自动' : '手动'}
              </span>
            </div>
            <div className="mt-1.5 text-[12px] font-semibold leading-5 text-slate-800">{t.title}</div>
            {/* S1-2 计划关不盲批：第一道关展示完整需求（不截断）+ 验收标准 */}
            <p className={`mt-0.5 text-cap leading-4 text-slate-500 ${t.gate === 'plan' ? '' : 'line-clamp-2'}`}>
              {t.description}
            </p>
            {t.gate === 'plan' && t.status === 'awaiting_approval' && !riskNotes[t.id] && (
              <RiskNoteLoader taskId={t.id} backendRepo={backendRepo} onLoaded={(note) => setRiskNotes((p) => ({ ...p, [t.id]: note }))} />
            )}
            {t.gate === 'plan' && riskNotes[t.id] && (
              <p className="mt-1 rounded bg-amber-50 px-1.5 py-0.5 text-micro leading-4 text-amber-700">
                ⚠ {riskNotes[t.id]}
              </p>
            )}
            {t.gate === 'plan' && t.acceptance && (
              <p className="mt-1 rounded bg-slate-50 px-1.5 py-0.5 text-micro leading-4 text-slate-600">
                <span className="font-semibold text-slate-500">验收标准：</span>
                {t.acceptance}
              </p>
            )}
            {t.status === 'rejected' && onCreateTask && (
              <button
                onClick={() =>
                  onCreateTask({
                    title: `${t.title}（重提）`,
                    description: t.description,
                    modules: t.modules ?? [],
                    acceptance: t.acceptance ?? '',
                    source: 'manual',
                    context: {},
                  })
                }
                className="mt-1.5 flex items-center gap-1 rounded-lg border border-slate-200 px-2 py-1 text-micro font-semibold text-slate-500 hover:bg-slate-50"
                title="以本任务为模板创建新任务（原驳回记录保留，D3 拍板）"
              >
                <Copy size={9} /> 复制为新任务
              </button>
            )}
            {/* M4-1：终态采集产物——agent 总结 + 变更摘要（diff 关审批的"改了什么"） */}
            {t.result && (
              <div className="mt-2 rounded-md border border-slate-100 bg-slate-50 p-2">
                {/* 实弹#3 防线：agent 未按协议产出 RESULT 行——审批人须警惕空执行/归因错位 */}
                {t.result.warnings?.map((w, i) => (
                  <p key={i} className="mb-1 rounded bg-amber-50 px-1.5 py-0.5 text-micro leading-4 text-amber-700">
                    ⚠ {w}
                  </p>
                ))}
                {t.result.result?.summary && (
                  <p className="text-cap leading-4 text-slate-700">
                    <span className="font-semibold text-slate-500">agent 总结：</span>
                    {t.result.result.summary}
                  </p>
                )}
                {t.result.result?.changed_modules && t.result.result.changed_modules.length > 0 && (
                  <div className="mt-1 flex flex-wrap gap-1">
                    {t.result.result.changed_modules.map((m) => (
                      <span key={m} className="rounded-full bg-white px-1.5 py-px font-mono text-micro text-blue-600 shadow-sm">
                        {m}
                      </span>
                    ))}
                  </div>
                )}
                {t.result.diffStat && (
                  <details className="mt-1.5">
                    <summary className="cursor-pointer text-micro font-semibold text-slate-500 hover:text-slate-700">
                      变更摘要（git）
                    </summary>
                    <pre className="mt-1 max-h-36 overflow-auto whitespace-pre-wrap rounded bg-white p-1.5 font-mono text-micro leading-4 text-slate-600">
                      {t.result.diffStat}
                    </pre>
                  </details>
                )}
                {/* M4-3：完整 diff 懒加载（按需读取归档，256KB 封顶在采集侧） */}
                {t.result.archivedPath && (
                  <details
                    className="mt-1.5"
                    onToggle={(e) => {
                      if ((e.target as HTMLDetailsElement).open) loadDiff(t.id)
                    }}
                  >
                    <summary className="cursor-pointer text-micro font-semibold text-slate-500 hover:text-slate-700">
                      <FileDiff size={9} className="mr-0.5 inline" />
                      完整 diff
                    </summary>
                    {diffLoading === t.id ? (
                      <p className="mt-1 flex items-center gap-1 text-micro text-slate-400">
                        <Loader2 size={10} className="animate-spin" /> 读取中…
                      </p>
                    ) : t.id in diffs ? (
                      diffs[t.id] ? (
                        <pre className="mt-1 max-h-72 overflow-auto whitespace-pre-wrap rounded bg-slate-800 p-2 font-mono text-micro leading-4 text-emerald-200">
                          {diffs[t.id]}
                        </pre>
                      ) : (
                        <p className="mt-1 text-micro text-slate-400">（无变更或未采集到 diff）</p>
                      )
                    ) : null}
                  </details>
                )}
                {t.result.archivedPath && (
                  <p className="mt-1 truncate text-micro text-slate-400" title={t.result.archivedPath}>
                    已归档：{t.result.archivedPath}
                  </p>
                )}
                {t.status === 'done' && t.result.result?.changed_modules && t.result.result.changed_modules.length > 0 && (
                  <button
                    onClick={recheck}
                    disabled={rechecking}
                    className="mt-1.5 flex items-center gap-1 rounded-lg border border-emerald-200 bg-emerald-50 px-2 py-1 text-micro font-semibold text-emerald-700 hover:bg-emerald-100 disabled:opacity-50"
                    title="任务成功 ≠ 架构变好：触发一次巡检，改动模块的健康变化将在详情页趋势条中可见"
                  >
                    <RefreshCw size={9} className={rechecking ? 'animate-spin' : ''} />
                    {rechecking ? '巡检中…（稍后看详情页趋势条）' : '重新巡检验证改动效果'}
                  </button>
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
                {rejectingId === t.id ? (
                  <button
                    onClick={() => decide(t.id, 'rejected', rejectReason.trim())}
                    disabled={deciding !== null || !rejectReason.trim()}
                    className="flex flex-1 items-center justify-center gap-1 rounded-lg bg-red-600 py-1.5 text-[11px] font-bold text-white hover:bg-red-700 disabled:opacity-40"
                    title={rejectReason.trim() ? '' : '驳回必须填写理由'}
                  >
                    {deciding === t.id + 'rejected' ? <Loader2 size={11} className="animate-spin" /> : <XCircle size={11} />}
                    确认驳回
                  </button>
                ) : (
                  <button
                    onClick={() => {
                      setRejectingId(t.id)
                      setRejectReason('')
                    }}
                    disabled={deciding !== null}
                    className="flex flex-1 items-center justify-center gap-1 rounded-lg border border-red-200 py-1.5 text-[11px] font-bold text-red-600 hover:bg-red-50 disabled:opacity-50"
                  >
                    <XCircle size={11} />
                    驳回
                  </button>
                )}
              </div>
            )}
            {/* M4-2：驳回理由输入（必填，后端 400 兜底） */}
            {rejectingId === t.id && (
              <div className="mt-2">
                <textarea
                  value={rejectReason}
                  onChange={(e) => setRejectReason(e.target.value)}
                  rows={2}
                  autoFocus
                  placeholder="驳回理由（必填，留痕可追溯）…"
                  className="w-full resize-none rounded-lg border border-red-200 bg-red-50/40 px-2.5 py-1.5 text-[11px] leading-4 text-slate-700 outline-none focus:border-red-300"
                />
                <div className="mt-1 flex justify-end gap-2">
                  <button
                    onClick={() => {
                      setRejectingId(null)
                      setRejectReason('')
                    }}
                    className="rounded-full px-2 py-0.5 text-micro text-slate-400 hover:text-slate-600"
                  >
                    取消
                  </button>
                </div>
              </div>
            )}
            {(t.status === 'running' || t.status === 'pending') && (
              <div className="mt-1.5">
                <p className="flex items-center gap-1 text-micro text-blue-500">
                  <Clock size={9} className="animate-pulse" /> agent 执行中，完成后进入下一关
                </p>
                {t.sessionId && taskLines[t.sessionId]?.filter((l) => l.startsWith('[err]')).slice(-1).map((l, i) => (
                  <p key={i} className="mt-0.5 truncate font-mono text-micro text-red-500">{l}</p>
                ))}
              </div>
            )}
          </div>
        )
      })}
    </div>
  )
}

// 改进#7：拉取 plan 关 flagged 留痕（supervised 风险预评估理由），无则静默
function RiskNoteLoader({ taskId, backendRepo, onLoaded }: { taskId: string; backendRepo: string | null; onLoaded: (note: string) => void }) {
  useEffect(() => {
    if (!backendRepo) return
    fetch(`/api/repos/${backendRepo}/tasks/${taskId}/approvals`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: { decision: string; note?: string }[] }) => {
        const flagged = d.data.find((a) => a.decision === 'flagged' && a.note)
        if (flagged?.note) onLoaded(flagged.note)
      })
      .catch(() => {})
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [taskId, backendRepo])
  return null
}
