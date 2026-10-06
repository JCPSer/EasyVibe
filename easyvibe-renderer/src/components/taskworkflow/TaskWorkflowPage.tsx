import { useCallback, useEffect, useMemo, useState } from 'react'
import { History, Loader2, ShieldAlert } from 'lucide-react'
import { toast } from '@/runtime/toast'
import { onTaskEvent } from '@/runtime/growthBus'
import { stageOf, gateLabel, STAGES } from '@/components/taskworkflow/taskStage'
import { rewindTask } from '@/components/taskworkflow/taskAdmin'
import { decideTask, devDocs, listTasks, taskApprovals, taskDiff } from '@/api/task'
import { StagePipeline } from '@/components/taskworkflow/StagePipeline'
import { TaskAdminButtons } from '@/components/taskworkflow/TaskAdminButtons'
import type { CodeMap } from '@/types/map'
import type { TaskDraft } from '@/shared/logic/taskContext'
import { parseImpact, taskDuration } from './diffParse'
import { STATUS_LABEL, type Approval, type DevDoc, type TaskItem } from './types'
import { DocCard } from './DocCard'
import { PhaseDocReview } from './PhaseDocReview'
import { StageLookback } from './StageLookback'
import { AnalysisStage } from './stages/AnalysisStage'
import { TerminalStage } from './stages/TerminalStage'
import { DiffStage } from './stages/DiffStage'
import { ReportStage } from './stages/ReportStage'
import { DoneStage } from './stages/DoneStage'
import { ErrorStage } from './stages/ErrorStage'

// 任务工作流页（方案 v3 §4.2 施工 + 2026-10-03 分阶段流扩展）：
// 五阶段管道头 + 阶段单态主区。本文件为壳：状态装配 + 列表/头部编排 + 阶段调度；
// 子组件与纯函数见 ./taskworkflow/*（2026-10-05 防膨胀拆分，行为零改动）。

