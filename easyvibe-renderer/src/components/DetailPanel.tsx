import { useEffect, useMemo, useState } from 'react'
import { X, FileCode2, KeyRound, Flag, StickyNote, ArrowDownToLine, ArrowUpFromLine, Boxes, Info, Wrench, ListChecks, MessagesSquare, Gauge} from 'lucide-react'
import { buildLayerTask, buildModuleTask, buildSubmoduleTask, type TaskDraft } from '@/shared/logic/taskContext'
import type { CodeMap, Layer, Module, SubMap, SubModule } from '@/types/map'
import { healthColor, healthLabel, dependentsOf } from '@/shared/logic/layout'
import { couplingAnalysis } from '@/shared/logic/depsAnalysis'
import { Badge } from '@/components/ui/badge'
import { Separator } from '@/components/ui/separator'
import { IssuesList } from '@/components/IssuesList'
import { healthHistory } from '@/api/canvas'
import { usage } from '@/api/repos'
import { PanelChat } from '@/components/chat/PanelChat'
import { Waypoints } from 'lucide-react'

// 共享契约类型（2026-10-05 下沉 renderer-shared/contract）：canvas 与 chat 两侧共用，
// 定义在此会让 chat 域反向依赖 map-canvas，故统一改为从 shared 引入。
import type { Selection, PanelTab } from '@/shared/contract/selection'
import type { ChatAboutTarget } from '@/shared/contract/chat'

interface Props {
  map: CodeMap
  selection: Selection
  tab: PanelTab
  onTabChange: (tab: PanelTab) => void
  submaps: Record<string, SubMap | 'loading' | 'error'>
  backendRepo: string | null
  onCreateTask: (draft: TaskDraft) => void
  onLocateModule: (moduleId: string) => void
  /** S1-1：打开视图（多模块时画布 solo 聚焦） */
  onOpenView?: (ids: string[]) => void
  /** v0.2：对话页签占位与详情视图的「就此对话」入口（携带当前选中对象） */
  onChatAbout?: (target: ChatAboutTarget) => void
  /** 2026-10-05 Redesign-A：右栏审批出口——跳工作台「任务对话」页裁决 */
  onGoWorkbench?: () => void
  /** 2026-10-05 依赖体检入口（详情页签「耦合概览」区块「看全部 →」） */
  onOpenDeps?: () => void
  onClose: () => void
  /** 改进#4：右栏可调宽（测试员 IA 反馈的非破坏性验证——宽度够不够先看数据） */
  width?: number
}

function Row({ icon, label, children }: { icon: React.ReactNode; label: string; children: React.ReactNode }) {
  return (
    <div>
      <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-slate-400 dark:text-slate-500">
        {icon}
        {label}
      </div>
      {children}
    </div>
  )
}

// S2：健康趋势条——module_health_history 自 M2-4 落库以来的第一个消费者。
// 分数序列（旧→新）条形图：让"这模块在变好还是腐烂"可见，复检闭环的可视化判据。
function HealthTrend({ backendRepo, moduleId }: { backendRepo: string; moduleId: string }) {
  const [rows, setRows] = useState<{ score: number; runId: string }[] | null>(null)
  useEffect(() => {
    let stale = false
    healthHistory(backendRepo, moduleId)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: { score: number; runId: string }[] }) => {
        if (!stale) setRows(d.data.slice().reverse()) // 旧→新
      })
      .catch(() => !stale && setRows([]))
    return () => {
      stale = true
    }
  }, [backendRepo, moduleId])

  if (rows === null) return null
  if (rows.length < 2) return null // 单点无趋势，不渲染（避免噪音）
  const latest = rows[rows.length - 1].score
  const prev = rows[rows.length - 2].score
  const delta = latest - prev
  return (
    <div className="mt-2 flex items-center gap-2">
      <span className="text-micro font-semibold text-slate-400 dark:text-slate-500">健康趋势</span>
      <div className="flex h-4 items-end gap-0.5">
        {rows.slice(-12).map((r, i) => (
          <div
            key={i}
            className="w-1.5 rounded-sm"
            style={{ height: `${Math.max(12, r.score)}%`, background: healthColor(r.score) }}
            title={`${r.score} 分`}
          />
        ))}
      </div>
      <span className={`text-micro font-bold ${delta >= 0 ? 'text-emerald-600' : 'text-red-500'}`}>
        {delta >= 0 ? '+' : ''}
        {delta}
      </span>
    </div>
  )
}

