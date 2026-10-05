import { useCallback, useEffect, useMemo, useState } from 'react'
import { ChevronDown, ChevronRight, Copy, ExternalLink, FileDiff, History, ShieldCheck } from 'lucide-react'
import { onTaskEvent } from '@/runtime/growthBus'
import { Select } from '@/components/ui/SelectMenu'
import { absTime, aggregateByModule, parseDiffStat, toMs } from '@/shared/logic/diffStat'
import type { CodeMap } from '@/types/map'

// M4-3 变更记录整页（按 ui-mockups/变更记录原型.png 施工）：
// 左列时间线（信任级别/模块筛选 + 可展开行：模块聚合 diff 条）
// + 右列变更详情（KPI / 基本信息 / 关联评审 / 备注）。
// 数据面：GET tasks（result.diffStat 解析）+ approvals（关联评审）——零新增后端。

interface ChangeTask {
  id: string
  title: string
  status: string
  trust: string
  modules: unknown
  createdAt: string
  updatedAt: string
  conversationId: string | null
  /** 归档信封：result 字段是解析后的 [EASYVIBE-RESULT]（summary/changed_modules 在里面） */
  result?: {
    result?: { summary?: string; changed_modules?: string[] }
    diffStat?: string
    archivedPath?: string
    warnings?: string[]
  } | null
}

interface Approval {
  id: string
  gate: string
  decision: string
  note: string | null
  decidedAt: string
}

const TRUST_LABEL: Record<string, string> = { manual: '手动', auto: '自动', supervised: '监督' }
const TRUST_CHIP: Record<string, string> = {
  manual: 'bg-blue-50 dark:bg-blue-950/40 text-blue-600 border-blue-200 dark:border-blue-900/60',
  auto: 'bg-slate-100 dark:bg-slate-800 text-slate-500 dark:text-slate-400 border-slate-200 dark:border-slate-700',
  supervised: 'bg-violet-50 text-violet-600 border-violet-200',
}
const STATUS_DOT: Record<string, string> = {
  done: '#10b981',
  failed: '#ef4444',
  running: '#f59e0b',
  awaiting_approval: '#8b5cf6',
  pending: '#94a3b8',
  interrupted: '#94a3b8',
  rejected: '#ef4444',
}
const GATE_LABEL: Record<string, string> = { plan: '计划审批', diff: 'Diff 审批', report: '审查报告' }