export function TaskWorkflowPage({
  backendRepo,
  map,
  onCreateTask,
  focusTask,
  onOpenRuns,
}: {
  backendRepo: string | null
  map: CodeMap
  /** 打回/失败 → 复制为新任务（origin_task_id 血缘，D3 拍板语义） */
  onCreateTask: (d: TaskDraft) => void
  /** v4 修订：看板点卡跳入——按 nonce 选中对应任务（首次挂载也生效） */
  focusTask?: { id: string; nonce: number } | null
  /** 2026-10-05 M4：本任务会话 → 运行页看完整流水（终端只留 200 行） */
  onOpenRuns?: (sessionId: string) => void
}) {
  const [tasks, setTasks] = useState<TaskItem[] | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  // 看板点卡跳入：nonce 变化即选中（含组件常驻后的每次跳入）
  useEffect(() => {
    if (focusTask?.id) setSelected(focusTask.id)
  }, [focusTask])
  // 详情快照（taskId 归属防过期响应，ChangesPage 同模式）
  const [detailFor, setDetailFor] = useState<{ taskId: string; approvals: Approval[]; diffFull: string | null; diffStat: string | null; docs: DevDoc[]; docsAt: number } | null>(null)
  /** diff 关当前任务键（fileSel 归属防串台；fileSel 状态由 DiffStage 自持） */
  const selKey = selected
  const [deciding, setDeciding] = useState<string | null>(null)
  const [rejectNote, setRejectNote] = useState('')
  const [rejecting, setRejecting] = useState(false)
  const load = useCallback(() => {
    if (!backendRepo) return
    listTasks(backendRepo)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: TaskItem[] } | null) => {
        if (!d?.data) return
        const rank = (t: TaskItem) => (t.status === 'awaiting_approval' ? 0 : t.status === 'running' ? 1 : 2)
        setTasks([...d.data].sort((a, b) => rank(a) - rank(b)))
      })
      .catch(() => {})
  }, [backendRepo])

  // 2026-10-04 审计 P1：产物文档删除后刷新（docsTick 驱动上面的拉取 effect 重跑）
  const [docsTick, setDocsTick] = useState(0)

  useEffect(() => {
    load()
  }, [load])
  useEffect(() => onTaskEvent(load), [load])

  const sel = tasks?.find((t) => t.id === selected) ?? null

  useEffect(() => {
    if (!backendRepo || !sel) return
    const tid = sel.id
    const apply = (patch: Partial<NonNullable<typeof detailFor>>) =>
      setDetailFor((prev) => (prev && prev.taskId === tid ? { ...prev, ...patch } : { taskId: tid, approvals: [], diffFull: null, diffStat: null, docs: [], docsAt: 0, ...patch }))
    taskApprovals(backendRepo, tid)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: Approval[] } | null) => apply({ approvals: d?.data ?? [] }))
      .catch(() => {})
    taskDiff(backendRepo, tid)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { diff?: string | null; diffStat?: string | null } } | null) =>
        apply({ diffFull: d?.data?.diff ?? null, diffStat: d?.data?.diffStat ?? null }),
      )
      .catch(() => {})
    // 产物文档（方案 v3 §4.4）：终态/审批关才拉，running 期文档窗随 now() 扩张——
    // 带 5s 最小间隔，轮询节拍复用终端 tick
    devDocs(backendRepo, tid)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { docs?: DevDoc[] } } | null) => apply({ docs: d?.data?.docs ?? [], docsAt: Date.now() }))
      .catch(() => {})
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [backendRepo, sel?.id, sel?.status, sel?.gate, docsTick])
  const detail = sel && detailFor?.taskId === sel.id ? detailFor : null
  const approvals = detail?.approvals ?? []
  const diffFull = detail?.diffFull ?? null
  const diffStat = detail?.diffStat ?? null
  const docs = detail?.docs ?? []
  const reviewTrail = useMemo(() => {
    const GLABEL: Record<string, string> = { plan: '任务书', analysis: '需求矩阵', solution: '方案', diff: 'Diff', report: '报告' }
    const DLABEL: Record<string, string> = { approved: '通过', rejected: '打回', skipped: '自动通过', flagged: '风险预评', rewind: '回退' }
    return [...approvals]
      .sort((a, b) => String(a.decidedAt).localeCompare(String(b.decidedAt)))
      .map((a) => `${GLABEL[a.gate] ?? a.gate}关${DLABEL[a.decision] ?? a.decision}${a.note ? `：${a.note}` : ''}`)
  }, [approvals])
  const impact = useMemo(() => parseImpact(diffStat), [diffStat])
  const isRunning = sel?.status === 'running'
  const decide = (decision: 'approved' | 'rejected', noteArg?: string) => {
    if (!backendRepo || !sel || deciding) return
    const note = noteArg ?? rejectNote
    if (decision === 'rejected' && !note.trim()) {
      toast('打回必须填写意见', 'error')
      return
    }
    setDeciding(decision)
    decideTask(backendRepo, sel.id, { decision, note: decision === 'rejected' ? note.trim() : undefined, gate: sel.gate })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        toast(decision === 'approved' ? '已通过' : '已打回')
        // diff 关通过只推进 report、卡留列——把真实状态说给用户（复审口径）
        if (decision === 'approved' && sel.gate === 'diff') toast('已通过 Diff 审批，还差终审（审查报告）', 'info')
        setRejecting(false)
        setRejectNote('')
        load()
      })
      .catch(() => toast('审批操作失败', 'error'))
      .finally(() => setDeciding(null))
  }
  // 管道回看（2026-10-05 方案 §3.2）：null = 跟随当前阶段；数值 = 回看该历史阶段。
  // 必须位于下方 early return 之前（hooks 序纪律——白屏战役的同款教训）。
  const stage = sel ? stageOf(sel.status, sel.gate) : null
  const [viewStage, setViewStage] = useState<number | null>(null)
  const [rewinding, setRewinding] = useState<'analysis' | 'solution' | null>(null)
  // 当前阶段索引（done 视为 5——全部可回看）；error 灰态不可回看
  const stageIdx: number | null = stage === 'done' ? STAGES.length : typeof stage === 'number' ? stage : null
  const viewing = viewStage !== null && stageIdx !== null && viewStage < stageIdx
  // 阶段推进/换任务即退出回看（评审#S4：否则回看内容与新阶段脱节）
  useEffect(() => {
    setViewStage(null)
  }, [selKey, stageIdx])
  const doRewind = (gate: 'analysis' | 'solution') => {
    if (!backendRepo || !sel || rewinding) return
    setRewinding(gate)
    rewindTask(backendRepo, sel.id, gate)
      .then(() => {
        toast(`已回到${gate === 'analysis' ? '需求分析' : '方案设计'}评审关——可打回（带意见重跑本阶段）或通过继续`, 'info')
        setViewStage(null)
        load()
      })
      .catch((e) => toast(e instanceof Error ? e.message : '回退失败', 'error'))
      .finally(() => setRewinding(null))
  }
  if (!backendRepo) {
    return <p className="p-8 text-center text-[12px] text-slate-400 dark:text-slate-500">需要本地后端在线</p>
  }
  if (tasks === null) {
    return (
      <div className="flex h-full items-center justify-center gap-2 text-[12px] text-slate-400 dark:text-slate-500">
        <Loader2 size={14} className="animate-spin" /> 加载任务…
      </div>
    )
  }

  const pendingCount = tasks.filter((t) => t.status === 'awaiting_approval').length
  return (
    <div className="flex h-full">
      {/* 左列：任务列表 */}
      <aside className="flex w-72 shrink-0 flex-col border-r border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
        <div className="border-b border-slate-100 dark:border-slate-800 px-3 py-2.5">
          <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">
            任务列表
            {pendingCount > 0 && (
              <span className="tnum ml-1.5 rounded-full bg-red-500 px-1.5 text-micro font-bold leading-4 text-white">{pendingCount}</span>
            )}
          </span>
          <p className="mt-0.5 text-micro text-slate-400 dark:text-slate-500">待你审批的任务在前</p>
        </div>
        <div className="min-h-0 flex-1 space-y-0.5 overflow-y-auto p-1.5">
          {tasks.map((t) => (
            <button
              key={t.id}
              onClick={() => setSelected(t.id)}
              className={`w-full rounded-lg px-2.5 py-2 text-left ${
                t.id === selected ? 'bg-blue-50 dark:bg-blue-950/40 ring-1 ring-blue-200' : 'hover:bg-slate-50 dark:hover:bg-slate-800/70'
              }`}
            >
              <div className="flex items-center gap-1.5">
                <span className={`min-w-0 flex-1 truncate text-[12px] font-semibold ${t.id === selected ? 'text-blue-700' : 'text-slate-700 dark:text-slate-200'}`}>
                  {t.title}
                </span>
                {t.status === 'awaiting_approval' && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-amber-500" />}
              </div>
              <p className="mt-0.5 flex items-center gap-1.5 text-micro text-slate-400 dark:text-slate-500">
                <span>{STATUS_LABEL[t.status] ?? t.status}</span>
                {gateLabel(t.status, t.gate) && (
                  <span className="rounded-full bg-slate-100 dark:bg-slate-800 px-1.5">{gateLabel(t.status, t.gate)}</span>
                )}
                <span className="tnum">{t.trust === 'auto' ? '自动' : t.trust === 'supervised' ? '监督' : '手动'}</span>
              </p>
            </button>
          ))}
          {tasks.length === 0 && <p className="px-2 py-6 text-center text-[11px] text-slate-400 dark:text-slate-500">暂无任务——从地图/问题/建议发起一个</p>}
        </div>
      </aside>

      {/* 右列：工作流主体 */}
      <div className="flex min-w-0 flex-1 flex-col">
        {!sel || stage === null ? (
          <p className="flex flex-1 items-center justify-center text-[12px] text-slate-400 dark:text-slate-500">选择左侧任务查看工作流</p>
        ) : (
          <>
            {/* 头部：标题 + 五阶段管道 */}
            <div className="border-b border-slate-100 dark:border-slate-800 bg-white dark:bg-slate-900 px-4 py-3">
              <div className="flex items-center gap-2">
                <h2 className="min-w-0 flex-1 truncate text-[14px] font-bold text-slate-800 dark:text-slate-100">{sel.title}</h2>
                <span
                  className={`shrink-0 rounded-full px-2 py-0.5 text-micro font-bold ${
                    sel.status === 'awaiting_approval' ? 'bg-amber-100 text-amber-700' : sel.status === 'running' ? 'bg-blue-50 dark:bg-blue-950/40 text-blue-600' : sel.status === 'done' ? 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-600' : 'bg-slate-100 dark:bg-slate-800 text-slate-500 dark:text-slate-400'
                  }`}
                >
                  {STATUS_LABEL[sel.status] ?? sel.status}
                </span>
                <span className="tnum shrink-0 rounded-full bg-slate-100 dark:bg-slate-800 px-2 py-0.5 text-micro font-semibold text-slate-500 dark:text-slate-400">{taskDuration(sel)}</span>
                {/* 管理三操作（重审 P0）：终止（活动）/ 重试（失败·中断）/ 删除（非运行） */}
                <TaskAdminButtons
                  repo={backendRepo}
                  taskId={sel.id}
                  status={sel.status}
                  onDone={load}
                  onDeleted={() => {
                    setSelected(null)
                    load()
                  }}
                />
              </div>
              {/* 五阶段管道：公共 StagePipeline（detail 档）——判定收敛到 taskStage 一处；
                  2026-10-05 管道回看：当前及之前的阶段可点击回看产物 */}
              <div className="mt-3">
                <StagePipeline
                  status={sel.status}
                  gate={sel.gate}
                  variant="detail"
                  selected={viewing ? viewStage! : undefined}
                  onSelectStage={(i) => setViewStage(i === stageIdx ? null : i)}
                />
              </div>
              {/* 回看横幅：明示只读 + 一键回到当前进度 */}
              {viewing && (
                <div className="mt-2 flex items-center gap-2 rounded-lg border border-violet-200 dark:border-violet-900/60 bg-violet-50 dark:bg-violet-950/40 px-3 py-1.5">
                  <History size={11} className="shrink-0 text-violet-500" />
                  <p className="min-w-0 flex-1 truncate text-micro text-violet-700 dark:text-violet-300">
                    正在回看「{STAGES[viewStage!]?.label}」——只读，不影响任务状态
                  </p>
                  <button
                    onClick={() => setViewStage(null)}
                    className="shrink-0 rounded-md border border-violet-200 dark:border-violet-800 px-2 py-0.5 text-micro font-semibold text-violet-600 hover:bg-violet-100 dark:hover:bg-violet-900/40"
                  >
                    回到当前进度
                  </button>
                </div>
              )}
              {/* 合约红线：审批必见 */}
              {(sel.result?.contractViolations?.length ?? 0) > 0 && (
                <div className="mt-2 rounded-lg border border-red-200 dark:border-red-900/60 bg-red-50 dark:bg-red-950/40 px-3 py-2">
                  <p className="flex items-center gap-1 text-micro font-bold text-red-600">
                    <ShieldAlert size={10} /> 影响面合约：{sel.result!.contractViolations!.length} 个文件越出声明边界
                  </p>
                  <ul className="mt-1 space-y-0.5">
                    {sel.result!.contractViolations!.slice(0, 5).map((v) => (
                      <li key={v} className="mono truncate text-micro text-red-500">{v}</li>
                    ))}
                  </ul>
                </div>
              )}
              {/* 评审留痕：迭代到通过的全过程 */}
              {reviewTrail.length > 0 && (
                <p className="mt-2 truncate text-[10px] leading-4 text-slate-400 dark:text-slate-500">
                  <span className="font-bold text-slate-500 dark:text-slate-400">评审轮回：</span>
                  {reviewTrail.map((r, i) => (
                    <span key={i}>{i > 0 && ' → '}{r}</span>
                  ))}
                </p>
              )}
            </div>
            {/* 主区：阶段单态切换（同一时间只有一个阶段是 now）；
                2026-10-05 管道回看：viewing 时整列换成历史阶段只读视图 */}
            <div className="flex min-h-0 flex-1">
              <div className="flex min-w-0 flex-1 flex-col">
                {viewing && viewStage !== null ? (
                  <StageLookback
                    backendRepo={backendRepo}
                    task={sel}
                    look={viewStage}
                    impact={impact}
                    onRewind={doRewind}
                    rewinding={rewinding}
                  />
                ) : (
                  <>
                {/* ① 需求分析·任务书待批 */}
                {stage === 0 && sel.gate === 'plan' && !isRunning && (
                  <AnalysisStage
                    sel={sel}
                    map={map}
                    approvals={approvals}
                    deciding={deciding}
                    rejecting={rejecting}
                    rejectNote={rejectNote}
                    setRejectNote={setRejectNote}
                    setRejecting={setRejecting}
                    onDecide={decide}
                  />
                )}

                {/* ① analysis 关：需求矩阵全文评审 */}
                {stage === 0 && sel.gate === 'analysis' && backendRepo && (
                  <PhaseDocReview
                    key={sel.id}
                    backendRepo={backendRepo}
                    taskId={sel.id}
                    dirHint="1_requirements_matrix"
                    title="需求矩阵评审"
                    deciding={deciding}
                    review={sel.result?.phaseReviews?.analysis ?? null}
                    onDecide={(d, note) => decide(d, note)}
                  />
                )}

                {/* ② solution 关：方案设计全文评审 */}
                {stage === 1 && sel.gate === 'solution' && backendRepo && (
                  <PhaseDocReview
                    key={sel.id}
                    backendRepo={backendRepo}
                    taskId={sel.id}
                    dirHint="2_requirements_solutions"
                    title="方案设计评审"
                    deciding={deciding}
                    review={sel.result?.phaseReviews?.solution ?? null}
                    compareDirHint="1_requirements_matrix"
                    compareTitle="需求矩阵（已评审）"
                    onDecide={(d, note) => decide(d, note)}
                  />
                )}

                {/* ③ 实施（及 ①② 产文档期间）：实时终端 */}
                {(stage === 2 || isRunning) && <TerminalStage sel={sel} repo={backendRepo} onOpenRuns={onOpenRuns} />}

                {/* ④ diff 关：双栏查看器 + 审查-修复闭环 + 裁决 */}
                {stage === 3 && (
                  <DiffStage
                    backendRepo={backendRepo}
                    sel={sel}
                    selKey={selKey}
                    diffFull={diffFull}
                    deciding={deciding}
                    rejecting={rejecting}
                    rejectNote={rejectNote}
                    setRejectNote={setRejectNote}
                    setRejecting={setRejecting}
                    onDecide={decide}
                    onReload={load}
                  />
                )}

                {/* ⑤ report 关：审查报告 */}
                {stage === 4 && (
                  <ReportStage
                    sel={sel}
                    impact={impact}
                    deciding={deciding}
                    rejecting={rejecting}
                    rejectNote={rejectNote}
                    setRejectNote={setRejectNote}
                    setRejecting={setRejecting}
                    onDecide={decide}
                  />
                )}

                {/* done：归档摘要 */}
                {stage === 'done' && <DoneStage sel={sel} backendRepo={backendRepo} impact={impact} />}

                {/* 未启动/终态灰态 */}
                {stage === 'error' && (
                  <ErrorStage
                    sel={sel}
                    backendRepo={backendRepo}
                    onCreateTask={onCreateTask}
                    onReload={load}
                    onClearedSelection={() => setSelected(null)}
                  />
                )}

                  </>
                )}
              </div>

              {/* 产物文档卡（④⑤ 与 done 的侧栏；管道回看 0/1 阶段时一并显示，产物随看随查） */}
              {(viewing || stage === 3 || stage === 4 || stage === 'done') && (
                <aside className="flex w-64 shrink-0 flex-col border-l border-slate-100 dark:border-slate-800 bg-slate-50/40 dark:bg-slate-900/40">
                  <p className="px-3 pt-3 text-micro font-bold uppercase tracking-wider text-slate-400 dark:text-slate-500">产物文档</p>
                  <div className="min-h-0 flex-1 space-y-1.5 overflow-y-auto p-2.5">
                    {docs.length === 0 && <p className="px-1 py-2 text-micro text-slate-400 dark:text-slate-500">本任务暂无产物文档</p>}
                    {docs.map((d) => (
                      <DocCard key={d.path} backendRepo={backendRepo} doc={d} onDeleted={() => setDocsTick((t) => t + 1)} />
                    ))}
                  </div>
                  <p className="px-3 pb-2.5 text-[9px] text-slate-300 dark:text-slate-600">路径规范：.easyvibe/development_docs/</p>
                </aside>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  )
}