// L1 治理账单：模块累计 agent 成本（用量页按模块归因的详情端落点）。
// 数据：GET /usage?days=30 的 byModule + sessions 过滤本模块；修复成效取 health-history 首末分差。
function GovernanceBill({ backendRepo, moduleId }: { backendRepo: string | null; moduleId: string }) {
  const [bill, setBill] = useState<{ cost: number | null; sessions: number; failed: number; recent: { id: string; label?: string | null; kind: string; costUsd?: number | null; status: string; startedAt: string }[] } | null>(null)
  const [scoreDelta, setScoreDelta] = useState<number | null>(null)
  useEffect(() => {
    if (!backendRepo) return
    let stale = false
    usage(backendRepo, 30)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { byModule?: { name: string; sessions: number; cost?: number | null; failed?: number | null }[]; sessions?: { id: string; label?: string | null; kind: string; moduleId?: string | null; costUsd?: number | null; status: string; startedAt: string }[] } } | null) => {
        if (stale || !d?.data) return
        const row = d.data.byModule?.find((m) => m.name === moduleId)
        const recent = (d.data.sessions ?? []).filter((s) => s.moduleId === moduleId).slice(0, 3)
        setBill({ cost: row?.cost ?? null, sessions: row?.sessions ?? 0, failed: row?.failed ?? 0, recent })
      })
      .catch(() => {})
    healthHistory(backendRepo, moduleId)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { score: number }[] } | null) => {
        if (stale) return
        const rows = d?.data ?? []
        if (rows.length >= 2) setScoreDelta(rows[rows.length - 1].score - rows[0].score)
      })
      .catch(() => {})
    return () => {
      stale = true
    }
  }, [backendRepo, moduleId])

  if (!backendRepo || !bill || (bill.sessions === 0 && bill.cost == null)) return null
  const KIND_ZH: Record<string, string> = { task: '修复', submap: '子图分析', 'subagent-review': '初审', 'subagent-audit': '审查' }
  return (
    <Row icon={<Gauge size={12} />} label="治理账单">
      <div className="space-y-1.5 rounded-lg border border-slate-100 dark:border-slate-800 bg-slate-50 dark:bg-slate-950/70 p-2.5">
        <div className="flex items-center gap-3 text-cap">
          <span className="tnum font-bold text-slate-700 dark:text-slate-200">{bill.cost == null ? '—' : `$${bill.cost.toFixed(2)}`}</span>
          <span className="text-slate-400 dark:text-slate-500">·</span>
          <span className="tnum text-slate-500 dark:text-slate-400">{bill.sessions} 次治理</span>
          {bill.failed > 0 && (
            <>
              <span className="text-slate-400 dark:text-slate-500">·</span>
              <span className="tnum text-red-500">{bill.failed} 次失败白跑</span>
            </>
          )}
          {scoreDelta !== null && (
            <>
              <span className="text-slate-400 dark:text-slate-500">·</span>
              <span className={`tnum font-semibold ${scoreDelta >= 0 ? 'text-emerald-600' : 'text-red-500'}`}>
                30 天分 {scoreDelta >= 0 ? '+' : ''}{scoreDelta}
              </span>
            </>
          )}
        </div>
        {bill.recent.length > 0 && (
          <div className="space-y-0.5">
            {bill.recent.map((r) => (
              <div key={r.id} className="flex items-center gap-1.5 text-micro text-slate-400 dark:text-slate-500">
                <span className={`h-1 w-1 rounded-full ${r.status === 'failed' ? 'bg-red-400' : 'bg-emerald-400'}`} />
                <span className="text-slate-500 dark:text-slate-400">{KIND_ZH[r.kind] ?? r.label ?? r.kind}</span>
                <span className="tnum">{r.costUsd == null ? '—' : `$${r.costUsd.toFixed(2)}`}</span>
                <span className="ml-auto">{r.startedAt.slice(5, 16).replace('T', ' ')}</span>
              </div>
            ))}
          </div>
        )}
        <p className="text-micro leading-4 text-slate-400 dark:text-slate-500">近 30 天 · 口径见「用量」页</p>
      </div>
    </Row>
  )
}