export function ChangesPage({ backendRepo, map, onOpenTask }: { backendRepo: string | null; map: CodeMap | null; onOpenTask?: (taskId: string) => void }) {
  const [tasks, setTasks] = useState<ChangeTask[] | null>(null)
  const [trustFilter, setTrustFilter] = useState<'all' | 'manual' | 'auto' | 'supervised'>('all')
  const [moduleFilter, setModuleFilter] = useState<string>('all')
  const [expanded, setExpanded] = useState<string | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  // 审批记录带任务 id 快照，切换选中项时天然丢弃旧数据（避免 effect 内同步 setState）
  const [approvalsFor, setApprovalsFor] = useState<{ taskId: string; list: Approval[] } | null>(null)
  // 回放（审查 2 欠账）：按需拉取完整 diff 原文，同审批记录的 taskId 快照模式
  const [diffFor, setDiffFor] = useState<{ taskId: string; text: string | null; loading: boolean } | null>(null)

  const loadDiff = async (taskId: string) => {
    if (!backendRepo) return
    setDiffFor({ taskId, text: null, loading: true })
    try {
      const r = await fetch(`/api/repos/${encodeURIComponent(backendRepo)}/tasks/${encodeURIComponent(taskId)}/diff`)
      const d = await r.json().catch(() => null)
      setDiffFor({ taskId, text: d?.data?.diff ?? null, loading: false })
    } catch {
      setDiffFor({ taskId, text: null, loading: false })
    }
  }

  const load = useCallback(() => {
    if (!backendRepo) return
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/tasks`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: ChangeTask[] } | null) => setTasks(d?.data ?? []))
      .catch(() => setTasks([]))
  }, [backendRepo])

  useEffect(() => {
    load()
  }, [load])
  useEffect(() => onTaskEvent(load), [load])

  useEffect(() => {
    if (!backendRepo || !selected) return
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/tasks/${encodeURIComponent(selected)}/approvals`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: Approval[] } | null) => setApprovalsFor({ taskId: selected, list: d?.data ?? [] }))
      .catch(() => setApprovalsFor({ taskId: selected, list: [] }))
  }, [backendRepo, selected])

  const filtered = useMemo(() => {
    let list = tasks ?? []
    if (trustFilter !== 'all') list = list.filter((t) => t.trust === trustFilter)
    if (moduleFilter !== 'all') {
      list = list.filter((t) => {
        const changed: string[] = t.result?.result?.changed_modules ?? []
        const declared = Array.isArray(t.modules) ? (t.modules as string[]) : []
        return changed.includes(moduleFilter) || declared.includes(moduleFilter)
      })
    }
    return [...list].sort((a, b) => b.createdAt.localeCompare(a.createdAt))
  }, [tasks, trustFilter, moduleFilter])

  const sel = filtered.find((t) => t.id === selected) ?? null
  const selStat = useMemo(() => (sel?.result?.diffStat ? parseDiffStat(sel.result.diffStat) : null), [sel])
  const selImpact = useMemo(() => {
    if (!selStat || !map) return []
    return aggregateByModule(
      selStat.files,
      map.modules.map((m) => ({ id: m.id, name: m.name, files: m.files as string[] })),
    )
  }, [selStat, map])
  const approvals = sel && approvalsFor?.taskId === sel.id ? approvalsFor.list : []
  const duration = (() => {
    if (!sel) return 0
    const end = toMs(sel.updatedAt)
    const start = toMs(sel.createdAt)
    return end !== null && start !== null ? Math.max(0, end - start) : 0
  })()
  const durText = duration <= 0 ? '—' : duration < 60000 ? `${Math.round(duration / 1000)} 秒` : `${Math.floor(duration / 60000)} 分钟 ${Math.round((duration % 60000) / 1000)} 秒`

  if (!backendRepo) {
    return <div className="flex h-full items-center justify-center text-[12px] text-slate-400 dark:text-slate-500">先在左侧选择一个项目。</div>
  }

  return (
    <div className="flex h-full flex-col">
      <div className="min-h-0 flex-1 overflow-hidden">
        <div className="flex h-full">
          {/* 左列：时间线 */}
          <div className="flex min-w-0 flex-1 flex-col">
            <div className="border-b border-slate-100 dark:border-slate-800 px-5 pb-3 pt-4">
              <h2 className="text-[15px] font-bold text-slate-800 dark:text-slate-100">变更记录</h2>
              <p className="mt-0.5 text-[11px] text-slate-400 dark:text-slate-500">
                这个仓库被 EasyVibe 改动过的每一次留痕：改了什么、涉及哪些模块、由哪个任务产生——可回放、可回溯。
              </p>
              {/* 筛选条 */}
              <div className="mt-3 flex items-center gap-2">
                <div className="flex overflow-hidden rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
                  {(['all', 'manual', 'auto', 'supervised'] as const).map((k) => (
                    <button
                      key={k}
                      onClick={() => setTrustFilter(k)}
                      className={`px-2.5 py-1 text-cap font-semibold ${
                        trustFilter === k ? 'bg-blue-600 text-white' : 'text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70'
                      }`}
                    >
                      {k === 'all' ? '全部' : TRUST_LABEL[k]}
                    </button>
                  ))}
                </div>
                <Select
                  value={moduleFilter}
                  onChange={setModuleFilter}
                  ariaLabel="按模块过滤"
                  className="w-40"
                  options={[
                    { value: 'all', label: '全部模块' },
                    ...(map?.modules ?? []).map((m) => ({ value: m.id, label: m.name })),
                  ]}
                />
                <span className="tnum ml-auto text-cap text-slate-300 dark:text-slate-600">共 {filtered.length} 条</span>
              </div>
            </div>

            <div className="min-h-0 flex-1 space-y-1.5 overflow-y-auto p-4">
              {filtered.map((t) => {
                const stat = t.result?.diffStat ? parseDiffStat(t.result.diffStat) : null
                const changedCount = t.result?.result?.changed_modules?.length ?? (Array.isArray(t.modules) ? t.modules.length : 0)
                const open = expanded === t.id
                const impact = open && stat && map
                  ? aggregateByModule(
                      stat.files,
                      map.modules.map((m) => ({ id: m.id, name: m.name, files: m.files as string[] })),
                    )
                  : []
                const maxI = Math.max(1, ...impact.map((i) => i.adds + i.dels))
                return (
                  <div
                    key={t.id}
                    className={`rounded-xl border bg-white dark:bg-slate-900 ${selected === t.id ? 'border-blue-300 dark:border-blue-800 ring-1 ring-blue-100' : 'border-slate-200 dark:border-slate-700'}`}
                  >
                    <button
                      onClick={() => {
                        setSelected(t.id)
                        setExpanded(open ? null : t.id)
                      }}
                      className="flex w-full items-center gap-2.5 px-3.5 py-2.5 text-left"
                    >
                      <span className="h-2 w-2 shrink-0 rounded-full" style={{ backgroundColor: STATUS_DOT[t.status] ?? '#94a3b8' }} />
                      <span className="tnum w-[92px] shrink-0 text-micro leading-3 text-slate-400 dark:text-slate-500">
                        {absTime(t.createdAt).split(' ')[0]}
                        <br />
                        {absTime(t.createdAt).split(' ')[1]}
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[12px] font-semibold text-slate-700 dark:text-slate-200">{t.title}</span>
                        <span className="mt-0.5 flex items-center gap-1.5 text-micro text-slate-400 dark:text-slate-500">
                          {changedCount > 0 && <span>{changedCount} 个模块</span>}
                          {stat && (
                            <span className="tnum">
                              <i className="not-italic text-emerald-500">+{stat.insertions}</i> /{' '}
                              <i className="not-italic text-red-400">-{stat.deletions}</i> 行
                            </span>
                          )}
                          <span className={`rounded-full border px-1.5 py-px text-micro font-semibold ${TRUST_CHIP[t.trust] ?? TRUST_CHIP.auto}`}>
                            {TRUST_LABEL[t.trust] ?? t.trust}
                          </span>
                        </span>
                      </span>
                      {open ? <ChevronDown size={13} className="shrink-0 text-slate-400 dark:text-slate-500" /> : <ChevronRight size={13} className="shrink-0 text-slate-300 dark:text-slate-600" />}
                    </button>
                    {open && impact.length > 0 && (
                      <div className="space-y-1.5 border-t border-slate-50 px-4 py-3">
                        {impact.map((i) => (
                          <div key={i.id} className="flex items-center gap-2">
                            <span className="w-20 shrink-0 truncate text-cap text-slate-500 dark:text-slate-400">{i.name}</span>
                            <div className="flex h-2 flex-1 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800">
                              <div className="bg-emerald-400" style={{ width: `${(i.adds / maxI) * 100}%` }} />
                              <div className="bg-red-300" style={{ width: `${(i.dels / maxI) * 100}%` }} />
                            </div>
                            <span className="tnum w-14 shrink-0 text-right text-micro text-slate-400 dark:text-slate-500">
                              <i className="not-italic text-emerald-500">+{i.adds}</i>{' '}
                              <i className="not-italic text-red-400">-{i.dels}</i>
                            </span>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )
              })}
              {filtered.length === 0 && (
                <div className="py-16 text-center text-[12px] text-slate-300 dark:text-slate-600">
                  {tasks === null ? '加载中…' : '没有符合筛选条件的变更记录。'}
                </div>
              )}
            </div>
          </div>

          {/* 右列：变更详情 */}
          <aside className="flex w-[320px] shrink-0 flex-col border-l border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900">
            {!sel ? (
              <div className="flex flex-1 flex-col items-center justify-center gap-2 text-slate-300 dark:text-slate-600">
                <History size={22} />
                <p className="text-[11px]">选择一条变更查看详情</p>
              </div>
            ) : (
              <div className="min-h-0 flex-1 overflow-y-auto">
                <div className="border-b border-slate-100 dark:border-slate-800 px-4 py-3">
                  <div className="flex items-center justify-between">
                    <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">变更详情</span>
                    <span className="flex items-center gap-1.5">
                      {/* 重审 P2：变更页→流水线的导航闭环（此前看完变更无处去处理） */}
                      {onOpenTask && (
                        <button
                          onClick={() => onOpenTask(sel.id)}
                          className="flex items-center gap-0.5 rounded-md border border-slate-200 dark:border-slate-700 px-2 py-0.5 text-micro font-semibold text-slate-500 dark:text-slate-400 hover:border-blue-300 hover:text-blue-600"
                          title="跳到任务页流水线视图，看审批留痕/产物/diff 全程"
                        >
                          <ExternalLink size={10} /> 查看流水线
                        </button>
                      )}
                      <span className="flex items-center gap-0.5 text-micro text-slate-300 dark:text-slate-600">
                        <ShieldCheck size={10} /> 全程留痕
                      </span>
                    </span>
                  </div>
                  {selStat && (
                    <div className="tnum mt-2.5 grid grid-cols-4 gap-1 text-center">
                      {[
                        { v: `+${selStat.insertions}`, l: '新增行', c: 'text-emerald-500' },
                        { v: `-${selStat.deletions}`, l: '删除行', c: 'text-red-400' },
                        { v: sel.result?.result?.changed_modules?.length ?? '—', l: '变更模块', c: 'text-slate-700 dark:text-slate-200' },
                        { v: selStat.fileCount, l: '文件变更', c: 'text-slate-700 dark:text-slate-200' },
                      ].map((k) => (
                        <div key={k.l} className="rounded-lg bg-slate-50 dark:bg-slate-950/70 py-1.5">
                          <p className={`text-[13px] font-bold ${k.c}`}>{k.v}</p>
                          <p className="mt-0.5 text-micro text-slate-400 dark:text-slate-500">{k.l}</p>
                        </div>
                      ))}
                    </div>
                  )}
                </div>

                <div className="border-b border-slate-100 dark:border-slate-800 px-4 py-3">
                  <p className="mb-1.5 text-cap font-bold text-slate-400 dark:text-slate-500">基本信息</p>
                  <dl className="space-y-1.5 text-[11px]">
                    {[
                      { l: '任务 ID', v: sel.id, mono: true, copy: true },
                      { l: '归纳会话', v: sel.conversationId ?? '—', mono: true, copy: !!sel.conversationId },
                      { l: '信任级别', v: TRUST_LABEL[sel.trust] ?? sel.trust },
                      { l: '耗时', v: durText },
                      { l: '创建时间', v: absTime(sel.createdAt) },
                    ].map((row) => (
                      <div key={row.l} className="flex items-center gap-2">
                        <dt className="w-16 shrink-0 text-slate-400 dark:text-slate-500">{row.l}</dt>
                        <dd className={`min-w-0 flex-1 truncate text-slate-600 dark:text-slate-300 ${row.mono ? 'mono text-micro' : ''}`}>{row.v}</dd>
                        {row.copy && (
                          <button
                            onClick={() => void navigator.clipboard?.writeText(String(row.v))}
                            className="shrink-0 rounded p-0.5 text-slate-300 dark:text-slate-600 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-500"
                            title="复制"
                          >
                            <Copy size={10} />
                          </button>
                        )}
                      </div>
                    ))}
                  </dl>
                </div>

                {selImpact.length > 0 && (
                  <div className="border-b border-slate-100 dark:border-slate-800 px-4 py-3">
                    <p className="mb-1.5 text-cap font-bold text-slate-400 dark:text-slate-500">模块影响面</p>
                    <div className="space-y-1.5">
                      {selImpact.map((i) => (
                        <div key={i.id} className="flex items-center gap-2">
                          <span className="w-20 shrink-0 truncate text-cap text-slate-500 dark:text-slate-400">{i.name}</span>
                          <div className="flex h-2 flex-1 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800">
                            <div className="bg-emerald-400" style={{ width: `${(i.adds / Math.max(1, ...selImpact.map((x) => x.adds + x.dels))) * 100}%` }} />
                            <div className="bg-red-300" style={{ width: `${(i.dels / Math.max(1, ...selImpact.map((x) => x.adds + x.dels))) * 100}%` }} />
                          </div>
                          <span className="tnum w-14 shrink-0 text-right text-micro text-slate-400 dark:text-slate-500">
                            <i className="not-italic text-emerald-500">+{i.adds}</i>{' '}
                            <i className="not-italic text-red-400">-{i.dels}</i>
                          </span>
                        </div>
                      ))}
                    </div>
                  </div>
                )}

                <div className="border-b border-slate-100 dark:border-slate-800 px-4 py-3">
                  <p className="mb-1.5 text-cap font-bold text-slate-400 dark:text-slate-500">关联评审</p>
                  {approvals.length === 0 ? (
                    <p className="text-[11px] text-slate-300 dark:text-slate-600">该任务没有经过审批关（自动模式或尚未到达）。</p>
                  ) : (
                    <div className="space-y-1.5">
                      {approvals.map((a) => (
                        <div key={a.id} className="flex items-center justify-between rounded-lg bg-slate-50 dark:bg-slate-950/70 px-2.5 py-1.5">
                          <div>
                            <p className="mono text-micro text-slate-600 dark:text-slate-300">{a.id}</p>
                            <p className="mt-0.5 text-micro text-slate-400 dark:text-slate-500">
                              {GATE_LABEL[a.gate] ?? a.gate} · {absTime(a.decidedAt)}
                              {a.note ? ` · ${a.note}` : ''}
                            </p>
                          </div>
                          <span
                            className={`rounded-full px-1.5 py-px text-micro font-semibold ${
                              a.decision === 'approved'
                                ? 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-600'
                                : a.decision === 'rejected'
                                  ? 'bg-red-50 dark:bg-red-950/40 text-red-500'
                                  : a.decision === 'rewind'
                                    ? 'bg-violet-50 dark:bg-violet-950/40 text-violet-600'
                                    : 'bg-slate-100 dark:bg-slate-800 text-slate-400 dark:text-slate-500'
                            }`}
                          >
                            {a.decision === 'approved' ? '已通过' : a.decision === 'rejected' ? '已驳回' : a.decision === 'rewind' ? '回退' : '已跳过'}
                          </span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>

                {(sel.result?.result?.summary || (sel.result?.warnings?.length ?? 0) > 0) && (
                  <div className="px-4 py-3">
                    <p className="mb-1.5 text-cap font-bold text-slate-400 dark:text-slate-500">备注</p>
                    {sel.result?.result?.summary && <p className="text-[11px] leading-4 text-slate-600 dark:text-slate-300">{sel.result.result.summary}</p>}
                    {sel.result?.warnings?.map((w, i) => (
                      <p key={i} className="mt-1 text-micro leading-4 text-amber-600">⚠ {w}</p>
                    ))}
                  </div>
                )}

                {/* 回放：完整 diff 原文（按需加载，首屏 400 行） */}
                <div className="border-t border-slate-100 dark:border-slate-800 px-4 py-3">
                  <div className="flex items-center justify-between">
                    <p className="text-cap font-bold text-slate-400 dark:text-slate-500">回放</p>
                    {diffFor?.taskId !== sel.id && (
                      <button
                        onClick={() => loadDiff(sel.id)}
                        className="flex items-center gap-1 rounded-md border border-slate-200 dark:border-slate-700 px-2 py-0.5 text-micro font-semibold text-slate-500 dark:text-slate-400 transition-colors hover:border-blue-300 hover:text-blue-600"
                      >
                        <FileDiff size={10} /> 查看改动原文
                      </button>
                    )}
                  </div>
                  {diffFor?.taskId === sel.id &&
                    (diffFor.loading ? (
                      <p className="text-micro mt-2 text-slate-300 dark:text-slate-600">加载中…</p>
                    ) : diffFor.text ? (
                      <pre className="mono text-cap mt-2 max-h-72 overflow-auto rounded-md bg-slate-50 dark:bg-slate-950/70 p-2.5 leading-4">
                        {diffFor.text.split('\n').slice(0, 400).map((line, i) => (
                          <div
                            key={i}
                            className={
                              line.startsWith('+') && !line.startsWith('+++')
                                ? 'bg-emerald-50 dark:bg-emerald-950/40 text-emerald-700'
                                : line.startsWith('-') && !line.startsWith('---')
                                  ? 'bg-red-50 dark:bg-red-950/40 text-red-600'
                                  : 'text-slate-600 dark:text-slate-300'
                            }
                          >
                            {line || ' '}
                          </div>
                        ))}
                        {diffFor.text.split('\n').length > 400 && (
                          /* 重审 P2：此前"完整内容在评审页查看"是无链接死文字——真按钮跳流水线 */
                          <div className="py-1 text-center">
                            {onOpenTask ? (
                              <button
                                onClick={() => onOpenTask(sel.id)}
                                className="rounded-md border border-slate-200 dark:border-slate-700 px-2 py-0.5 text-micro font-semibold text-slate-500 dark:text-slate-400 hover:border-blue-300 hover:text-blue-600"
                              >
                                仅显示前 400 行——去流水线查看完整 diff →
                              </button>
                            ) : (
                              <span className="text-micro text-slate-400 dark:text-slate-500">（仅显示前 400 行，完整内容在评审页查看）</span>
                            )}
                          </div>
                        )}
                      </pre>
                    ) : (
                      <p className="text-micro mt-2 text-slate-300 dark:text-slate-600">该任务没有可回放的改动（可能未产生 diff 或尚未归档）。</p>
                    ))}
                </div>
              </div>
            )}
          </aside>
        </div>
      </div>
      {/* 底条 */}
      <div className="flex items-center justify-center gap-1 border-t border-slate-100 dark:border-slate-800 bg-slate-50/60 dark:bg-slate-900/60 py-1.5 text-micro text-slate-400 dark:text-slate-500">
        <History size={10} /> 全程留痕 · 可回放
      </div>
    </div>
  )
}