function ModuleView({ map, mod, onCreateTask, backendRepo, onChatAbout, onOpenDeps }: { map: CodeMap; mod: Module; onCreateTask: (d: TaskDraft) => void; backendRepo: string | null; onChatAbout?: (target: ChatAboutTarget) => void; onOpenDeps?: () => void }) {
  const color = healthColor(mod.health.score)
  const deps = mod.dependencies.map((id) => map.modules.find((m) => m.id === id)).filter(Boolean) as Module[]
  const dependents = dependentsOf(map, mod)
    .map((id) => map.modules.find((m) => m.id === id))
    .filter(Boolean) as Module[]
  // 2026-10-05 依赖体检入口：耦合概览（≤3 行 = 计数 + 首条违规 + 看全部）
  const analysis = useMemo(() => couplingAnalysis(map), [map])
  const nameOf = (id: string) => analysis.moduleById.get(id)?.name ?? id
  const myViolations = analysis.violations.filter((e) => e.from === mod.id || e.to === mod.id)
  const firstViolation = myViolations[0]

  return (
    <>
      <div>
        <div className="flex items-center gap-2">
          <h2 className="text-[15px] font-bold text-slate-800 dark:text-slate-100">{mod.name}</h2>
          <Badge variant="outline" className="border-slate-200 dark:border-slate-700 text-slate-500 dark:text-slate-400">
            {mod.id}
          </Badge>
        </div>
        <p className="mt-1 text-[12px] leading-5 text-slate-500 dark:text-slate-400">{mod.responsibility}</p>
        <p className="mt-1 text-[11px] text-slate-400 dark:text-slate-500">
          所属层：{map.layers.find((l) => l.id === mod.layer)?.name ?? mod.layer}
        </p>
      </div>

      <div className="rounded-lg border p-3" style={{ borderColor: `${color}55`, background: `${color}0d` }}>
        {/* M4-1 真人测试 Bug#3：原 justify-between 三元素挤一行，按钮压住指标文案——改两行布局 */}
        <div className="flex items-center justify-between">
          <span className="text-[11px] font-semibold" style={{ color }}>
            {healthLabel(mod.health.score)} · {mod.health.score}/100
          </span>
          <div className="flex items-center gap-1.5">
            {/* v0.2：对话入口（带模块上下文跳「任务对话」，@芯片+预填文本） */}
            {onChatAbout && (
              <button
                onClick={() => onChatAbout({ refId: mod.id, refName: mod.name, kind: 'module' })}
                className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1 text-micro font-bold text-slate-600 dark:text-slate-300 hover:border-blue-300 hover:text-blue-600"
                title="就此模块发起对话：跳转「任务对话」并自动带入模块上下文"
              >
                <MessagesSquare size={10} /> 就此对话
              </button>
            )}
            <button
              onClick={() => onCreateTask(buildModuleTask(map, mod.id))}
              className="flex items-center gap-1 rounded-full bg-blue-600 px-2.5 py-1 text-micro font-bold text-white hover:bg-blue-700"
              title="指哪打哪：以本模块为上下文发起修复任务"
            >
              <Wrench size={10} /> 发起修复
            </button>
          </div>
        </div>
        <p className="mt-1 text-cap text-slate-400 dark:text-slate-500">
          coupling {mod.health.coupling} · complexity {mod.health.complexity} · churn {mod.health.churn ?? 'n/a'}
        </p>
        {backendRepo && <HealthTrend backendRepo={backendRepo} moduleId={mod.id} />}
        {mod.health.review_note && <p className="mt-2 text-[12px] leading-5 text-slate-600 dark:text-slate-300">{mod.health.review_note}</p>}
      </div>

      <Row icon={<KeyRound size={12} />} label="关键入口">
        <div className="space-y-1.5">
          {mod.key_entries.slice(0, 6).map((k) => (
            <div key={k.file + k.symbol} className="rounded-md bg-slate-50 dark:bg-slate-950/70 px-2.5 py-1.5">
              <div className="font-mono text-[11px] font-medium text-slate-700 dark:text-slate-200">{k.symbol}</div>
              <div className="truncate font-mono text-micro text-slate-400 dark:text-slate-500">{k.file}</div>
            </div>
          ))}
          {mod.key_entries.length === 0 && <p className="text-[11px] text-slate-400 dark:text-slate-500">（无）</p>}
        </div>
      </Row>

      <Row icon={<FileCode2 size={12} />} label="文件归属">
        <div className="space-y-1">
          {mod.files.map((f) => (
            <div key={f} className="truncate font-mono text-cap text-slate-500 dark:text-slate-400">
              {f}
            </div>
          ))}
        </div>
      </Row>

      <Row icon={<ArrowDownToLine size={12} />} label={`依赖（${deps.length}）`}>
        <div className="flex flex-wrap gap-1.5">
          {deps.map((d) => (
            <Badge key={d.id} variant="secondary" className="bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300">
              {d.name}
            </Badge>
          ))}
          {deps.length === 0 && <p className="text-[11px] text-slate-400 dark:text-slate-500">（无）</p>}
        </div>
      </Row>

      <Row icon={<ArrowUpFromLine size={12} />} label={`被依赖（${dependents.length}）`}>
        <div className="flex flex-wrap gap-1.5">
          {dependents.map((d) => (
            <Badge key={d.id} variant="secondary" className="bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300">
              {d.name}
            </Badge>
          ))}
          {dependents.length === 0 && <p className="text-[11px] text-slate-400 dark:text-slate-500">（无）</p>}
        </div>
      </Row>

      <GovernanceBill backendRepo={backendRepo} moduleId={mod.id} />

      <Row icon={<Waypoints size={12} />} label="耦合概览">
        <div className="space-y-1.5 rounded-lg border border-slate-100 dark:border-slate-800 bg-slate-50 dark:bg-slate-950/70 p-2.5">
          <p className="tnum text-cap text-slate-500 dark:text-slate-400">
            出 {analysis.fanOut.get(mod.id) ?? 0} · 入 {analysis.fanIn.get(mod.id) ?? 0} · 逆向 {myViolations.length}
            {analysis.cycleModuleIds.has(mod.id) && <span className="text-amber-500"> · 在循环群内</span>}
          </p>
          {firstViolation && (
            <p className="text-cap leading-4 text-red-500">
              「{nameOf(firstViolation.from)}」反向调用了「{nameOf(firstViolation.to)}」
            </p>
          )}
          {onOpenDeps && (
            <button onClick={onOpenDeps} className="text-micro font-bold text-blue-600 hover:text-blue-700">
              看全部 →
            </button>
          )}
        </div>
      </Row>

      {mod.health.decay_flags.length > 0 && (
        <Row icon={<Flag size={12} />} label="腐化标记">
          <div className="flex flex-wrap gap-1.5">
            {mod.health.decay_flags.map((f) => (
              <Badge key={f} className="border-red-200 dark:border-red-900/60 bg-red-50 dark:bg-red-950/40 font-normal text-red-600">
                {f}
              </Badge>
            ))}
          </div>
        </Row>
      )}

      <Separator />

      <Row icon={<StickyNote size={12} />} label="说明">
        <p className="text-[11px] leading-5 text-slate-400 dark:text-slate-500">
          健康度为 LLM 巡检评估结果，仅供架构演进参考；关键入口与文件归属来自语义代码地图（{map.meta.generator}）。
        </p>
      </Row>
    </>
  )
}

function LayerView({ map, layer, onCreateTask, onChatAbout }: { map: CodeMap; layer: Layer; onCreateTask: (d: TaskDraft) => void; onChatAbout?: (target: ChatAboutTarget) => void }) {
  const mods = map.modules.filter((m) => m.layer === layer.id)
  const avg = Math.round(mods.reduce((s, m) => s + m.health.score, 0) / Math.max(mods.length, 1))
  const color = healthColor(avg)
  const modIds = new Set(mods.map((m) => m.id))
  const violations = map.edges.filter(
    (e) => e.direction_violation && (modIds.has(e.from) || modIds.has(e.to)),
  )
  const downViolations = violations.filter((e) => modIds.has(e.from) && !modIds.has(e.to)).length

  return (
    <>
      <div>
        <div className="flex items-center gap-2">
          <Boxes size={16} className="text-slate-500 dark:text-slate-400" />
          <h2 className="text-[15px] font-bold text-slate-800 dark:text-slate-100">{layer.name}</h2>
          <Badge variant="outline" className="border-slate-200 dark:border-slate-700 text-slate-500 dark:text-slate-400">
            {layer.id}
          </Badge>
        </div>
        <p className="mt-1 text-[12px] leading-5 text-slate-500 dark:text-slate-400">{layer.description}</p>
        <p className="mt-1 text-[11px] text-slate-400 dark:text-slate-500">层序：L{layer.order}（0 为最顶层 / 入口侧）</p>
      </div>

      <div className="rounded-lg border p-3" style={{ borderColor: `${color}55`, background: `${color}0d` }}>
        {/* M4-1.5 陪审团硬缺陷：三元素挤一行压字断行——两行布局 */}
        <div className="flex items-center justify-between">
          <span className="text-[11px] font-semibold" style={{ color }}>
            层健康（聚合） · <span className="tnum">{avg}</span>/100
          </span>
          <div className="flex items-center gap-1.5">
            {onChatAbout && (
              <button
                onClick={() => onChatAbout({ refId: layer.id, refName: layer.name, kind: 'layer' })}
                className="flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1 text-micro font-bold text-slate-600 dark:text-slate-300 hover:border-blue-300 hover:text-blue-600"
                title="就此层发起对话：跳转「任务对话」并自动带入层上下文"
              >
                <MessagesSquare size={10} /> 就此对话
              </button>
            )}
            <button
              onClick={() => onCreateTask(buildLayerTask(map, layer.id))}
              className="flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full bg-blue-600 px-2.5 py-1 text-micro font-bold text-white hover:bg-blue-700"
              title="指哪打哪：以本层为上下文发起治理任务"
            >
              <Wrench size={10} /> 发起治理
            </button>
          </div>
        </div>
        <p className="mt-1 text-cap text-slate-400 dark:text-slate-500">
          {mods.length} 模块 · 逆向依赖 <span className="tnum">{violations.length}</span> 条
        </p>
        <p className="mt-2 flex items-start gap-1.5 text-cap leading-4 text-slate-500 dark:text-slate-400">
          <Info size={11} className="mt-0.5 shrink-0 text-slate-400 dark:text-slate-500" />
          层健康是成员模块分数的聚合参考，并非 LLM 独立评估；LLM 评估仅模块级与架构级两级。
        </p>
      </div>

      <Row icon={<Flag size={12} />} label={`层间逆向依赖（${violations.length}）`}>
        <div className="space-y-1.5">
          {violations.slice(0, 8).map((e, i) => (
            <div key={i} className="flex items-center gap-1.5 rounded-md bg-red-50 dark:bg-red-950/40 px-2 py-1.5 text-cap text-red-600">
              <span className="font-mono font-medium">{e.from}</span>
              <span className="text-red-400">→</span>
              <span className="font-mono font-medium">{e.to}</span>
              <span className="ml-auto truncate text-red-400">{e.label ?? e.type}</span>
            </div>
          ))}
          {violations.length === 0 && <p className="text-[11px] text-slate-400 dark:text-slate-500">（无，该层方向约束良好）</p>}
        </div>
      </Row>

      <Row icon={<Boxes size={12} />} label={`成员模块（${mods.length}）`}>
        <div className="space-y-1.5">
          {mods.map((m) => (
            <div key={m.id} className="flex items-center gap-2 rounded-md bg-slate-50 dark:bg-slate-950/70 px-2.5 py-2">
              <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: healthColor(m.health.score) }} />
              <div className="min-w-0">
                <div className="text-[12px] font-medium text-slate-700 dark:text-slate-200">{m.name}</div>
                <div className="truncate text-micro text-slate-400 dark:text-slate-500">{m.responsibility}</div>
              </div>
              <span className="ml-auto text-cap font-semibold" style={{ color: healthColor(m.health.score) }}>
                {m.health.score}
              </span>
            </div>
          ))}
        </div>
      </Row>

      {downViolations > 0 && (
        <p className="text-cap leading-4 text-slate-400 dark:text-slate-500">
          其中 {downViolations} 条为指向更上层的逆向依赖（本层模块主动引用上层）。
        </p>
      )}
    </>
  )
}

// 子模块详情（§8 子图 drill-down 层）
function SubmoduleView({ parent, sub, submap, onCreateTask }: { parent: Module; sub: SubModule; submap: SubMap; onCreateTask: (d: TaskDraft) => void }) {
  const color = healthColor(sub.health.score)
  const siblings = new Map(submap.sub_modules.map((s) => [s.id, s]))
  const deps = sub.dependencies.map((id) => siblings.get(id)).filter(Boolean) as SubModule[]
  const dependents = submap.edges.filter((e) => e.to === sub.id).map((e) => siblings.get(e.from)).filter(Boolean) as SubModule[]

  return (
    <>
      <div>
        <div className="flex items-center gap-2">
          <h2 className="text-[15px] font-bold text-slate-800 dark:text-slate-100">{sub.name}</h2>
          {/* M4-1.5：子模块补上修复入口（模块/层都有，此前独缺） */}
          <button
            onClick={() => onCreateTask(buildSubmoduleTask(parent, sub))}
            className="ml-auto flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full bg-blue-600 px-2.5 py-1 text-micro font-bold text-white hover:bg-blue-700"
            title="指哪打哪：以父模块+该子模块为上下文发起修复任务"
          >
            <Wrench size={10} /> 发起修复
          </button>
          <Badge variant="outline" className="border-slate-200 dark:border-slate-700 text-slate-500 dark:text-slate-400">
            {sub.id}
          </Badge>
        </div>
        <p className="mt-1 text-[12px] leading-5 text-slate-500 dark:text-slate-400">{sub.responsibility}</p>
        <p className="mt-1 text-[11px] text-slate-400 dark:text-slate-500">
          所属模块：{parent.name}（内部结构，无层概念）
        </p>
      </div>

      <div className="rounded-lg border p-3" style={{ borderColor: `${color}55`, background: `${color}0d` }}>
        <div className="flex items-center justify-between">
          <span className="text-[11px] font-semibold" style={{ color }}>
            {healthLabel(sub.health.score)} · {sub.health.score}/100
          </span>
          <span className="text-cap text-slate-400 dark:text-slate-500">
            coupling {sub.health.coupling} · complexity {sub.health.complexity} · churn {sub.health.churn ?? 'n/a'}
          </span>
        </div>
        {sub.health.review_note && <p className="mt-2 text-[12px] leading-5 text-slate-600 dark:text-slate-300">{sub.health.review_note}</p>}
      </div>

      <Row icon={<KeyRound size={12} />} label="关键入口">
        <div className="space-y-1.5">
          {sub.key_entries.slice(0, 6).map((k) => (
            <div key={k.file + k.symbol} className="rounded-md bg-slate-50 dark:bg-slate-950/70 px-2.5 py-1.5">
              <div className="font-mono text-[11px] font-medium text-slate-700 dark:text-slate-200">{k.symbol}</div>
              <div className="truncate font-mono text-micro text-slate-400 dark:text-slate-500">{k.file}</div>
            </div>
          ))}
        </div>
      </Row>

      <Row icon={<FileCode2 size={12} />} label="文件归属">
        <div className="space-y-1">
          {sub.files.map((f) => (
            <div key={f} className="truncate font-mono text-cap text-slate-500 dark:text-slate-400">{f}</div>
          ))}
        </div>
      </Row>

      <Row icon={<ArrowDownToLine size={12} />} label={`内部依赖（${deps.length}）`}>
        <div className="flex flex-wrap gap-1.5">
          {deps.map((d) => (
            <Badge key={d.id} variant="secondary" className="bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300">{d.name}</Badge>
          ))}
          {deps.length === 0 && <p className="text-[11px] text-slate-400 dark:text-slate-500">（无）</p>}
        </div>
      </Row>

      <Row icon={<ArrowUpFromLine size={12} />} label={`被内部依赖（${dependents.length}）`}>
        <div className="flex flex-wrap gap-1.5">
          {dependents.map((d) => (
            <Badge key={d.id} variant="secondary" className="bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300">{d.name}</Badge>
          ))}
          {dependents.length === 0 && <p className="text-[11px] text-slate-400 dark:text-slate-500">（无）</p>}
        </div>
      </Row>

      {sub.health.decay_flags.length > 0 && (
        <Row icon={<Flag size={12} />} label="腐化标记">
          <div className="flex flex-wrap gap-1.5">
            {sub.health.decay_flags.map((f) => (
              <Badge key={f} className="border-red-200 dark:border-red-900/60 bg-red-50 dark:bg-red-950/40 font-normal text-red-600">{f}</Badge>
            ))}
          </div>
        </Row>
      )}

      <Separator />

      <Row icon={<Info size={12} />} label="说明">
        <p className="text-[11px] leading-5 text-slate-400 dark:text-slate-500">
          子模块健康为<b>展开时的派生评估</b>（父模块内部的实现质量），不回流父模块分、不写入主地图文件；
          LLM 独立评估仍只有模块级与架构级两级。生成者：{submap.generator}。
        </p>
      </Row>
    </>
  )
}

export function DetailPanel({ map, selection, tab, onTabChange, submaps, backendRepo, onCreateTask, onLocateModule, onChatAbout, onGoWorkbench, onOpenDeps, onClose, width }: Props) {
  const module = selection?.kind === 'module' ? map.modules.find((m) => m.id === selection.id) : undefined
  const layer = selection?.kind === 'layer' ? map.layers.find((l) => l.id === selection.id) : undefined
  const parent = selection?.kind === 'submodule' ? map.modules.find((m) => m.id === selection.parentId) : undefined
  const submap = selection?.kind === 'submodule' ? submaps[selection.parentId] : undefined
  const smLoaded = submap && submap !== 'loading' && submap !== 'error' ? submap : undefined
  const sub = selection?.kind === 'submodule' && smLoaded
    ? smLoaded.sub_modules.find((s) => s.id === selection.subId)
    : undefined

  const detailLabel = module ? '模块详情' : layer ? '架构层详情' : sub ? '子模块详情' : '选中详情'
  // M4-1.5 选区收敛：模块或子模块选中时，问题清单随之收敛到该模块
  const scopeModuleName = module?.name ?? parent?.name

  return (
    <aside className="anim-panel-in flex h-full shrink-0 flex-col border-l border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900" style={{ width: width ?? 340 }}>
      <div className="flex items-center justify-between border-b border-slate-100 dark:border-slate-800 px-3 py-2">
        <div className="flex gap-1">
          {/* M4-1 瘦身：右栏只留 详情/问题/对话 三页签（v3 定稿顺序）；建议/视图移至顶栏抽屉，任务移至工作区页 */}
          {(
            [
              ['detail', detailLabel, <Boxes key="d" size={13} />],
              ['issues', '问题', <ListChecks key="i" size={13} />],
              ['chat', '对话', <MessagesSquare key="c" size={13} />],
            ] as const
          ).map(([key, label, icon]) => (
            <button
              key={key as string}
              onClick={() => onTabChange(key as typeof tab)}
              className={`flex items-center gap-1 rounded-md px-2 py-1.5 text-[12px] font-semibold transition-colors ${
                tab === key ? 'bg-blue-50 dark:bg-blue-950/40 text-blue-600' : 'text-slate-400 dark:text-slate-500 hover:text-slate-600'
              }`}
            >
              {icon}
              {label as string}
            </button>
          ))}
        </div>
        <button onClick={onClose} className="rounded p-1 text-slate-400 dark:text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-600">
          <X size={16} />
        </button>
      </div>
      {/* key={tab}：切页签重触发 150ms 淡入（评审 R3——此前内容瞬换无过渡） */}
      {tab === 'chat' ? (
        /* 2026-10-05 Redesign-A：对话页签独立容器——QuickAsk（检查器文档流）自管
           ContextBar/文档流/输入坞的纵向滚动，不再套 p-4 滚动层（双滚动条/贴边观感硬伤之源） */
        <div key={tab} className="anim-fade-in-fast flex h-full min-h-0 flex-1 flex-col overflow-hidden">
          <PanelChat
            backendRepo={backendRepo}
            map={map}
            selection={selection}
            onCreateTask={onCreateTask}
            onLocateModule={onLocateModule}
            onGoWorkbench={onGoWorkbench}
          />
        </div>
      ) : (
      <div key={tab} className="anim-fade-in-fast flex-1 space-y-5 overflow-y-auto p-4">
        {tab === 'issues' && (
          <IssuesList
            map={map}
            onLocate={(id) => onLocateModule(id)}
            onCreateTask={onCreateTask}
            backendRepo={backendRepo}
            scopeId={selection?.kind === 'module' ? selection.id : selection?.kind === 'submodule' ? selection.parentId : undefined}
            scopeName={scopeModuleName}
          />
        )}
        {tab === 'detail' && module && <ModuleView map={map} mod={module} onCreateTask={onCreateTask} backendRepo={backendRepo} onChatAbout={onChatAbout} onOpenDeps={onOpenDeps} />}
        {tab === 'detail' && !module && layer && <LayerView map={map} layer={layer} onCreateTask={onCreateTask} onChatAbout={onChatAbout} />}
        {tab === 'detail' && !module && !layer && sub && parent && smLoaded && (
          <SubmoduleView parent={parent} sub={sub} submap={smLoaded} onCreateTask={onCreateTask} />
        )}
        {tab === 'detail' && !module && !layer && !sub && selection?.kind === 'submodule' && (
          <p className="pt-8 text-center text-[12px] leading-5 text-slate-400 dark:text-slate-500">
            子模块数据不存在或仍在加载中，请稍候再点击。
          </p>
        )}
        {/* M4-1 详情空选态 = 地图级摘要（设计师要求：禁止空白，三态规范最高频面板落地） */}
        {tab === 'detail' && !module && !layer && !sub && selection?.kind !== 'submodule' && (
          <div className="space-y-3 pt-2">
            <p className="text-[11px] font-semibold text-slate-400 dark:text-slate-500">地图概览</p>
            <div className="grid grid-cols-2 gap-2">
              {[
                ['模块', `${map.modules.length}`],
                ['依赖边', `${map.edges.length}`],
                ['架构健康分', `${map.health.score}`],
                ['耦合度', map.health.coupling],
              ].map(([k, v]) => (
                <div key={k} className="rounded-lg border border-slate-100 dark:border-slate-800 bg-slate-50 dark:bg-slate-950/70 px-3 py-2">
                  <p className="text-micro text-slate-400 dark:text-slate-500">{k}</p>
                  <p className="text-[15px] font-bold text-slate-700 dark:text-slate-200">{v}</p>
                </div>
              ))}
            </div>
            {map.health.review_note && (
              <p className="rounded-lg border border-slate-100 dark:border-slate-800 bg-slate-50 dark:bg-slate-950/70 px-3 py-2 text-[11px] leading-5 text-slate-500 dark:text-slate-400">
                {map.health.review_note}
              </p>
            )}
            <p className="text-cap leading-5 text-slate-400 dark:text-slate-500">
              点击画布中的模块卡片、层标签或展开的子模块查看详情；选中模块后顶部工具栏可展开内部结构。
            </p>
          </div>
        )}
      </div>
      )}
    </aside>
  )
}
