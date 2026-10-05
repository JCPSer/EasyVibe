import { ToastHost, dismissToast, toast } from '@/lib/toast'
import { Component, useEffect, useMemo, useRef, useState, useCallback } from 'react'
import {
  ReactFlow,
  Controls,
  MiniMap,
  Panel,
  Position,
  MarkerType,
  useReactFlow,
  ReactFlowProvider,
  type Edge,
  type Node,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { Activity, AlertTriangle, Bot, FolderOpen, GitBranch, Loader2, PanelRightOpen, UnfoldVertical, FoldVertical, RefreshCw, Focus, Play, Pause, RotateCcw, X, Sparkles, Settings, Lightbulb, WifiOff, FileDown, Plus, Info, LayoutGrid, BookOpen, ScrollText, Plug, MessagesSquare, CircleHelp} from 'lucide-react'

import type { CodeMap, GrowthEvent, SubMap } from '@/types/map'
import { layoutMap, healthColor, NODE_W, NODE_H, SUB_W, SUB_H } from '@/lib/layout'
import { loadOnboarding, markCheck, markWelcomeShown, dismiss, completeAll, resetForReview, clearTaskIdea, loadTaskIdea, saveTaskIdea, prefersReducedMotion, CHECK_KEYS } from '@/lib/onboarding'
import { ONBOARDING_COPY } from '@/lib/onboardingCopy'
import { ModuleNode, type ModuleNodeType } from '@/components/ModuleNode'
import { BandNode, type BandNodeType } from '@/components/BandNode'
import { ExpandedModuleNode, type ExpandedModuleNodeType } from '@/components/ExpandedModuleNode'
import { SubmoduleNode, type SubmoduleNodeType } from '@/components/SubmoduleNode'
import { DetailPanel, type Selection, type PanelTab } from '@/components/DetailPanel'
import { AppShell, type PageId } from '@/components/AppShell'
import { PlaceholderPage } from '@/components/PlaceholderPage'
import { ModulesPage } from '@/components/ModulesPage'
import { WorkbenchPage } from '@/components/WorkbenchPage'
import { TaskPage } from '@/components/TaskPage'
import { DriftPage } from '@/components/DriftPage'
import { HealthPage } from '@/components/HealthPage'
import { ChangesPage } from '@/components/ChangesPage'
import { GitPage } from '@/components/GitPage'
import { ViewsPanel } from '@/components/ViewsPanel'
import { SuggestPanel } from '@/components/SuggestPanel'
import { SettingsPanel } from '@/components/SettingsPanel'
import { TaskFormPanel } from '@/components/TaskFormPanel'
import { ThemeToggle } from '@/components/ThemeToggle'
import { SessionBubble } from '@/components/SessionBubble'
import { DepsPage } from '@/components/DepsPage'
import { RunsPage } from '@/components/RunsPage'
import { UsagePage } from '@/components/UsagePage'
import { WelcomePage } from '@/components/WelcomePage'
import { OnboardingChecklist } from '@/components/OnboardingChecklist'
import type { TaskDraft } from '@/lib/taskContext'
import { isIssueModule } from '@/components/IssuesList'
import { emitFreshnessEvent, emitGrowthEvent, emitPatrolFinished, emitQueueChanged, emitSessionEvent, emitSessionOutput, emitTaskEvent, notifyWsClosed, onFreshnessEvent, onGrowthEvent, onPatrolFinished, onQueueChanged, onSessionEvent, onSessionOutput, onTaskEvent, setWsCloseListener } from '@/lib/growthBus'
import { enqueue } from '@/lib/sessionQueue'
import { sysNotify } from '@/lib/notify'
import { isTauriRuntime } from '@/lib/env'
import { pushTerminalLine } from '@/lib/terminalBuffer'
import { track } from '@/lib/analytics'
import { isValidGrowthEvent, mergeGrowthEvents, parseGrowthText } from '@/lib/growthMerge'
import { downloadHealthReport } from '@/lib/healthReport'
import { couplingAnalysis } from '@/lib/depsAnalysis'
import { initUpdater } from '@/lib/updater'

const nodeTypes = { module: ModuleNode, moduleExpanded: ExpandedModuleNode, submodule: SubmoduleNode, band: BandNode }

interface Filters {
  violationsOnly: boolean
  issuesOnly: boolean
  solo: boolean // 只看选中模块的依赖（强聚焦）
}

const MAX_EXPANDED = 3

interface GrowthState {
  events: GrowthEvent[]
  index: number // 已消费事件数
  playing: boolean
  done: boolean
}

function buildFlow(
  map: CodeMap,
  selection: Selection,
  filters: Filters,
  expanded: Map<string, SubMap | 'loading' | 'error'>,
  onSelectLayer: (id: string) => void,
  retrySubmap?: (id: string) => void,
  growth?: { layers: Set<string>; modules: Set<string> } | null,
  analyzeSubmap?: (id: string) => void,
  agentLinesFor?: (id: string) => string[] | undefined,
  analyzeErrorFor?: (id: string) => string | undefined,
  onCollapse?: (id: string) => void,
  onOpenRuns?: (sessionId: string) => void,
  submapSessionIdFor?: (id: string) => string | undefined,
) {
  // 展开元信息：加载中给 4 个骨架位；错误态不再给骨架（M4-1 诚实三态——此前 error 也渲染"分析中…"骨架，
  // 造成"头部报错 + 身体永远转圈"的撕裂画面）
  const expandedMeta = new Map<string, { ids: string[]; loading: boolean }>()
  for (const [id, sm] of expanded) {
    expandedMeta.set(id, sm === 'loading' ? { ids: ['__s0', '__s1', '__s2', '__s3'], loading: true } : sm === 'error' ? { ids: [], loading: false } : { ids: sm.sub_modules.map((s) => s.id), loading: false })
  }
  const { positions, bands, blocks } = layoutMap(map, expandedMeta)

  const inCount = new Map<string, number>()
  const outCount = new Map<string, number>()
  for (const e of map.edges) {
    outCount.set(e.from, (outCount.get(e.from) ?? 0) + 1)
    inCount.set(e.to, (inCount.get(e.to) ?? 0) + 1)
  }

  // 选中模块的邻域：自身 + 直接依赖/被依赖
  const neighborhood = new Set<string>()
  const selModuleId = selection?.kind === 'module' ? selection.id : selection?.kind === 'submodule' ? selection.parentId : null
  if (selModuleId) {
    neighborhood.add(selModuleId)
    for (const e of map.edges) {
      if (e.from === selModuleId) neighborhood.add(e.to)
      if (e.to === selModuleId) neighborhood.add(e.from)
    }
  }
  const issueIds = new Set(map.modules.filter(isIssueModule).map((m) => m.id))

  const nodeDim = (id: string) => {
    let op = 1
    if (selModuleId && !neighborhood.has(id)) op *= filters.solo ? 0.1 : 0.3
    if (filters.issuesOnly && !issueIds.has(id)) op *= 0.22
    return op
  }

  const nodes: Node[] = [
    ...bands.flatMap((box, i): BandNodeType[] => {
      const layer = map.layers.find((l) => l.id === box.layerId)!
      if (growth && !growth.layers.has(layer.id)) return []
      const mods = map.modules.filter((m) => m.layer === layer.id)
      const avgScore = Math.round(mods.reduce((s, m) => s + m.health.score, 0) / Math.max(mods.length, 1))
      const modIds = new Set(mods.map((m) => m.id))
      const violations = map.edges.filter(
        (e) => e.direction_violation && (modIds.has(e.from) || modIds.has(e.to)),
      ).length
      return [{
        id: `band-${box.layerId}`,
        type: 'band',
        position: { x: box.x, y: box.y },
        data: {
          layer, box, index: i,
          stats: { count: mods.length, avgScore, violations },
          selected: selection?.kind === 'layer' && selection.id === layer.id,
          onSelect: onSelectLayer,
        },
        draggable: false,
        selectable: false,
        zIndex: -1,
        width: box.width,
        height: box.height,
        className: growth ? 'growth-born' : undefined,
      }]
    }),
    ...map.modules.flatMap((mod): Node[] => {
      if (growth && !growth.modules.has(mod.id)) return []
      if (expanded.has(mod.id)) {
        const block = blocks.get(mod.id)!
        const sm = expanded.get(mod.id)!
        return [{
          id: mod.id,
          type: 'moduleExpanded',
          position: positions.get(mod.id)!,
          data: {
            module: mod,
            inCount: inCount.get(mod.id) ?? 0,
            outCount: outCount.get(mod.id) ?? 0,
            loading: sm === 'loading',
            error: sm === 'error',
            subCount: sm === 'loading' || sm === 'error' ? 0 : sm.sub_modules.length,
            onRetry: retrySubmap ? () => retrySubmap(mod.id) : undefined,
            onAnalyze: analyzeSubmap ? () => analyzeSubmap(mod.id) : undefined,
            agentLines: agentLinesFor?.(mod.id),
            analyzeError: analyzeErrorFor?.(mod.id),
            onCollapse: onCollapse ? () => onCollapse(mod.id) : undefined,
            analyzeSessionId: submapSessionIdFor?.(mod.id),
            onOpenRuns: onOpenRuns ? (sid: string) => onOpenRuns(sid) : undefined,
          },
          sourcePosition: Position.Bottom,
          targetPosition: Position.Top,
          width: block.width,
          height: block.height,
          style: { opacity: nodeDim(mod.id) },
          zIndex: 1,
          className: growth ? 'growth-born' : undefined,
        } satisfies ExpandedModuleNodeType]
      }
      return [{
        id: mod.id,
        type: 'module',
        position: positions.get(mod.id)!,
        data: { module: mod, inCount: inCount.get(mod.id) ?? 0, outCount: outCount.get(mod.id) ?? 0 },
        sourcePosition: Position.Bottom,
        targetPosition: Position.Top,
        width: NODE_W,
        height: NODE_H,
        style: { opacity: nodeDim(mod.id) },
        className: growth ? 'growth-born' : undefined,
      } satisfies ModuleNodeType]
    }),
    // 子模块节点（父节点内部，extent=parent）
    ...[...blocks.entries()].flatMap(([parentId, block]): Node[] => {
      const sm = expanded.get(parentId)!
      const parent = map.modules.find((m) => m.id === parentId)!
      const inDeg = new Map<string, number>()
      const outDeg = new Map<string, number>()
      if (sm !== 'loading' && sm !== 'error') {
        for (const e of sm.edges) {
          outDeg.set(e.from, (outDeg.get(e.from) ?? 0) + 1)
          inDeg.set(e.to, (inDeg.get(e.to) ?? 0) + 1)
        }
      }
      return block.childOrder.map((sid): Node => ({
        id: `sub:${parentId}:${sid}`,
        type: 'submodule',
        position: block.childPositions.get(sid)!,
        data: {
          sub: sm === 'loading' || sm === 'error' ? undefined : sm.sub_modules.find((s) => s.id === sid),
          loading: sm === 'loading' || sm === 'error',
          inCount: inDeg.get(sid) ?? 0,
          outCount: outDeg.get(sid) ?? 0,
          parentName: parent.name,
        },
        parentId,
        extent: 'parent',
        draggable: false,
        width: SUB_W,
        height: SUB_H,
      }) satisfies SubmoduleNodeType)
    }),
  ]

  const inIdx = new Map<string, number>()
  const outIdx = new Map<string, number>()

  const edgeDim = (touches: boolean, isViolation: boolean) => {
    let dim = 1
    if (selModuleId && !touches) dim *= filters.solo ? 0.04 : 0.07
    if (filters.violationsOnly && !isViolation) dim *= 0.06
    if (filters.issuesOnly && !touches) dim *= 0.5
    return dim
  }

  const visibleEdges = growth
    ? map.edges.filter((e) => growth.modules.has(e.from) && growth.modules.has(e.to))
    : map.edges
  // v2 依赖透镜：solo 聚焦态下邻域边按「相对层序方向」着色（口径单一来源 depsAnalysis；
  // 通道裁决 2026-10-05 评审 S1——色相=方向，strength 只留线宽差）
  const lens = filters.solo && selModuleId ? couplingAnalysis(map) : null
  const edges: Edge[] = visibleEdges.map((e, i) => {
    const violation = e.direction_violation === true
    const si = Math.min(outIdx.get(e.from) ?? 0, 17) // 与节点 MAX_HANDLES=18 对齐，超出复用最后一个 handle
    outIdx.set(e.from, (outIdx.get(e.from) ?? 0) + 1)
    const ti = Math.min(inIdx.get(e.to) ?? 0, 17)
    inIdx.set(e.to, (inIdx.get(e.to) ?? 0) + 1)
    const touches = e.from === selModuleId || e.to === selModuleId
    if (lens && selModuleId && touches) {
      const fo = lens.orderOf(e.from)
      const toOrder = lens.orderOf(e.to)
      // 违规红 > 环成员琥珀 > 同层蓝 > 顺层灰
      const stroke = violation ? '#ef4444' : lens.cycleModuleIds.has(e.from) && lens.cycleModuleIds.has(e.to) ? '#d97706' : fo === toOrder ? '#3b82f6' : '#64748b'
      const width = e.strength === 'strong' ? 2.5 : e.strength === 'normal' ? 1.8 : 1.2
      return {
        id: `e-${e.id ?? i}`,
        source: e.from,
        target: e.to,
        sourceHandle: `s${si}`,
        targetHandle: `t${ti}`,
        type: 'default',
        style: violation
          ? { strokeWidth: width, stroke, strokeDasharray: '6 4', opacity: 0.95 }
          : { strokeWidth: width, stroke, opacity: 0.9 },
        animated: violation,
        label: violation ? '⚠' : undefined,
        labelStyle: { fill: '#ef4444', fontSize: 13, fontWeight: 700 },
        labelBgStyle: { fill: 'rgba(255,255,255,0.9)', fillOpacity: 0.9 },
        markerEnd: { type: MarkerType.ArrowClosed, width: 14, height: 14, color: stroke },
        interactionWidth: 20,
      } satisfies Edge
    }
    const strengthStyle =
      e.strength === 'strong'
        ? { strokeWidth: 2.1, stroke: '#64748b', opacity: 0.85 }
        : e.strength === 'normal'
          ? { strokeWidth: 1.6, stroke: '#94a3b8', opacity: 0.7 }
          : { strokeWidth: 1.1, stroke: '#cbd5e1', opacity: 0.6 }
    const dim = edgeDim(e.from === selModuleId || e.to === selModuleId, violation)
    return {
      id: `e-${e.id ?? i}`,
      source: e.from,
      target: e.to,
      sourceHandle: `s${si}`,
      targetHandle: `t${ti}`,
      type: 'default',
      style: violation
        ? { strokeWidth: 1.8, stroke: '#ef4444', strokeDasharray: '6 4', opacity: 0.9 * dim }
        : { ...strengthStyle, opacity: strengthStyle.opacity * dim },
      animated: violation && dim > 0.5,
      label: violation && dim > 0.5 ? '⚠' : undefined,
      labelStyle: { fill: '#ef4444', fontSize: 13, fontWeight: 700 },
      labelBgStyle: { fill: 'rgba(255,255,255,0.9)', fillOpacity: 0.9 },
      markerEnd: { type: MarkerType.ArrowClosed, width: 14, height: 14, color: violation ? '#ef4444' : '#94a3b8' },
      interactionWidth: 20,
    } satisfies Edge
  })

  // 内部连线（展开模块的子图边）
  for (const [parentId, sm] of expanded) {
    if (sm === 'loading' || sm === 'error') continue
    const sin = new Map<string, number>()
    const sout = new Map<string, number>()
    sm.edges.forEach((e, i) => {
      const cyclic = e.circular_dep === true
      const si = Math.min(sout.get(e.from) ?? 0, 9) // 子模块节点 MAX_HANDLES=10
      sout.set(e.from, (sout.get(e.from) ?? 0) + 1)
      const ti = Math.min(sin.get(e.to) ?? 0, 9)
      sin.set(e.to, (sin.get(e.to) ?? 0) + 1)
      const touches = selection?.kind === 'submodule'
        ? (selection.parentId === parentId && (selection.subId === e.from || selection.subId === e.to))
        : selModuleId === parentId || !selModuleId
      let dim = touches ? 1 : selModuleId ? 0.08 : 1
      if (filters.violationsOnly) dim *= 0.15
      const base = cyclic
        ? { strokeWidth: 1.7, stroke: '#f97316', strokeDasharray: '5 4', opacity: 0.9 }
        : e.strength === 'strong'
          ? { strokeWidth: 1.8, stroke: '#64748b', opacity: 0.7 }
          : { strokeWidth: 1.3, stroke: '#94a3b8', opacity: 0.55 }
      edges.push({
        id: `se-${parentId}-${i}`,
        source: `sub:${parentId}:${e.from}`,
        target: `sub:${parentId}:${e.to}`,
        sourceHandle: `s${si}`,
        targetHandle: `t${ti}`,
        type: 'default',
        style: { ...base, opacity: base.opacity * dim },
        animated: cyclic && dim > 0.5,
        label: cyclic && dim > 0.5 ? '↻' : undefined,
        labelStyle: { fill: '#f97316', fontSize: 12, fontWeight: 700 },
        labelBgStyle: { fill: 'rgba(255,255,255,0.9)', fillOpacity: 0.9 },
        markerEnd: { type: MarkerType.ArrowClosed, width: 12, height: 12, color: cyclic ? '#f97316' : '#94a3b8' },
        zIndex: 5,
      })
    })
  }

  return { nodes, edges }
}

// M4-1.5 陪审团：图例默认收起为角落小条（此前大卡片压在画布黄金区），点击展开
function Legend({ violations }: { violations: number }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="rounded-xl border border-slate-200 dark:border-slate-700 bg-white/95 dark:bg-slate-900/95 px-3 py-2 text-[11px] text-slate-600 dark:text-slate-300 shadow-sm backdrop-blur">
      <button onClick={() => setOpen((v) => !v)} className="flex items-center gap-1.5 text-micro font-bold uppercase tracking-wider text-slate-400 dark:text-slate-500 hover:text-slate-600">
        <Info size={11} /> 图例{open ? ' ▴' : ' ▾'}
      </button>
      {open && (
        <div className="mt-1.5 space-y-1.5 anim-fade-in-fast">
          <div className="flex items-center gap-2">
            <span className="inline-block h-2.5 w-2.5 rounded-full" style={{ background: '#10b981' }} /> Healthy（≥75）
          </div>
          <div className="flex items-center gap-2">
            <span className="inline-block h-2.5 w-2.5 rounded-full" style={{ background: '#f59e0b' }} /> Warning（60–74）
          </div>
          <div className="flex items-center gap-2">
            <span className="inline-block h-2.5 w-2.5 rounded-full" style={{ background: '#ef4444' }} /> Error（&lt;60）
          </div>
          <div className="flex items-center gap-2">
            <span className="inline-block w-5 border-t-2 border-slate-400" /> Dependency
          </div>
          <div className="flex items-center gap-2">
            <span className="inline-block w-5 border-t-2 border-dashed border-red-500" />
            <span className="flex items-center gap-1">
              <AlertTriangle size={11} className="text-red-500" /> 逆向依赖 violation（{violations}）
            </span>
          </div>
          <div className="flex items-center gap-2">
            <span className="inline-block w-5 border-t-2 border-dashed border-orange-500" />
            <span className="flex items-center gap-1 text-orange-600">内部循环依赖（子图）</span>
          </div>
        </div>
      )}
    </div>
  )
}

function FilterButton({ active, onClick, label, activeClass }: { active: boolean; onClick: () => void; label: string; activeClass: string }) {
  return (
    <button
      onClick={onClick}
      className={`rounded-full border px-3 py-1 text-[11px] font-semibold transition-colors ${
        active ? activeClass : 'border-transparent text-slate-500 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-700/70'
      }`}
    >
      {label}
    </button>
  )
}


// 选中模块时的横向工具栏（F1a）
function ModuleToolbar({
  moduleName,
  expanded,
  backendActive,
  inducing,
  solo,
  onToggleExpand,
  onToggleSolo,
  onReinduce,
  onChat,
}: {
  moduleName: string
  expanded: boolean
  backendActive: boolean
  inducing: boolean
  solo: boolean
  onToggleExpand: () => void
  onToggleSolo: () => void
  onReinduce: () => void
  /** v0.2：就此模块对话——带上下文跳「任务对话」页 */
  onChat?: () => void
}) {
  return (
    <div className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 bg-white/95 dark:bg-slate-900/95 py-1.5 pl-4 pr-2 shadow-sm backdrop-blur">
      <span className="mr-1 max-w-[180px] truncate text-[12px] font-bold text-slate-700 dark:text-slate-200">{moduleName}</span>
      {onChat && (
        <button
          onClick={onChat}
          className="flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full border px-3 py-1 text-[11px] font-semibold transition-colors border-transparent text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700/70"
          title="就此模块发起对话：跳转「任务对话」并自动带入模块上下文"
        >
          <MessagesSquare size={12} />
          对话
        </button>
      )}
      <button
        onClick={onToggleExpand}
        className={`flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full border px-3 py-1 text-[11px] font-semibold transition-colors ${
          expanded ? 'border-blue-300 dark:border-blue-800 bg-blue-50 dark:bg-blue-950/40 text-blue-600' : 'border-transparent text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700/70'
        }`}
      >
        {expanded ? <FoldVertical size={12} /> : <UnfoldVertical size={12} />}
        {expanded ? '收起内部' : '展开内部结构'}
      </button>
      <button
        onClick={onToggleSolo}
        className={`flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full border px-3 py-1 text-[11px] font-semibold transition-colors ${
          solo ? 'border-indigo-300 dark:border-indigo-800 bg-indigo-50 dark:bg-indigo-950/40 text-indigo-600' : 'border-transparent text-slate-500 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-700/70'
        }`}
      >
        <Focus size={12} />
        {solo ? '✕ 退出聚焦' : '只看依赖'}
      </button>
      <button
        onClick={onReinduce}
        disabled={!backendActive || inducing}
        title={
          !backendActive
            ? '需要本地后端服务（cargo run 启动后可用）'
            : inducing
              ? '归纳进行中，完成后自动恢复'
              : '重新归纳该仓库：spawn agent 按 v2.2 协议执行，全程直播'
        }
        className={`flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full border px-3 py-1 text-[11px] font-semibold transition-colors ${
          inducing
            ? 'cursor-wait border-amber-300 dark:border-amber-800 bg-amber-50 dark:bg-amber-950/40 text-amber-700'
            : backendActive
              ? 'border-transparent text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-700/70'
              : 'cursor-not-allowed border-transparent text-slate-300 dark:text-slate-600'
        }`}
      >
        <RefreshCw size={12} className={inducing ? 'animate-spin' : ''} />
        {inducing ? '归纳中…' : '重新归纳'}
      </button>
    </div>
  )
}

// 生长回放控制条（消费 v2.2 growth.log 事件流）
function GrowthPanel({
  growth,
  onPause,
  onRestart,
  onExit,
}: {
  growth: GrowthState
  onPause: () => void
  onRestart: () => void
  onExit: () => void
}) {
  const total = growth.events.length
  const pct = total === 0 ? 0 : Math.round((growth.index / total) * 100)
  const cur = growth.index < growth.events.length ? growth.events[growth.index] : null
  const status = growth.done
    ? '归纳完成'
    : total === 0
      ? '等待生长事件…'
    : cur?.type === 'layer'
      ? `分层：${cur.layer.name}`
      : cur?.type === 'module'
        ? `正在分析模块：${cur.module.name}`
        : cur?.type === 'arch_health'
          ? '架构级健康评估'
          : '初始化'

  return (
    <div className="flex w-[460px] items-center gap-3 rounded-xl border border-slate-200 dark:border-slate-700 bg-white/95 dark:bg-slate-900/95 px-4 py-2.5 shadow-sm backdrop-blur">
      <button
        onClick={onPause}
        disabled={growth.done}
        className="rounded-full p-1.5 text-slate-500 dark:text-slate-400 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-700 disabled:opacity-30"
        title={growth.playing ? '暂停' : '继续'}
      >
        {growth.playing ? <Pause size={14} /> : <Play size={14} />}
      </button>
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline justify-between">
          <span className="truncate text-[12px] font-semibold text-slate-700 dark:text-slate-200">
            <Sparkles size={11} className="mr-1 inline text-blue-500" />
            {status}
          </span>
          <span className="text-micro tabular-nums text-slate-400 dark:text-slate-500">{pct}%</span>
        </div>
        <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800">
          <div
            className={`h-full rounded-full transition-all duration-500 ${growth.done ? 'bg-emerald-500' : 'bg-blue-500'}`}
            style={{ width: `${pct}%` }}
          />
        </div>
      </div>
      <button onClick={onRestart} className="rounded-full p-1.5 text-slate-400 dark:text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-600" title="重播">
        <RotateCcw size={13} />
      </button>
      <button onClick={onExit} className="rounded-full p-1.5 text-slate-400 dark:text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-600" title="退出演示">
        <X size={14} />
      </button>
    </div>
  )
}

function Canvas({
  map,
  backendRepo,
  onPatrollingChange,
  panelOpen,
  onPanelOpenChange,
  panelWidth,
  onPanelWidthChange,
  viewRequest,
  onViewRequestConsumed,
  guide,
  onTaskCreated,
  agentReady,
  onChatAbout,
  onGoWorkbench,
  onInspectEdge,
  onOpenRuns,
  onOpenDeps,
  lensRequest,
  onLensRequestConsumed,
  dark = false,
}: {
  map: CodeMap
  backendRepo: string | null
  /** M4-1：巡检终态回调（状态本体在壳层，顶栏按钮在 App） */
  onPatrollingChange: (v: boolean) => void
  /** M4-1：右栏开合/宽度由壳层持有（随项目持久化） */
  panelOpen: boolean
  onPanelOpenChange: (open: boolean) => void
  panelWidth: number
  onPanelWidthChange: (w: number) => void
  /** 视图定位请求（顶栏视图抽屉 → 画布聚焦），消费后回执 */
  viewRequest?: string[] | null
  onViewRequestConsumed?: () => void
  /** M4-1 旧入口引导卡（渲染在右栏上方，可关闭） */
  guide?: React.ReactNode
  /** 任务创建成功 → 壳层跳任务页（带新任务 id 选中） */
  onTaskCreated: (taskId: string) => void
  /** M2 降级：agent 缺失（false）时任务表单禁止提交 */
  agentReady: boolean
  /** v0.2：「就此对话」入口上抛（详情视图/模块工具栏/对话占位页签共用）——
   * 壳层转为跨页携带上下文跳「任务对话」 */
  onChatAbout?: (target: { refId: string; refName: string; kind: 'module' | 'layer' }) => void
  /** 2026-10-05 Redesign-A：右栏审批出口——QuickAsk 的审批角标/审批卡跳工作台裁决 */
  onGoWorkbench?: () => void
  /** 2026-10-05 依赖透镜：边浮卡 [详情] → 跳依赖体检页并聚焦对应卡片 */
  onInspectEdge?: (cardId: string) => void
  /** 2026-10-05 M4：画布「分析中」模块 → 运行页看该会话流水 */
  onOpenRuns?: (sessionId: string) => void
  /** 2026-10-05 右栏「耦合概览」入口（DetailPanel 上抛） */
  onOpenDeps?: () => void
  /** 2026-10-05 依赖透镜跳入：选中模块 + 打开 solo 聚焦 */
  lensRequest?: string | null
  onLensRequestConsumed?: () => void
  /** 2026-10-04 暗黑模式：小地图底色/遮罩主题感知 */
  dark?: boolean
}) {
  const [selection, setSelection] = useState<Selection>(null)
  // M4-1 瘦身：右栏只留 详情/问题/对话 三页签（v3 定稿顺序）；建议/视图移至顶栏抽屉，任务移至工作区页
  const [tab, setTab] = useState<PanelTab>('detail')
  // 改进#4：右栏可调宽（默认 340–560；对话页签放宽到 720，D1-C）
  const handleTabChange = useCallback(
    (t: PanelTab) => {
      setTab(t)
      // 离开对话页签时若宽度超出默认上限，收回（避免宽栏压窄其他页签内容）
      if (t !== 'chat') onPanelWidthChange(Math.min(panelWidth, 560))
    },
    [onPanelWidthChange, panelWidth],
  )
  const startPanelDrag = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault()
      const startX = e.clientX
      const startW = panelWidth
      const maxW = tab === 'chat' ? 720 : 560
      const onMove = (ev: MouseEvent) => onPanelWidthChange(Math.min(maxW, Math.max(340, startW + (startX - ev.clientX))))
      const onUp = () => {
        window.removeEventListener('mousemove', onMove)
        window.removeEventListener('mouseup', onUp)
      }
      window.addEventListener('mousemove', onMove)
      window.addEventListener('mouseup', onUp)
    },
    [panelWidth, tab, onPanelWidthChange],
  )
  const [filters, setFilters] = useState<Filters>({ violationsOnly: false, issuesOnly: false, solo: false })
  const [expandedIds, setExpandedIds] = useState<string[]>([])
  const [submaps, setSubmaps] = useState<Record<string, SubMap | 'loading' | 'error'>>({})
  const [growth, setGrowth] = useState<GrowthState | null>(null)
  const [liveActivity, setLiveActivity] = useState(false)
  const [inducing, setInducing] = useState(false)
  const [freshness, setFreshness] = useState<string | null>(null)
  // 改进#2：agent 过程直播——按会话存最近输出（子图分析/任务执行）
  const [agentLines, setAgentLines] = useState<Record<string, string[]>>({})
  // 子图分析会话号（模块 id → sessionId，用于匹配输出流）
  const [submapSessions, setSubmapSessions] = useState<Record<string, string>>({})
  // M4-1 诚实三态：分析错误原因（启动失败/会话失败/超时）按模块记录，UI 必须说人话
  const [submapErrors, setSubmapErrors] = useState<Record<string, string>>({})

  // 2026-10-04 实弹「渲染问题」修复：切仓库必须清掉上一仓库的画布状态——selection 是
  // keep-alive 的，旧仓库的模块 id 在新地图里不存在，nodeDim 的 neighborhood 判定会把
  // 新地图全部模块压到 0.3 透明度（整图洗白像蒙了层纱）；展开子图/过滤同理属于旧仓库上下文
  useEffect(() => {
    setSelection(null)
    setExpandedIds([])
    setSubmaps({})
    setSubmapErrors({})
    setSubmapSessions({})
    setFilters({ violationsOnly: false, issuesOnly: false, solo: false })
    setGrowth(null)
    setAgentLines({})
  }, [backendRepo])
  const [freshnessInfo, setFreshnessInfo] = useState<{ commitsSinceMap?: number | null }>({})
  const [taskDraft, setTaskDraft] = useState<TaskDraft | null>(null)
  // R3 B2：draft 序号——每次打开新表单 +1 作 key，强制重置组件实例，
  // 杜绝「留在画布」后再发起任务时复用旧实例（旧草稿+假"已创建"横幅残留）
  const [draftSeq, setDraftSeq] = useState(0)
  const openTaskDraft = (d: TaskDraft) => {
    setDraftSeq((s) => s + 1)
    setTaskDraft(d)
  }
  const growthRef = useRef<GrowthState | null>(null)
  const submapsRef = useRef<typeof submaps | null>(null)
  useEffect(() => {
    growthRef.current = growth
  }, [growth])
  useEffect(() => {
    submapsRef.current = submaps
  }, [submaps])

  // R2：WS 断线 → 退出生长模式（重连后由用户重新进入，startGrowth 拉全量对齐）
  useEffect(() => setWsCloseListener(() => setGrowth(null)), [])

  // 真人测试反馈#1："只看依赖"是单程票——Esc 退出 + 底部常驻指示条
  useEffect(() => {
    if (!filters.solo) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setFilters((f) => ({ ...f, solo: false }))
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [filters.solo])

  // 改进#2：订阅 agent 输出流，按会话保留最近 4 行。
  // P1 审查 2#13 性能修复：此前每个 stdout 行都 setAgentLines → buildFlow 全量重建（布局重算+全图闪烁）。
  // 现在两道闸：① 只接收"当前在地图上可见的子图分析会话"的行（任务执行的输出 TaskPanel 自己订阅）；
  // ② 500ms 尾随节流合并突发（分析高峰时行率可达数行/秒）。
  const submapSessionsRef = useRef<Record<string, string>>({})
  const agentBufferRef = useRef<Record<string, string[]>>({})
  const agentFlushTimerRef = useRef<number | undefined>(undefined)
  useEffect(() => {
    submapSessionsRef.current = submapSessions
    // 分析结束/重试的会话：其输出行不再展示，缓冲与状态同步剪枝
    const alive = new Set(Object.values(submapSessions))
    agentBufferRef.current = Object.fromEntries(Object.entries(agentBufferRef.current).filter(([k]) => alive.has(k)))
    setAgentLines((prev) => {
      const next = Object.fromEntries(Object.entries(prev).filter(([k]) => alive.has(k)))
      return Object.keys(next).length === Object.keys(prev).length ? prev : next
    })
  }, [submapSessions])
  useEffect(() => {
    const flush = () => {
      agentFlushTimerRef.current = undefined
      const buf = agentBufferRef.current
      if (Object.keys(buf).length === 0) return
      setAgentLines({ ...buf })
    }
    return onSessionOutput((e) => {
      const visible = Object.values(submapSessionsRef.current).includes(e.sessionId)
      if (!visible) return
      const cur = agentBufferRef.current[e.sessionId] ?? []
      agentBufferRef.current[e.sessionId] = [...cur.slice(-3), e.line]
      if (agentFlushTimerRef.current === undefined) {
        agentFlushTimerRef.current = window.setTimeout(flush, 500)
      }
    })
  }, [])

  // S2：地图保鲜——启动拉一次 + WS freshness.changed 增量（git 有新提交而地图未更新）
  useEffect(() => {
    if (!backendRepo) return
    fetch(`/api/repos/${backendRepo}/freshness`)      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: { status: string; commitsSinceMap?: number | null } }) => {
        setFreshness(d.data.status === 'fresh' ? null : d.data.status)
        setFreshnessInfo({ commitsSinceMap: d.data.commitsSinceMap })
      })
      .catch(() => {})
    return onFreshnessEvent((e) => {
      if (e.repo !== backendRepo) return
      setFreshness(e.status === 'fresh' ? null : e.status)
      setFreshnessInfo({ commitsSinceMap: e.commitsSinceMap })
    })
  }, [backendRepo])

  // 会话状态：终态（succeeded/failed）解除"归纳中"/"巡检中"（patrol 会话以 patrol- 前缀区分）
  useEffect(
    () =>
      onSessionEvent((evt) => {
        // M4-1 诚实三态：子图分析会话终态失败 → 立即报"会话失败"，不等轮询超时
        setSubmapSessions((prev) => {
          const hit = Object.entries(prev).find(([, sid]) => sid === evt.sessionId)
          if (hit && evt.status === 'failed') {
            setSubmapErrors((e) => ({ ...e, [hit[0]]: '分析会话失败（agent 未能完成内部结构分析）。可重试「深入分析」；多次失败请检查 LLM 配置。' }))
            setSubmaps((p) => ({ ...p, [hit[0]]: 'error' }))
            setSubmapSessions((prev) => {
              const n = { ...prev }
              delete n[hit![0]]
              return n
            })
          }
          return prev
        })
        if (evt.status !== 'succeeded' && evt.status !== 'failed') return
        setInducing(false)
        // 重归纳的直播生长会话诚实收尾：一个生长事件都没等到（agent 未按协议产出）→
        // 收起空面板，不假装播完；有事件的会话等自己的 done 事件自然结束
        if (growthFromReinduce.current) {
          growthFromReinduce.current = false
          setGrowth((g) => (g && g.events.length === 0 && !g.done ? null : g))
        }
        // R3 C1：stub 模式会话 id 带 patrol- 前缀可在此解除；真实模式（ind-N）统一走 patrol.finished 事件
        if (evt.sessionId.startsWith('patrol-')) onPatrollingChange(false)
      }),
    [],
  )

  // R3 C1：巡检终态事件——真实模式会话 id 是 ind-N，前缀判定永远等不到，此前"巡检中"永不解除
  useEffect(
    () =>
      onPatrolFinished((evt) => {
        if (evt.repo !== backendRepo) return
        onPatrollingChange(false)
      }),
    [backendRepo, onPatrollingChange],
  )

  // 直播订阅：WS 到达的 growth.event 追加进当前生长会话；未在生长模式则点亮"归纳活动"指示
  useEffect(
    () =>
      onGrowthEvent((event) => {
        if (!isValidGrowthEvent(event)) return // 坏消息直接丢弃（R1）
        const g = growthRef.current
        if (!g) {
          setLiveActivity(true)
          return
        }
        setGrowth({ ...g, events: [...g.events, event] })
      }),
    [],
  )
  const [headerExpanded, setHeaderExpanded] = useState(false)
  const { fitView, setCenter } = useReactFlow()

  // 生长回放：消费 growth.log（v2.2 协议），已到达的层/模块集合
  const arrived = useMemo(() => {
    if (!growth) return null
    const layers = new Set<string>()
    const modules = new Set<string>()
    for (let i = 0; i < growth.index && i < growth.events.length; i++) {
      const e = growth.events[i]
      if (e.type === 'layer') layers.add(e.layer.id)
      if (e.type === 'module') modules.add(e.module.id)
    }
    return { layers, modules }
  }, [growth])

  // 事件推进定时器：500ms 一帧；直播模式下新事件经总线追加后由同一节奏点亮
  useEffect(() => {
    if (!growth?.playing) return
    const t = setInterval(() => {
      setGrowth((g) => {
        if (!g || !g.playing) return g
        if (g.index >= g.events.length) {
          const last = g.events[g.events.length - 1]
          return last?.type === 'done' ? { ...g, playing: false, done: true } : g
        }
        return { ...g, index: g.index + 1 }
      })
    }, 500)
    return () => clearInterval(t)
  }, [growth?.playing])

  const startGrowth = useCallback(() => {
    const url = backendRepo ? `/api/repos/${backendRepo}/growth` : '/data/growth.log'
    fetch(url)
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.text()
      })
      .then((text) => {
        const events = parseGrowthText(text, !!backendRepo) // 坏行/坏消息跳过（Y1/R1）
        setSelection(null)
        setExpandedIds([])
        setLiveActivity(false)
        setGrowth({ events, index: 0, playing: true, done: false })
      })
      .catch(() => setGrowth(null))
  }, [backendRepo])

  const onSelectLayer = useCallback((id: string) => {
    setSelection({ kind: 'layer', id })
    setTab('detail')
    onPanelOpenChange(true)
  }, [])

  const toggleExpand = useCallback((id: string) => {
    setExpandedIds((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id].slice(-MAX_EXPANDED)))
    setSubmaps((prev) => (prev[id] ? prev : { ...prev, [id]: 'loading' }))
  }, [])

  // 写路径（M2-3）：触发重新归纳 → 后端 spawn agent 按 v2.2 执行；自动进入直播模式看生长
  const growthFromReinduce = useRef(false) // 直播会话标记：终态时诚实收尾（空事件 → 收起，不装播完）
  const startReinduce = useCallback(() => {
    if (!backendRepo || inducing) return
    // R7 清债：重新归纳 = spawn 全仓库 agent（数分钟 + LLM 成本），先确认
    if (!window.confirm('重新归纳将 spawn agent 全量分析仓库（通常数分钟），期间地图数据会被刷新。继续？')) return
    setInducing(true)
    fetch(`/api/repos/${backendRepo}/reinduce`, { method: 'POST' })
      .then((r) => {
        // S2：必须抛 Response 本体——catch 里要读 status/body 判别 409（与 analyzeSubmap 同因）
        if (!r.ok) throw r
        // 2026-10-04 实弹修复：此前调 startGrowth() 回放 growth.log 文件快照——那是上一次
        // 归纳的旧记录，几秒播完"done"，用户误以为归纳结束。改为开空直播会话，
        // WS 的 growth.event 随 agent 产出实时追加，直到 done 事件真正到达
        growthFromReinduce.current = true
        setGrowth({ events: [], index: 0, playing: true, done: false })
      })
      .catch(async (e) => {
        setInducing(false)
        // S2：409（单会话纪律）不再静默——入队，当前会话结束后自动接续
        if ((e as Response)?.status === 409) {
          const res = await enqueue(backendRepo, 'reinduce')
          if (res?.outcome === 'replaced')
            toast(`已加入队列：归纳将在当前会话结束后自动开始（已替换排队：${res.replacedLabel ?? '旧任务'}）`)
          else if (res?.outcome === 'queued') toast('已加入队列：归纳将在当前会话结束后自动开始')
          else if (res?.outcome === 'started') {
            // 竞态消解：入队裁决时已无活动会话，后端直接执行——走与手动相同的 armed 逻辑
            growthFromReinduce.current = true
            setGrowth({ events: [], index: 0, playing: true, done: false })
            setInducing(true)
            toast('已直接开始归纳')
          }
          return
        }
        toast('归纳启动失败（请确认后端在线后重试）。', 'error')
      })
  }, [backendRepo, inducing])


  // 懒加载子图：任何 loading 状态触发取数（后端模式走 API，否则静态文件）
  useEffect(() => {
    for (const [id, v] of Object.entries(submaps)) {
      if (v !== 'loading') continue
      const url = backendRepo ? `/api/repos/${backendRepo}/modules/${id}` : `/data/modules/${id}.json`
      fetch(url)
        .then((r) => {
          if (!r.ok) throw new Error(String(r.status))
          return r.json() as Promise<SubMap>
        })
        .then((d) => setSubmaps((p) => ({ ...p, [id]: d })))
        .catch(() => {
          // R4：失败保留展开态并标记 error（容器内显示重试），不再无声消失
          // M4-1.5 修：分析会话进行中（已启动未失败）时 404 只是产物未到——保持 loading，
          // 否则"分析中"秒变"错误"再跳回，用户以为失败了（试用实弹抓到）
          setSubmaps((p) => (submapSessions[id] ? p : { ...p, [id]: 'error' }))
        })
    }
  }, [submaps, backendRepo, submapSessions])

  const retrySubmap = useCallback((id: string) => {
    setSubmaps((prev) => ({ ...prev, [id]: 'loading' }))
  }, [])

  // 子图深入分析：派透明 agent 扫描模块文件生成子图，落盘后即可加载。
  // M4-1 诚实三态：① POST 失败 → 立即报"启动失败"；② 会话终态失败 → 立即报"会话失败"（不等轮询）；
  // ③ 轮询 6s×40=4 分钟无产物 → 报"超时"并停止。杜绝"永远转圈"。
  const analyzeSubmap = useCallback(
    (id: string) => {
      if (!backendRepo) return
      setSubmapErrors((prev) => ({ ...prev, [id]: '' }))
      fetch(`/api/repos/${backendRepo}/modules/${encodeURIComponent(id)}/analyze-submap`, { method: 'POST' })
        .then(async (r) => {
          // 必须抛 Response 本体：catch 里要读 body 判别 409（单会话纪律）——
          // 曾抛 new Error(status)，catch 的 (e as Response).json() 拿到 undefined，
          // 409 人话文案永远走不到，用户看到的是"后端离线"甩锅
          if (!r.ok) throw r
          const sess = (await r.json()) as { sessionId?: string; session_id?: string }
          const sid = sess.sessionId ?? sess.session_id ?? ''
          setSubmapSessions((prev) => ({ ...prev, [id]: sid }))
          setSubmaps((prev) => ({ ...prev, [id]: 'loading' }))
          let n = 0
          const t = window.setInterval(() => {
            n += 1
            const cur = submapsRef.current?.[id]
            if (cur && cur !== 'loading' && cur !== 'error') {
              window.clearInterval(t)
              return
            }
            if (n >= 40) {
              window.clearInterval(t)
              // 超时必须显式告之——此前静默停轮询，界面永远"分析中"
              setSubmapErrors((prev) => ({
                ...prev,
                [id]: '分析超时（4 分钟未产出内部结构）。可能是 agent 执行缓慢或失败，请重试「深入分析」。',
              }))
              setSubmaps((prev) => ({ ...prev, [id]: 'error' }))
              setSubmapSessions((prev) => {
                const n = { ...prev }
                delete n[id]
                return n
              })
              return
            }
            retrySubmap(id)
          }, 6000)
        })
        .catch(async (e) => {
          // 409=单会话纪律（另一个分析/归纳在跑）——一键入队，当前会话结束后自动接续
          let msg = '分析启动失败（请确认后端在线后重试）。'
          try {
            const body = await (e as Response)?.json?.()
            if (body?.code === 'CONFLICT' || /conflict|活动会话/.test(String(body?.error ?? ''))) {
              const res = await enqueue(backendRepo, 'submap', id)
              if (res?.outcome === 'replaced')
                msg = `已加入队列，当前会话结束后自动开始分析（已替换排队：${res.replacedLabel ?? '旧任务'}）`
              else if (res?.outcome === 'queued') msg = '已加入队列，当前会话结束后自动开始分析'
              else if (res?.outcome === 'started') msg = '分析已直接开始，稍候重新展开即可看到内部结构'
            }
          } catch { /* 保持默认文案 */ }
          setSubmapErrors((prev) => ({ ...prev, [id]: msg }))
          setSubmaps((prev) => ({ ...prev, [id]: 'error' }))
        })
    },
    [backendRepo, retrySubmap],
  )

  // I3：排队任务被排空（当前会话终态 → 后端自动接续）——前端走与手动入口相同的 armed 逻辑：
  // reinduce=空直播会话+归纳中；patrol=巡检中（真实解除仍靠 patrol.finished）；
  // submap=重新调用正常分析流程（此时会话可注册，自然进入既有 6s 轮询成功路径，I4 不重建状态机）
  useEffect(
    () =>
      onQueueChanged((evt) => {
        if (evt.repo !== backendRepo || evt.type !== 'drained' || !evt.job) return
        if (evt.job.kind === 'reinduce') {
          growthFromReinduce.current = true
          setGrowth({ events: [], index: 0, playing: true, done: false })
          setInducing(true)
          toast('排队任务已接续：归纳自动开始')
        } else if (evt.job.kind === 'patrol') {
          onPatrollingChange(true)
        } else if (evt.job.kind === 'submap' && evt.job.moduleId) {
          analyzeSubmap(evt.job.moduleId)
        }
      }),
    [backendRepo, analyzeSubmap, onPatrollingChange],
  )

  const expanded = useMemo(() => {
    const m = new Map<string, SubMap | 'loading' | 'error'>()
    for (const id of expandedIds) if (submaps[id]) m.set(id, submaps[id])
    return m
  }, [expandedIds, submaps])

  // 直播合并：生长事件的模块/层/边合入基准地图——map.json 在归纳完成前不含新模块，
  // 生长事件本身才是"进行中模块"的事实源。合并用全量事件（布局稳定），显隐由 arrived 控制。
  const mergedMap = useMemo(
    () => (growth ? mergeGrowthEvents(map, growth.events) : map),
    [map, growth?.events],
  )

  const emptyExpanded = useMemo(() => new Map<string, SubMap | 'loading' | 'error'>(), [])
  const effectiveExpanded = growth ? emptyExpanded : expanded
  const growthVisible = growth ? arrived : null

  const { nodes, edges } = useMemo(
    () => buildFlow(mergedMap, selection, filters, effectiveExpanded, onSelectLayer, retrySubmap, growthVisible, analyzeSubmap, (id) => agentLines[submapSessions[id] ?? ''], (id) => submapErrors[id], toggleExpand, onOpenRuns, (id) => submapSessions[id]),
    [mergedMap, selection, filters, effectiveExpanded, onSelectLayer, retrySubmap, growthVisible, analyzeSubmap, agentLines, submapSessions, submapErrors, toggleExpand],
  )

  // S1-1 画布定位收口：pan/zoom 到目标模块 + 选中高亮。
  // 对话 refs 芯片 / 问题清单定位 / 视图打开三处共用（此前只切右栏、画布毫无反馈）
  const focusModule = useCallback(
    (id: string) => {
      const node = nodes.find((n) => n.id === id)
      setSelection({ kind: 'module', id })
      setTab('detail')
      onPanelOpenChange(true)
      if (!node) return
      const w = node.measured?.width ?? node.width ?? 220
      setCenter(node.position.x + w / 2, node.position.y + 70, { zoom: 1.15, duration: 450 })
    },
    [nodes],
  )

  // S1-1 视图打开：定位首模块 + 多模块视图自动 solo 聚焦（复用 dim 机制，兑现跨模块视图）
  const openView = useCallback(
    (ids: string[]) => {
      if (ids.length === 0) return
      // 重置既有过滤（试用测试抓到：旧"只看依赖"会污染视图的 solo 聚焦）
      setFilters({ violationsOnly: false, issuesOnly: false, solo: ids.length > 1 })
      focusModule(ids[0])
    },
    [focusModule],
  )

  // M4-1 顶栏视图抽屉 → 画布定位：消费 App 下发的视图请求
  useEffect(() => {
    if (viewRequest && viewRequest.length > 0) {
      openView(viewRequest)
      onViewRequestConsumed?.()
    }
  }, [viewRequest, openView, onViewRequestConsumed])

  // 2026-10-05 依赖透镜跳入：选中 + solo 聚焦（DepsPage「在画布上看」/ 右栏「看全部」）
  useEffect(() => {
    if (!lensRequest) return
    if (mergedMap.modules.some((m) => m.id === lensRequest)) {
      focusModule(lensRequest)
      setFilters((f) => ({ ...f, solo: true }))
    }
    onLensRequestConsumed?.()
  }, [lensRequest, mergedMap, focusModule, onLensRequestConsumed])

  useEffect(() => {
    const t = setTimeout(() => {
      // M4-1.5 叙事重排（陪审团）：进图先给诊断——存在显著风险时聚焦最红的模块，
      // 让用户第一眼看到"哪里最疼"；健康项目才 fit 全景
      const worst = [...mergedMap.modules].sort((a, b) => a.health.score - b.health.score)[0]
      const risky = !!worst && (worst.health.score < 60 || violations > 0)
      if (risky) {
        const node = nodes.find((n) => n.id === worst!.id)
        if (node) {
          const w = node.measured?.width ?? node.width ?? 220
          setCenter(node.position.x + w / 2, node.position.y + 70, { zoom: 1.0, duration: 450 })
          return
        }
      }
      fitView({ padding: 0.12, duration: 300 })
    }, 60)
    return () => clearTimeout(t)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fitView, map])

  const onNodeClick = useCallback((_e: unknown, node: Node) => {
    if (growth) return // 生长回放期间禁用选中
    if (node.type === 'module' || node.type === 'moduleExpanded') {
      // 2026-10-04 实弹交互：已选中模块再点一次 = 展开内部结构（再点头部/工具栏收起），
      // 不用用户先选中再挪到工具栏找「展开内部结构」按钮
      const alreadySelected = selection?.kind === 'module' && selection.id === node.id
      if (node.type === 'module' && alreadySelected) toggleExpand(node.id)
      setSelection({ kind: 'module', id: node.id })
      setTab('detail')
      onPanelOpenChange(true)
    } else if (node.type === 'submodule' && node.parentId) {
      // 加载中的骨架不可选中（其 id 是占位符，选中后数据到达会无法匹配）
      if ((node.data as { loading?: boolean }).loading) return
      const subId = node.id.slice(`sub:${node.parentId}:`.length)
      setSelection({ kind: 'submodule', parentId: node.parentId, subId })
      setTab('detail')
      onPanelOpenChange(true)
    }
  }, [growth, selection, toggleExpand, setTab, onPanelOpenChange])
  const onPaneClick = useCallback(() => setSelection(null), [])

  const toggleFilter = (key: keyof Filters) => setFilters((f) => ({ ...f, [key]: !f[key] }))

  // 2026-10-05 依赖透镜：边浮卡——fixed 定位跟随鼠标；命中区由 ReactFlow interactionWidth 加宽（评审 D6：裸边 1-2px 命中率极差）
  const [edgeTip, setEdgeTip] = useState<{ edgeId: string; x: number; y: number } | null>(null)
  const tipEdge = edgeTip ? mergedMap.edges.find((e) => `e-${e.id}` === edgeTip.edgeId) : undefined

  const violations = mergedMap.edges.filter((e) => e.direction_violation).length
  // 工具栏对模块选中与其子模块选中都生效（收起/展开操作的是父模块）
  const toolbarModuleId = selection?.kind === 'module' ? selection.id : selection?.kind === 'submodule' ? selection.parentId : null
  const selModule = toolbarModuleId ? map.modules.find((m) => m.id === toolbarModuleId) : undefined

  return (
    <div className="flex h-full w-full overflow-hidden bg-slate-50 dark:bg-slate-950/70">
      {/* 中央画布 */}
      <div className="relative flex-1">
        {/* ui-test P2：xyflow 的 <Background> 在 store transform 未就绪的更新周期里把
            cx/cy/r/pattern x/y 算成 NaN（库内部行为，挂载时机绕不过）。
            改用 CSS 径向渐变点阵：视觉等价、零 SVG、零 NaN（暗色点阵在 index.css 的 .bg-dots 里）。 */}
        <div aria-hidden className="bg-dots pointer-events-none absolute inset-0 z-0" />
        <ReactFlow
          colorMode={dark ? 'dark' : 'light'}
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onNodeClick={onNodeClick}
          onPaneClick={onPaneClick}
          nodesDraggable={false}
          minZoom={0.08}
          maxZoom={1.6}
          proOptions={{ hideAttribution: true }}
          nodesConnectable={false}
          deleteKeyCode={null}
          onEdgeMouseEnter={(ev, edge) => setEdgeTip({ edgeId: edge.id, x: ev.clientX, y: ev.clientY })}
          onEdgeMouseMove={(ev, edge) =>
            setEdgeTip((t) => (t && t.edgeId === edge.id ? { ...t, x: ev.clientX, y: ev.clientY } : t))
          }
          onEdgeMouseLeave={() => setEdgeTip(null)}
        >
          <Controls showInteractive={false} position="bottom-left" />
          <MiniMap
            position="bottom-right"
            pannable
            zoomable
            bgColor={dark ? '#0f172a' : '#f8fafc'}
            nodeColor={(n) =>
              n.type === 'module' || n.type === 'moduleExpanded'
                ? healthColor((n.data as { module: { health: { score: number } } }).module.health.score)
                : 'rgba(0,0,0,0)'
            }
            maskColor={dark ? 'rgba(2,6,23,0.72)' : 'rgba(226,232,240,0.7)'}
            style={{ width: 200, height: 130 }}
          />
          {/* 右上角：图例（M4-1.5 去挤：架构健康主视觉已入头部卡，原 ArchHealthCard 信息重复，撤下） */}
          <Panel position="top-right" className="flex flex-col gap-2">
            <Legend violations={violations} />
          </Panel>

          {/* 顶部中央：选中模块的横向工具栏（F1a）；mt 让出头部卡片高度（展开简介时更高），窄屏不遮挡 */}
          {selModule && (
            <Panel position="top-center" style={{ marginTop: headerExpanded ? 215 : 125 }}>
              <ModuleToolbar
                moduleName={selModule.name}
                expanded={expandedIds.includes(selModule.id)}
                backendActive={!!backendRepo}
                inducing={inducing}
                solo={filters.solo}
                onToggleExpand={() => toggleExpand(selModule.id)}
                onToggleSolo={() => toggleFilter('solo')}
                onReinduce={startReinduce}
                onChat={onChatAbout ? () => onChatAbout({ refId: selModule.id, refName: selModule.name, kind: 'module' }) : undefined}
              />
            </Panel>
          )}

          {/* 底部中央：生长回放控制条 或 全局过滤 */}
          <Panel position="bottom-center" className="mb-2">
            {growth ? (
              <GrowthPanel
                growth={growth}
                onPause={() => setGrowth((g) => (g ? { ...g, playing: !g.playing } : g))}
                onRestart={() => setGrowth((g) => (g ? { ...g, index: 0, playing: true, done: false } : g))}
                onExit={() => setGrowth(null)}
              />
            ) : (
              <div className="flex items-center gap-1.5 rounded-full border border-slate-200 dark:border-slate-700 bg-white/95 dark:bg-slate-900/95 px-2 py-1.5 shadow-sm backdrop-blur">
                {/* 真人测试#1：聚焦常驻指示——任何时刻看得见、一键退得出（Esc 同效） */}
                {filters.solo && selModule && (
                  <button
                    onClick={() => setFilters((f) => ({ ...f, solo: false }))}
                    className="flex items-center gap-1 rounded-full border border-indigo-300 dark:border-indigo-800 bg-indigo-50 dark:bg-indigo-950/40 px-2.5 py-1 text-cap font-semibold text-indigo-600 hover:bg-indigo-100"
                    title="退出聚焦（Esc）"
                  >
                    <Focus size={10} />
                    聚焦：{selModule.name} <span className="text-indigo-400">✕</span>
                  </button>
                )}
                <FilterButton
                  active={filters.violationsOnly}
                  onClick={() => toggleFilter('violationsOnly')}
                  label="只看违规"
                  activeClass="border-red-300 bg-red-50 dark:bg-red-950/40 text-red-600"
                />
                <FilterButton
                  active={filters.issuesOnly}
                  onClick={() => toggleFilter('issuesOnly')}
                  label="问题视图"
                  activeClass="border-amber-300 dark:border-amber-800 bg-amber-50 dark:bg-amber-950/40 text-amber-700"
                />
                {(filters.violationsOnly || filters.issuesOnly) && (
                  <button
                    onClick={() => setFilters({ violationsOnly: false, issuesOnly: false, solo: filters.solo })}
                    className="rounded-full px-2 py-1 text-cap text-slate-400 dark:text-slate-500 hover:text-slate-600"
                  >
                    重置
                  </button>
                )}
              </div>
            )}
          </Panel>
        </ReactFlow>

        {/* 依赖透镜：边浮卡（hover 主图边 → 一行结论 + [详情] 跳体检页） */}
        {tipEdge && edgeTip && (
          <div
            className="glass fixed z-50 w-60 rounded-xl border border-slate-200 dark:border-slate-700 bg-white/95 dark:bg-slate-900/95 p-3 shadow-lg"
            style={{ left: edgeTip.x + 14, top: edgeTip.y + 14 }}
          >
            <p className="text-[12px] font-bold text-slate-800 dark:text-slate-100">
              {mergedMap.modules.find((m) => m.id === tipEdge.from)?.name ?? tipEdge.from}
              <span className="mx-1 text-slate-300 dark:text-slate-600">→</span>
              {mergedMap.modules.find((m) => m.id === tipEdge.to)?.name ?? tipEdge.to}
            </p>
            <p className="mt-0.5 text-micro text-slate-400 dark:text-slate-500">
              {tipEdge.type} · {tipEdge.label ?? '1 处引用'}
              {tipEdge.direction_violation && <span className="text-red-500"> · ⚠ 逆向</span>}
            </p>
            {tipEdge.direction_violation && onInspectEdge && (
              <button
                onClick={() => {
                  onInspectEdge(`vio-${tipEdge.id}`)
                  setEdgeTip(null)
                }}
                className="mt-2 rounded-md bg-blue-50 dark:bg-blue-950/40 px-2 py-1 text-micro font-bold text-blue-600 hover:bg-blue-100 dark:hover:bg-blue-900/40"
              >
                详情 →
              </button>
            )}
          </div>
        )}

        {/* 头部信息条（M4-1：全局组件已移至应用壳顶栏，此处仅保留地图本地信息） */}
        <div className="pointer-events-none absolute left-0 top-0 z-10 w-full">
          <div className="px-5 py-3">
            <div className="pointer-events-auto inline-block rounded-xl border border-slate-200 dark:border-slate-700 bg-white/95 dark:bg-slate-900/95 px-4 py-2.5 shadow-sm backdrop-blur">
              <div className="flex items-center gap-2">
                <span className="text-micro font-bold uppercase tracking-widest text-blue-600">架构地图</span>
                <span className="text-micro text-slate-300 dark:text-slate-600">|</span>
                <h1 className="text-[13px] font-bold text-slate-800 dark:text-slate-100">{map.meta.repo}</h1>
                {/* M4-1.5 叙事重排（陪审团）：健康分立为画布内主视觉——先给诊断，再给地图 */}
                <button
                  onClick={() => {
                    setTab('issues')
                    onPanelOpenChange(true)
                  }}
                  className="ml-3 flex items-center gap-1.5 rounded-lg border border-slate-100 dark:border-slate-800 bg-slate-50 dark:bg-slate-950/70 pl-1.5 pr-2 py-0.5 hover:border-blue-200 hover:bg-blue-50/60"
                  title="架构健康综合评分（点击在右栏查看全部问题）"
                >
                  <span className="tnum text-[20px] font-black leading-6" style={{ color: healthColor(map.health.score) }}>
                    {map.health.score}
                  </span>
                  <span className="flex flex-col items-start leading-none">
                    <span className="text-micro font-semibold text-slate-400 dark:text-slate-500">架构健康</span>
                    {/* 评审 Y7：阈值与 healthColor/Legend 对齐——<60 红（Error）、60-74 amber（Warning），此前 70 分吃红色误报 */}
                    {map.health.score < 60 ? (
                      <span className="mt-0.5 text-micro font-semibold text-red-500">有问题 · 查看 →</span>
                    ) : map.health.score < 75 ? (
                      <span className="mt-0.5 text-micro font-semibold text-amber-500">有改进空间 · 查看 →</span>
                    ) : null}
                  </span>
                </button>
              </div>
              <button
                onClick={() => setHeaderExpanded((v) => !v)}
                className="mt-0.5 block max-w-[520px] text-left"
                title={headerExpanded ? '收起简介' : '展开简介'}
              >
                <p className={`text-[11px] text-slate-500 dark:text-slate-400 ${headerExpanded ? 'max-h-28 overflow-y-auto' : 'truncate'}`}>
                  {map.meta.description}
                  <span className="ml-1 text-micro font-medium text-blue-400">
                    {headerExpanded ? '▲ 收起' : '▼ 展开'}
                  </span>
                </p>
              </button>
              <div className="mt-1 flex items-center gap-3 text-cap text-slate-400 dark:text-slate-500">
                <span className="flex items-center gap-1">
                  <GitBranch size={11} /> {map.meta.generator}
                </span>
                <span>{mergedMap.modules.length} 模块</span>
                <span>{mergedMap.layers.length} 层</span>
                <span>{mergedMap.edges.length} 依赖</span>
                <span className="text-red-500">{violations} 逆向</span>
                {!backendRepo && (
                  <span
                    className="flex items-center gap-1 rounded-full bg-amber-100 px-1.5 py-px font-semibold text-amber-700"
                    title="后端不在线：当前为静态演示数据，重新归纳/巡检/任务不可用"
                  >
                    <WifiOff size={9} /> 演示数据 · 后端离线
                  </span>
                )}
                {freshness && (
                  <span
                    className={`flex items-center gap-1 rounded-full px-1.5 py-px font-semibold ${
                      freshness === 'stale' ? 'bg-red-100 text-red-700 dark:bg-red-950/60 dark:text-red-300' : 'bg-amber-100 text-amber-700 dark:bg-amber-950/60 dark:text-amber-300'
                    }`}
                    title={`git 有 ${freshnessInfo.commitsSinceMap ?? '?'} 个提交在地图生成之后——对话/建议/健康分可能基于过时信息`}
                  >
                    <AlertTriangle size={9} />
                    地图已过时 · {freshness === 'stale' ? '建议重新归纳' : `${freshnessInfo.commitsSinceMap ?? '?'} 个新提交未归纳`}
                    {/* 2026-10-04 实弹：只摆问题不给出路是死胡同——drifting/stale 两档都挂行动按钮 */}
                    {backendRepo && !inducing && (
                      <button
                        onClick={startReinduce}
                        className="ml-0.5 flex items-center gap-0.5 rounded-full bg-white/80 dark:bg-slate-800/80 px-1.5 py-px text-micro font-bold text-red-600 dark:text-amber-300 shadow-sm transition-colors hover:bg-white dark:hover:bg-slate-700"
                        title="立即重新归纳：agent 按 v2.2 协议重跑，全程直播"
                      >
                        <RefreshCw size={8} /> 立即归纳
                      </button>
                    )}
                  </span>
                )}
                {inducing && (
                  <span className="flex items-center gap-1 font-semibold text-amber-600">
                    <RefreshCw size={10} className="animate-spin" />
                    归纳中…
                  </span>
                )}
                <button
                  onClick={startGrowth}
                  disabled={!!growth}
                  className={`ml-1 flex items-center gap-1 rounded-full border px-2 py-0.5 font-semibold transition-colors disabled:opacity-40 ${
                    liveActivity && !growth
                      ? 'border-red-300 bg-red-50 dark:bg-red-950/40 text-red-600 animate-pulse'
                      : 'border-blue-200 dark:border-blue-900/60 bg-blue-50 dark:bg-blue-950/40 text-blue-600 hover:bg-blue-100'
                  }`}
                  title={backendRepo ? '观看实时生长（直播 growth.log 事件）' : '回放归纳过程（静态 growth.log）'}
                >
                  <Play size={10} />
                  {liveActivity && !growth ? '归纳活动 · 观看生长' : '生长演示'}
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
      {/* 右侧详情面板（M4-1：三页签 详情/问题/对话 + 顶部旧入口引导卡） */}
      {panelOpen ? (
        <>
        {/* 改进#4：右栏宽度拖拽手柄 */}
        <div
          onMouseDown={startPanelDrag}
          className="w-1 shrink-0 cursor-col-resize bg-slate-100 dark:bg-slate-800 transition-colors hover:bg-blue-300"
          title="拖拽调整面板宽度"
        />
        <div className="flex w-full shrink-0 flex-col" style={{ width: panelWidth }}>
          {guide}
          <div className="min-h-0 flex-1">
            <DetailPanel
              map={map}
              selection={selection}
              tab={tab}
              onTabChange={handleTabChange}
              submaps={submaps}
              backendRepo={backendRepo}
              onCreateTask={openTaskDraft}
              onLocateModule={focusModule}
              onOpenView={openView}
              onChatAbout={onChatAbout}
              onGoWorkbench={onGoWorkbench}
              onOpenDeps={onOpenDeps}
              onClose={() => onPanelOpenChange(false)}
              width={panelWidth}
            />
          </div>
        </div>
        </>
      ) : (
        <button
          onClick={() => panelOpen === false && onPanelOpenChange(true)}
          className="flex w-9 shrink-0 flex-col items-center gap-2 border-l border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 py-4 text-slate-400 dark:text-slate-500 hover:text-blue-600"
          title="展开面板"
        >
          <PanelRightOpen size={15} />
          <span className="text-micro [writing-mode:vertical-rl]">{selection ? '详情' : '面板'}</span>
        </button>
      )}

      {/* 任务表单（指哪打哪：模块/问题/层入口预填）；创建成功后跳任务页（由壳接管） */}
      {taskDraft && (
        <TaskFormPanel
          key={`canvas-draft-${draftSeq}`}
          backendRepo={backendRepo}
          draft={taskDraft}
          map={map}
          onClose={() => setTaskDraft(null)}
          onCreated={onTaskCreated}
          onLocateModule={focusModule}
          agentReady={agentReady}
        />
      )}
    </div>
  )
}

// R1 兜底：渲染异常拦截，白屏换成可恢复提示
class CanvasBoundary extends Component<{ children: React.ReactNode }, { err: Error | null }> {
  state = { err: null as Error | null }
  static getDerivedStateFromError(err: Error) {
    return { err }
  }
  render() {
    if (this.state.err) {
      return (
        <div className="flex h-screen flex-col items-center justify-center gap-2 text-[13px] text-slate-500 dark:text-slate-400">
          <span className="font-semibold text-slate-700 dark:text-slate-200">页面渲染出错（已拦截白屏）</span>
          <span className="max-w-[420px] text-center text-slate-400 dark:text-slate-500">{String(this.state.err).slice(0, 200)}</span>
          <button className="rounded-lg border border-slate-200 dark:border-slate-700 px-3 py-1.5 text-blue-600 hover:bg-slate-50 dark:hover:bg-slate-800/70" onClick={() => location.reload()}>
            刷新恢复
          </button>
        </div>
      )
    }
    return this.props.children
  }
}

// 首归纳等待页：轮询 progress.json 展示真实阶段与百分比（"正在边推导 80%"而非干转圈）
function InductionWaiting({ repo }: { repo: string }) {  const [prog, setProg] = useState<{ phase: string; percent: number; modulesDone: number; modulesTotal: number } | null>(null)
  // 2026-10-04 新手引导：等待期是引导黄金时间（调研 B 节）——三层递进：
  // 真实进度叙事（原有）+ 概念卡片轮播 + 任务想法预填（归纳完成后带入任务对话）
  const [cardIdx, setCardIdx] = useState(0)
  const [idea, setIdea] = useState(() => loadTaskIdea() ?? '')
  const [ideaSaved, setIdeaSaved] = useState(false)
  const reduced = prefersReducedMotion()
  useEffect(() => {
    if (reduced) return // 尊重减弱动效：不自动轮播，手动翻页即可
    const t = window.setInterval(() => setCardIdx((i) => (i + 1) % ONBOARDING_COPY.concepts.length), 20000)
    return () => window.clearInterval(t)
  }, [reduced])
  useEffect(() => {
    let stale = false
    const tick = () => {
      fetch(`/api/repos/${repo}/progress`)
        .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
        .then((d: { data: { phase: string; percent: number; modules_done: number; modules_total: number } | null }) => {
          if (stale || !d.data) return
          setProg({ phase: d.data.phase, percent: d.data.percent, modulesDone: d.data.modules_done, modulesTotal: d.data.modules_total })
        })
        .catch(() => {})
    }
    tick()
    const t = window.setInterval(tick, 2000)
    return () => {
      stale = true
      window.clearInterval(t)
    }
  }, [repo])

  const PHASE_LABEL: Record<string, string> = {
    init: '准备中',
    layering: '分层分析',
    module_scan: '模块扫描',
    emit: '模块归纳',
    edging: '依赖边推导',
    consistency: '一致性校验',
    finalize: '收尾写盘',
    done: '完成',
  }
  const concept = ONBOARDING_COPY.concepts[cardIdx]
  return (
    <div className="flex h-screen flex-col items-center justify-center gap-4 px-6 text-[13px] text-slate-500 dark:text-slate-400">
      {/* 第 1 层：真实进度叙事（永远不让等待页只有 spinner） */}
      <Loader2 size={18} className="animate-spin text-blue-500" />
      <span className="font-semibold text-slate-700 dark:text-slate-200">
        正在归纳代码地图{prog ? `：${PHASE_LABEL[prog.phase] ?? prog.phase} ${prog.percent}%` : '…'}
      </span>
      {prog && (
        <div className="h-1.5 w-64 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800" role="progressbar" aria-valuenow={prog.percent} aria-valuemin={0} aria-valuemax={100}>
          <div className="h-full rounded-full bg-blue-500 transition-[width] duration-500" style={{ width: `${prog.percent}%` }} />
        </div>
      )}
      <span className="max-w-[420px] text-center text-[11px] leading-4 text-slate-400 dark:text-slate-500">
        {prog && prog.modulesTotal > 0
          ? `已归纳 ${prog.modulesDone}/${prog.modulesTotal} 个模块`
          : '后台 agent 执行中（通常数分钟，取决于仓库规模）'}
        ，完成后地图会自动出现
      </span>

      {/* 第 2 层：概念卡片轮播（每张 ~20s 自动翻，可手动点；key 驱动翻页淡入——评审 G7） */}
      <div className="w-full max-w-md rounded-xl border border-slate-100 dark:border-slate-800 bg-white/90 dark:bg-slate-900/90 px-4 py-3 shadow-sm">
        <div key={cardIdx} className="anim-fade-in-fast">
        <div className="flex items-center justify-between">
          <p className="text-[12px] font-bold text-slate-700 dark:text-slate-200">{concept.title}</p>
          <div className="flex gap-1">
            {ONBOARDING_COPY.concepts.map((c, i) => (
              <button
                key={c.id}
                onClick={() => setCardIdx(i)}
                aria-label={`第 ${i + 1} 张：${c.title}`}
                className={`h-1.5 rounded-full transition-all ${i === cardIdx ? 'w-4 bg-blue-500' : 'w-1.5 bg-slate-200 hover:bg-slate-300'}`}
              />
            ))}
          </div>
        </div>
        <p className="mt-1.5 text-[11.5px] leading-5 text-slate-500 dark:text-slate-400">{concept.body}</p>
        </div>
      </div>

      {/* 第 3 层：提前参与——任务想法预填（归纳完成后带入任务对话） */}
      <div className="w-full max-w-md">
        <p className="text-[11px] font-semibold text-slate-500 dark:text-slate-400">{ONBOARDING_COPY.waiting.ideaTitle}</p>
        <div className="mt-1 flex gap-1.5">
          <input
            value={idea}
            onChange={(e) => { setIdea(e.target.value); setIdeaSaved(false) }}
            placeholder={ONBOARDING_COPY.waiting.ideaPlaceholder}
            className="min-w-0 flex-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1.5 text-[12px] outline-none transition-colors focus:border-blue-300 focus:ring-2 focus:ring-blue-100"
          />
          <button
            onClick={() => { saveTaskIdea(idea.trim()); setIdeaSaved(true) }}
            disabled={!idea.trim()}
            className="shrink-0 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1.5 text-[11px] font-semibold text-slate-500 dark:text-slate-400 hover:border-blue-300 hover:text-blue-600 disabled:opacity-40"
          >
            {ONBOARDING_COPY.waiting.ideaButton}
          </button>
        </div>
        {ideaSaved && <p className="mt-1 text-[10.5px] text-emerald-600">{ONBOARDING_COPY.waiting.ideaSaved}</p>}
      </div>
    </div>
  )
}

// P0 对标缺口#3：系统通知惰性封装——首次需要时才申请权限；不支持的宿主静默降级
function notifySystem(title: string, body: string) {
  if (typeof Notification === 'undefined') return
  if (Notification.permission === 'granted') {
    new Notification(title, { body })
  } else if (Notification.permission === 'default') {
    void Notification.requestPermission().then((p) => {
      if (p === 'granted') new Notification(title, { body })
    })
  }
}

// P0 审查前端#1：地图加载守门员——区分三种真实状态，消灭"出错也转圈"死锁：
// · progress 显示归纳进行中 → 等待页（原行为）
// · 加载出错（5xx/网络/后端离线） → 错误卡（重试 / 开始归纳）
// · 从未生成且无归纳（progress 无文件） → 引导卡（开始归纳）
function MapGate({ repo, error, onRetry, agentReady }: { repo: string; error: string | null; onRetry: () => void; agentReady: boolean }) {
  const [inducing, setInducing] = useState<boolean | null>(null) // null=探测中
  const [starting, setStarting] = useState(false)

  useEffect(() => {
    let stale = false
    const tick = () => {
      fetch(`/api/repos/${encodeURIComponent(repo)}/progress`)
        .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
        .then((d: { data: { phase: string } | null }) => {
          if (stale) return
          setInducing(!!d.data && d.data.phase !== 'done')
        })
        .catch(() => {
          if (!stale) setInducing(false) // progress 也拿不到 → 按出错处理
        })
    }
    tick()
    const t = window.setInterval(tick, 3000)
    return () => {
      stale = true
      window.clearInterval(t)
    }
  }, [repo])

  if (inducing === null) {
    return (
      <div className="flex h-screen items-center justify-center gap-2 text-[13px] text-slate-500 dark:text-slate-400">
        <Loader2 size={16} className="animate-spin" /> 正在探测仓库状态…
      </div>
    )
  }
  if (inducing) return <InductionWaiting repo={repo} />

  const startInduce = async () => {
    setStarting(true)
    try {
      const r = await fetch(`/api/repos/${encodeURIComponent(repo)}/reinduce`, { method: 'POST' })
      const d = await r.json().catch(() => null)
      if (!r.ok) {
        toast(d?.error ?? '归纳发起失败', 'error')
        setStarting(false)
        return
      }
      setInducing(true)
    } catch {
      toast('归纳发起失败（需要后端在线）', 'error')
      setStarting(false)
    }
  }

  return (
    <div className="flex h-screen flex-col items-center justify-center gap-3 text-[13px] text-slate-500 dark:text-slate-400">
      <span className="font-semibold text-slate-700 dark:text-slate-200">{error ? '代码地图加载失败' : '该仓库尚未生成代码地图'}</span>
      {error && <span className="max-w-[460px] text-center text-[11px] leading-4 text-slate-400 dark:text-slate-500">{String(error).slice(0, 200)}</span>}
      {!error && <span className="text-[11px] text-slate-400 dark:text-slate-500">发起归纳后，EasyVibe 的 agent 会扫描仓库并生成架构地图（通常数分钟）</span>}
      <div className="flex gap-2">
        {error && (
          <button
            onClick={onRetry}
            className="flex items-center gap-1.5 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-3 py-1.5 text-[12px] font-semibold text-slate-600 dark:text-slate-300 hover:bg-slate-50 dark:hover:bg-slate-800/70"
          >
            <RotateCcw size={12} /> 重试
          </button>
        )}
        <button
          onClick={startInduce}
          disabled={starting || !agentReady}
          title={agentReady ? undefined : '未检测到执行 agent——先安装或在设置中配置'}
          className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-[12px] font-semibold text-white hover:bg-blue-700 disabled:opacity-40"
        >
          {starting ? <Loader2 size={12} className="animate-spin" /> : <Play size={12} />} 开始归纳
        </button>
      </div>
    </div>
  )
}

export default function App() {
  const [map, setMap] = useState<CodeMap | null>(null)
  const [error, setError] = useState<string | null>(null)
  // 后端模式：探测 /api/health 成功且仓库列表非空则启用；失败降级静态 demo 数据
  const [backendRepo, setBackendRepo] = useState<string | null>(null)
  const [repos, setRepos] = useState<{ id: string; name: string }[]>([])
  // 重审 P2 bug：后端离线与"在线但未选仓库"此前共用 backendRepo=null 一个状态——
  // 离线时切换器显示"未选择仓库/尚未挂载"与画布上的演示数据自相矛盾（用户截图的困惑点）。
  // 三态显式化：null=探测中 / true=在线 / false=离线（演示数据模式）
  const [backendOnline, setBackendOnline] = useState<boolean | null>(null)
  // M2 引导与降级：执行 agent 状态（found=null 探测中/离线——按可用处理，只有显式 false 才拦截）
  const [agentState, setAgentState] = useState<{
    found: boolean | null
    detected: { command: string; path: string; version: string | null }[]
  }>({ found: null, detected: [] })
  const loadAgentState = useCallback(() => {
    fetch('/api/agent/status')
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { effective?: { found: boolean }; detected?: { command: string; path: string; version: string | null }[] } } | null) => {
        if (d?.data?.effective) {
          setAgentState({ found: d.data.effective.found, detected: d.data.detected ?? [] })
        }
      })
      .catch(() => {})
  }, [])
  // 后端在线后加载 agent 状态 + 30s 轮询（装好后自然恢复，无需手动刷新）
  useEffect(() => {
    if (backendOnline !== true) return
    loadAgentState()
    const t = window.setInterval(loadAgentState, 30000)
    return () => window.clearInterval(t)
  }, [backendOnline, loadAgentState])

  // M2"采用"动作的端点序列（方案 §5.2）：settings/set → test → status；任一步失败保留横幅并报原因
  const adoptAgent = async (command: string) => {
    try {
      const presetId = ['claude', 'codex', 'opencode'].includes(command) ? command : 'custom'
      const put = (key: string, value: unknown) =>
        fetch('/api/settings/set', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ scope: 'global', key, value }) })
      const [r1, r2] = await Promise.all([put('agent.command', command), put('agent.preset', presetId)])
      if (!r1.ok || !r2.ok) throw new Error('配置写入失败')
      const r3 = await fetch('/api/agent/test', { method: 'POST' })
      const d3 = await r3.json().catch(() => null)
      toast(
        d3?.data?.ok ? `已采用 ${command}，协议兼容（${d3.data.latencyMs}ms）` : `已采用 ${command}，但协议测试未通过：${d3?.data?.protocol ?? '未知'}`,
        d3?.data?.ok ? 'info' : 'error',
      )
      loadAgentState()
    } catch (e) {
      toast(e instanceof Error ? e.message : '采用失败', 'error')
    }
  }

  const CLAUDE_INSTALL_CMD = 'npm install -g @anthropic-ai/claude-code'
  const copyInstallCmd = () => {
    void navigator.clipboard?.writeText(CLAUDE_INSTALL_CMD)
    toast('安装命令已复制——在终端粘贴执行，完成后回到这里点"重新探测"')
  }
  const [reloadTick, setReloadTick] = useState(0)
  // 运行会话气泡重拉信号：WS onopen（重连）时递增，作为 resyncKey 传给 SessionBubble（I5②）
  const [queueResyncTick, setQueueResyncTick] = useState(0)
  // Y7：后端版本感知——WS 重连（全量重同步点）比对版本，变化提示刷新
  // R3 #4：WS effect 不重跑，onopen 闭包读 state 永远是初值——版本比对存 ref
  // （首次连接为 null 时比较被短路、后端热重启后前端永远拿不到"请刷新"提示的问题）
  const serverVersionRef = useRef<string | null>(null)

  // D5-2：桌面壳自动更新（仅 Tauri 环境生效，浏览器 no-op）
  useEffect(() => {
    initUpdater()
  }, [])

  // P1 审查 2#16：AppShell 徽标曾是被定义却从不传入的死功能——
  // 待审批计数实时接通：初始拉取 + WS 任务事件驱动（信息架构明写"评审（徽标）"）
  // v4 扩展：双计数（待审批 + 执行中）+ 注意力条数据（第一个待审批任务的话术）
  const [pendingApprovals, setPendingApprovals] = useState(0)
  const [runningCount, setRunningCount] = useState(0)
  const [attention, setAttention] = useState<{ count: number; sample: string } | null>(null)
  useEffect(() => {
    if (!backendRepo) {
      setPendingApprovals(0)
      setRunningCount(0)
      setAttention(null)
      return
    }
    const GL: Record<string, string> = { plan: '任务书审批', analysis: '需求矩阵评审', solution: '方案评审', diff: 'Diff 审批', report: '审查报告' }
    const load = () =>
      fetch(`/api/repos/${encodeURIComponent(backendRepo)}/tasks`)
        .then((r) => (r.ok ? r.json() : null))
        .then((d: { data?: { status: string; title: string; gate?: string | null }[] } | null) => {
          const ts = d?.data ?? []
          setPendingApprovals(ts.filter((t) => t.status === 'awaiting_approval').length)
          setRunningCount(ts.filter((t) => t.status === 'running').length)
          const waiting = ts.filter((t) => t.status === 'awaiting_approval')
          setAttention(
            waiting.length > 0
              ? { count: waiting.length, sample: `「${waiting[0].title}」停在${GL[waiting[0].gate ?? ''] ?? '审批'}` }
              : null,
          )
        })
        .catch(() => {})
    load()
    return onTaskEvent(load)
  }, [backendRepo])

  // M4-1 应用壳状态：页面 / 顶栏抽屉 / 仓库管理面板 / 引导卡 / 任务表单 / 视图定位请求
  const [page, setPage] = useState<PageId>('map')
  // 2026-10-04 暗黑模式：主题状态（localStorage 持久化，默认跟随系统）；html.dark 驱动 Tailwind class 策略
  const [dark, setDark] = useState(() => {
    try {
      const saved = window.localStorage.getItem('ev.theme')
      if (saved === 'dark' || saved === 'light') return saved === 'dark'
      return window.matchMedia('(prefers-color-scheme: dark)').matches
    } catch {
      return false
    }
  })
  useEffect(() => {
    document.documentElement.classList.toggle('dark', dark)
    try {
      window.localStorage.setItem('ev.theme', dark ? 'dark' : 'light')
    } catch { /* 静默 */ }
  }, [dark])
  // v0.2：地图页「就此对话」跨页上下文（消费即清；后写覆盖先写，竞态语义自然）
  const [pendingChatContext, setPendingChatContext] = useState<{ refId: string; refName: string; kind: 'module' | 'layer' } | null>(null)
  /** 2026-10-05 依赖体检：画布边浮卡跳入时的聚焦卡片 id */
  const [depsFocus, setDepsFocus] = useState<string | null>(null)
  /** 2026-10-05 依赖透镜：DepsPage「在画布上看」→ 画布选中并开 solo（filters 状态本体在 Canvas） */
  const [lensRequest, setLensRequest] = useState<string | null>(null)
  /** 2026-10-05 用量页/任务页/画布 → 运行页会话 deeplink */
  const [runsFocus, setRunsFocus] = useState<string | null>(null)
  // 2026-10-04 新手引导：版本化状态（lib/onboarding）+ 帮助菜单强制重开
  const [onboarding, setOnboarding] = useState(loadOnboarding)
  const [welcomeOpen, setWelcomeOpen] = useState(false)
  // ui-test P1：表单创建成功 → 任务页流水线选中该任务（nonce 区分多次跳入）
  const [taskFocus, setTaskFocus] = useState<{ id: string; nonce: number } | null>(null)
  const [overlay, setOverlay] = useState<'views' | 'suggest' | null>(null)
  const [repoPanelOpen, setRepoPanelOpen] = useState(false)
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null) // 重审 P1：注销双选项确认（保留/清除数据）
  const [guideDismissed, setGuideDismissed] = useState(
    () => typeof window !== 'undefined' && localStorage.getItem('ev.m4.guide') === '1',
  )
  const [taskDraft, setTaskDraft] = useState<TaskDraft | null>(null)
  // R3 B2：draft 序号——每次打开新表单 +1 作 key，强制重置组件实例（同 Canvas 层逻辑）
  const [draftSeq, setDraftSeq] = useState(0)
  const openTaskDraft = (d: TaskDraft) => {
    setDraftSeq((s) => s + 1)
    setTaskDraft(d)
  }
  const [viewRequest, setViewRequest] = useState<string[] | null>(null)
  // 右栏开合/宽度提升到 App：随项目持久化（settings_repo 的 repo scope，后端已有设施）
  const [panelOpen, setPanelOpen] = useState(true)
  const [panelWidth, setPanelWidth] = useState(340)
  // 巡检状态（M4-1 从 Canvas 上移：顶栏"巡检"按钮由壳层持有）
  const [patrolling, setPatrolling] = useState(false)
  const startPatrol = useCallback(() => {
    if (!backendRepo || patrolling) return
    setPatrolling(true)
    fetch(`/api/repos/${backendRepo}/patrol`, { method: 'POST' })
      .then((r) => {
        // S2：抛 Response 本体——catch 里判别 409（单会话纪律）入队
        if (!r.ok) throw r
      })
      .catch(async (e) => {
        setPatrolling(false)
        // S2：409 不再静默——入队，当前会话结束后自动接续
        if ((e as Response)?.status === 409) {
          const res = await enqueue(backendRepo, 'patrol')
          if (res?.outcome === 'replaced')
            toast(`已加入队列：巡检将在当前会话结束后自动开始（已替换排队：${res.replacedLabel ?? '旧任务'}）`)
          else if (res?.outcome === 'queued') toast('已加入队列：巡检将在当前会话结束后自动开始')
          else if (res?.outcome === 'started') {
            setPatrolling(true)
            toast('已直接开始巡检')
          }
          return
        }
        toast('巡检启动失败（请确认后端在线后重试）。', 'error')
      })
  }, [backendRepo, patrolling])

  const saveUiPref = useCallback(
    (key: string, value: unknown) => {
      if (!backendRepo) return
      fetch('/api/settings/set', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ scope: backendRepo, key, value }),
      }).catch(() => {})
    },
    [backendRepo],
  )

  const handlePageChange = useCallback(
    (p: PageId) => {
      setPage(p)
      saveUiPref('ui.page', p)
    },
    [saveUiPref],
  )

  // 2026-10-05 白屏自愈：后端运行期死亡时壳会自重启并广播 backend-recovered——
  // 届时 webview 已空白，唯一能做的就是整页重载（React 状态丢失可接受，数据都在后端）
  useEffect(() => {
    if (!isTauriRuntime()) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    import('@tauri-apps/api/event')
      .then((m) => m.listen('backend-recovered', () => window.location.reload()))
      .then((fn) => {
        if (cancelled) fn()
        else unlisten = fn
      })
      .catch(() => {})
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  // 2026-10-05 系统级通知：窗口失焦/后台时送达通知中心（toast 只有前台可见）。
  // 会话失败 = 需要人来看；排队 drained+started = 离开等排队的用户该回来了。
  useEffect(
    () =>
      onSessionEvent((e) => {
        if (e.repo !== backendRepo || e.status !== 'failed') return
        void sysNotify('EasyVibe · 会话失败', `会话 ${e.sessionId} 执行失败——回来看看原因`)
      }),
    [backendRepo],
  )
  useEffect(
    () =>
      onQueueChanged((e) => {
        if (e.repo !== backendRepo || e.type !== 'drained' || !e.started) return
        void sysNotify('EasyVibe · 排队任务已开始', e.job?.label ?? '')
      }),
    [backendRepo],
  )

  // v0.2：「就此对话」统一收口——带上下文跳「任务对话」页（Canvas 工具栏/详情视图/占位页签共用）
  const goChatAbout = useCallback(
    (target: { refId: string; refName: string; kind: 'module' | 'layer' }) => {
      setPendingChatContext(target)
      handlePageChange('workbench')
    },
    [handlePageChange],
  )

  // ---------- 新手引导：事件驱动勾选（调研 C2——完成 = 真实激活动作，不是"看过"） ----------
  useEffect(() => {
    if (repos.length > 0) setOnboarding((prev) => markCheck(prev, 'addRepo'))
  }, [repos.length])
  useEffect(() => {
    if (page === 'map' && map) setOnboarding((prev) => markCheck(prev, 'viewMap'))
  }, [page, map])
  useEffect(() => {
    if (page === 'health' || page === 'drift') setOnboarding((prev) => markCheck(prev, 'viewHealth'))
  }, [page])
  // firstApproval：轮询探测"任何任务存在审批记录"（只在本项未完成时跑，30s 节拍）
  useEffect(() => {
    if (!backendRepo || onboarding.checklist.firstApproval === 'done') return
    let dead = false
    const probe = async () => {
      try {
        const r = await fetch(`/api/repos/${backendRepo}/tasks`)
        const d: { data?: { id: string }[] } = r.ok ? await r.json() : null
        for (const t of (d?.data ?? []).slice(0, 5)) {
          const ra = await fetch(`/api/repos/${backendRepo}/tasks/${encodeURIComponent(t.id)}/approvals`)
          if (!ra.ok) continue
          const da: { data?: unknown[] } = await ra.json()
          if ((da?.data?.length ?? 0) > 0) {
            if (!dead) setOnboarding((prev) => markCheck(prev, 'firstApproval'))
            return
          }
        }
      } catch {
        /* 后端离线等场景静默 */
      }
    }
    void probe()
    const t = window.setInterval(() => void probe(), 30000)
    return () => {
      dead = true
      window.clearInterval(t)
    }
  }, [backendRepo, onboarding.checklist.firstApproval])
  // 全部完成 → 庆祝 + 自动收尾（只触发一次）
  const onboardDoneRef = useRef(-1)
  useEffect(() => {
    const n = CHECK_KEYS.filter((k) => onboarding.checklist[k] === 'done').length
    if (n === CHECK_KEYS.length && onboardDoneRef.current !== n) {
      toast(ONBOARDING_COPY.checklist.doneToast, 'info')
      setOnboarding((prev) => completeAll(prev))
    }
    onboardDoneRef.current = n
  }, [onboarding])

  // 每项目记忆：右栏宽度 / 面板开合随项目持久化（M4-1 状态持久化，防刷新丢位置）。
  // 页签不恢复（2026-10-03 用户裁定）：切换仓库固定落架构地图——上次在 A 仓库看任务，
  // 切到 B 仓库还停在任务页是错位的；页签位置由"当前在看什么"决定，跨项目无意义。
  useEffect(() => {
    if (!backendRepo) return
    setPage('map') // 切仓库先回架构地图，数据到达前不闪旧页
    let stale = false
    fetch(`/api/settings?scope=${encodeURIComponent(backendRepo)}`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { key: string; value: unknown }[] } | null) => {
        if (stale || !d?.data) return
        const get = (k: string) => d.data!.find((i) => i.key === k)?.value
        const w = get('ui.panelWidth')
        if (typeof w === 'number' && Number.isFinite(w)) setPanelWidth(Math.min(560, Math.max(340, w)))
        const po = get('ui.panelOpen')
        if (typeof po === 'boolean') setPanelOpen(po)
      })
      .catch(() => {})
    return () => {
      stale = true
    }
  }, [backendRepo])

  const handlePanelOpenChange = useCallback(
    (open: boolean) => {
      setPanelOpen(open)
      saveUiPref('ui.panelOpen', open)
    },
    [saveUiPref],
  )

  const handlePanelWidthChange = useCallback(
    (w: number) => {
      setPanelWidth(w)
      saveUiPref('ui.panelWidth', w)
    },
    [saveUiPref],
  )

  // 抽屉 Esc 关闭（真人测试 Bug#2：全屏遮罩层 Esc 无效，用户第一反应是"卡住了"）
  useEffect(() => {
    if (!overlay) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOverlay(null)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [overlay])

  useEffect(() => {
    let cancelled = false
    let timer: number | undefined
    // 后端探测：/api/repos 取仓库列表（Y7 回归修复——此前误把 /api/health 的响应当 repos 解析，
    // data 无 length 恒为演示模式，桌面壳与浏览器一并中招）
    // R2 P0 两轮老账：探测只跑一次，后端晚启动（桌面壳冷启动 30s 内常见）永不可达只能刷新——
    // 现在探测成功前每 5 秒重试，永不死心。
    const probe = () => {
      fetch('/api/repos')
        .then((r) => (r.ok ? r.json() : Promise.reject(new Error('no backend'))))
        .then((d: { data?: { id: string; name: string }[] }) => {
          if (!d.data || d.data.length === 0) {
            // 在线但零仓库：与离线显式区分（"尚未挂载"是真实状态，不是探测失败）
            setBackendOnline(true)
            if (!cancelled) timer = window.setTimeout(probe, 5000)
            return
          }
          setBackendOnline(true)
          setRepos(d.data)
          setBackendRepo((cur) => cur ?? d.data![0].id) // 保留当前选择（切换器驱动）
          // 版本感知（Y7）：仅取 version，不参与后端模式判定
          fetch('/api/health')
            .then((r) => (r.ok ? r.json() : null))
            .then((h: { data?: { version?: string } } | null) => {
              if (!cancelled && h?.data?.version) serverVersionRef.current = h.data.version
            })
            .catch(() => {})
        })
        .catch(() => {
          if (cancelled) return
          setBackendRepo(null)
          setBackendOnline(false)
          timer = window.setTimeout(probe, 5000)
        })
    }
    probe()
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [])

  // WS 订阅：map.changed 触发重取 / growth 与 session 事件转发；指数退避重连 + 重连后全量重同步
  useEffect(() => {
    if (!backendRepo) return
    let closed = false
    let ws: WebSocket | null = null
    let retry = 0
    let timer: number | undefined

    const connect = () => {
      if (closed) return
      const proto = location.protocol === 'https:' ? 'wss' : 'ws'
      ws = new WebSocket(`${proto}://${location.host}/ws`)
      ws.onopen = () => {
        retry = 0
        setReloadTick((t) => t + 1) // 全量重同步（覆盖断线期间的变更）
        // I5②：WS 重连 → 触发运行会话气泡重拉队列快照（resyncKey 递增，SessionBubble 消费）
        setQueueResyncTick((t) => t + 1)
        // Y7：重连点比对后端版本——热重启/更新后前端是旧契约，提示刷新
        fetch('/api/health')
          .then((r) => r.json())
          .then((h: { data?: { version?: string } }) => {
            const v = h.data?.version
            const prev = serverVersionRef.current
            if (v && prev && v !== prev) {
              toast(`后端已更新（${prev} → ${v}），刷新页面以加载新界面`, 'error')
            }
            if (v) {
              serverVersionRef.current = v
            }
          })
          .catch(() => {})
      }
      ws.onmessage = (e) => {
        try {
          const msg = JSON.parse(e.data)
          if (msg.name === 'map.changed' && msg.data?.repo === backendRepo) setReloadTick((t) => t + 1)
          if (msg.name === 'growth.event' && msg.data?.repo === backendRepo) emitGrowthEvent(msg.data.event)
          if (msg.name === 'session.statusChanged') {
            const d = msg.data
            if (d?.repo === backendRepo) emitSessionEvent({ repo: d.repo, sessionId: d.sessionId, status: d.status })
          }
          if (msg.name === 'queue.changed') {
            // 会话排队变更（入队/替换/取消/排空/失败）——SessionBubble 与 drained 接管逻辑消费
            const d = msg.data
            if (d?.repo === backendRepo)
              emitQueueChanged({ repo: d.repo, type: d.type, job: d.job, started: d.started, error: d.error })
          }
          if (msg.name === 'session.output') {
            emitSessionOutput({ sessionId: msg.data.sessionId, seq: msg.data.seq ?? 0, stream: msg.data.stream ?? 'stdout', line: msg.data.line })
            // 方案 v3 §4.2：终端推送挂在常驻的 App 层（页面卸载也在攒）——
            // 工作流页的终端缓冲因此切走再回来不丢
            pushTerminalLine(msg.data.sessionId, msg.data.line)
          }
          if (msg.name === 'patrol.finished') {
            // R3 C1：巡检终态成为产品事件（真实模式会话 id 是 ind-N，靠事件而非前缀判定）
            emitPatrolFinished({ repo: msg.data.repo, runId: msg.data.runId, status: msg.data.status })
          }
          if (msg.name === 'freshness.changed' && msg.data?.repo === backendRepo) {
            emitFreshnessEvent({
              repo: msg.data.repo,
              status: msg.data.status,
              latestCommitAt: msg.data.latestCommitAt,
              commitsSinceMap: msg.data.commitsSinceMap,
            })
          }
          if (msg.name === 'task.contractAlert') {
            // L2 过程预警：任务执行中哨兵抓到的新增越界——比终态红线早 N 分钟到达
            // R3 D1：埋点（过程预警曝光）+ 操作按钮直达评审（此前裸 toast 无入口）；info 色与终态红区分
            // ui-test-2026-10-03 P1：措辞去内部 task id（用户看不懂），任务终态即撤下（见 statusChanged）
            const d = msg.data
            if (d?.repo !== backendRepo) return
            track(d.repo, 'ui.contractAlert.shown', { taskId: d.taskId })
            toast(`影响面预警：有任务正在越界改动（${(d.files ?? []).slice(0, 2).join('、')}${(d.files?.length ?? 0) > 2 ? ' 等' : ''}）`, 'info', {
              label: '去处理',
              onClick: () => {
                track(d.repo, 'ui.contractAlert.click', { taskId: d.taskId })
                handlePageChange('tasks')
              },
            }, true, `contract:${d.taskId}`)
            if (document.hidden) notifySystem('EasyVibe · 影响面预警', `有任务正在越界：${(d.files ?? []).slice(0, 3).join('、')}`)
          }
          if (msg.name === 'task.contractViolated') {
            // R2 裂缝#3：auto/supervised 任务无审批关——越界经 WS 主动送达（与审批通知同双通道）
            // R3 D1：埋点（终态红线曝光）+ 直达评审操作
            const d = msg.data
            if (d?.repo !== backendRepo) return
            track(d.repo, 'ui.contractViolated.shown', { taskId: d.taskId })
            toast(`影响面合约：有任务越界改动 ${d.files?.length ?? 0} 个文件`, 'error', {
              label: '去处理',
              onClick: () => {
                track(d.repo, 'ui.contractViolated.click', { taskId: d.taskId })
                handlePageChange('tasks')
              },
            }, true, `contract:${d.taskId}`)
            if (document.hidden) notifySystem('EasyVibe · 影响面越界', `有任务越界 ${d.files?.length ?? 0} 个文件`)
          }
          if (msg.name === 'task.statusChanged') {
            const d = msg.data
            if (d?.repo !== backendRepo) return
            emitTaskEvent({ repo: d.repo, taskId: d.taskId, status: d.status, gate: d.gate })
            // P0 对标缺口#3：审批零通知——manual 任务在计划关等批，用户不盯窗口就卡死。
            // 系统通知（惰性申请权限）+ 页内 toast 双通道；仅窗口隐藏时弹系统通知防打扰
            if (d?.status === 'awaiting_approval') {
              toast('有任务等待你的审批', 'info')
              if (document.hidden) notifySystem('EasyVibe · 待审批', `有任务已到达审批关${d.gate ? `（${d.gate}）` : ''}`)
            }
            if (d?.status === 'failed') {
              toast('有任务执行失败', 'error')
              if (document.hidden) notifySystem('EasyVibe · 任务失败', '有任务执行失败，回应用查看详情')
            }
            // ui-test-2026-10-03：任务终态（含 kill）即撤下它的越界预警——已死任务不再"正在越界"
            if (['failed', 'done', 'rejected', 'interrupted'].includes(d?.status)) {
              dismissToast(`contract:${d.taskId}`)
            }
          }
        } catch {
          /* 忽略坏消息 */
        }
      }
      ws.onclose = () => {
        if (closed) return
        notifyWsClosed() // R2：断线时通知 Canvas 退出生长模式（重连后重新进入会拉全量）
        retry += 1
        timer = window.setTimeout(connect, Math.min(15000, 1000 * 2 ** retry))
      }
    }
    connect()
    return () => {
      closed = true
      window.clearTimeout(timer)
      ws?.close()
    }
    // handlePageChange/track 仅用于事件回调内的瞬态动作（跳页/埋点），不应重启 WS 连接
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [backendRepo])

  useEffect(() => {
    const url = backendRepo ? `/api/repos/${backendRepo}/map` : '/data/map.json'
    fetch(url)
      .then((r) => {
        // 404 = 仓库尚未归纳——合法状态，走 MapGate 的"尚未生成"引导（开始归纳），不是错误
        if (r.status === 404 && backendRepo) return null
        if (!r.ok) throw new Error(`HTTP ${r.status}`)
        return r.json() as Promise<CodeMap>
      })
      .then((m) => {
        setError(null) // 实弹#4 前端根因：成功后必须清错误态，否则 (error && backendRepo) 恒真永远白屏等待
        if (m) setMap(m)
      })
      .catch((e) => setError(String(e)))
  }, [backendRepo, reloadTick])

  if (error && !backendRepo) {
    return (
      <div className="flex h-screen items-center justify-center text-[13px] text-red-500">
        静态数据加载失败（/data/map.json）：{error}
      </div>
    )
  }
  const switchRepo = (id: string) => {
    if (id === backendRepo) return
    setMap(null)
    setError(null)
    setBackendRepo(id)
  }

  // D5 仓库管理：刷新列表（添加/移除后）；后端事实源是 /api/repos
  const refreshRepos = useCallback(() => {
    fetch('/api/repos')
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { id: string; name: string }[] } | null) => {
        if (!d?.data) return
        setRepos(d.data)
        setBackendRepo((cur) => (cur && d.data!.some((r) => r.id === cur) ? cur : d.data![0]?.id ?? null))
      })
      .catch(() => {})
  }, [])

  // 添加本地仓库：桌面壳走系统目录选择器（Tauri dialog），浏览器降级为路径输入
  const addRepo = useCallback(async (): Promise<boolean> => {
    let path: string | null = null
    try {
      if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
        const { open } = await import('@tauri-apps/plugin-dialog')
        const sel = await open({ directory: true, title: '选择本地仓库目录' })
        path = typeof sel === 'string' ? sel : null
      } else {
        path = window.prompt('输入本地仓库目录的绝对路径')
      }
    } catch {
      path = window.prompt('目录选择器不可用，输入本地仓库目录的绝对路径')
    }
    if (!path?.trim()) return false
    try {
      const r = await fetch('/api/repos', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path: path.trim() }),
      })
      const d = await r.json().catch(() => null)
      if (!r.ok) {
        toast(d?.error ?? '添加失败（目录不可读或已挂载）', 'error')
        return false
      }
      toast('已添加仓库，正在归纳…')
      refreshRepos()
      setBackendRepo(d.data.id)
      return true
    } catch {
      toast('添加失败（需要后端在线）', 'error')
      return false
    }
  }, [refreshRepos])

  // 重审 P1：移除仓库现在会杀活动会话（此前 running 的 agent 成孤儿占死写互斥）；
  // wipe=true 额外抹掉该仓库在本地库的全部痕迹（任务/会话/审批/巡检/事件/仓库级设置）
  const removeRepo = useCallback(async (id: string, wipe: boolean) => {
    try {
      const r = await fetch(`/api/repos/${encodeURIComponent(id)}${wipe ? '?wipe=true' : ''}`, { method: 'DELETE' })
      if (!r.ok) {
        toast('移除失败', 'error')
        return
      }
      toast(wipe ? '已移除仓库并清除其数据' : '已移除仓库（数据保留，重新添加后可见）')
      if (id === backendRepo) setMap(null)
      setConfirmRemove(null)
      refreshRepos()
    } catch {
      toast('移除失败（需要后端在线）', 'error')
    }
  }, [backendRepo, refreshRepos])

  // P0 审查前端#1：地图加载失败不再一律渲染"归纳中"——
  // MapGate 用 /progress 区分"真在归纳"（等待页）与"真出错"（错误卡：重试/开始归纳）
  if (backendRepo && (error || !map)) {
    return <MapGate repo={backendRepo} error={error} onRetry={() => setReloadTick((t) => t + 1)} agentReady={agentState.found !== false} />
  }
  if (!map) {
    return (
      <div className="flex h-screen items-center justify-center gap-2 text-[13px] text-slate-500 dark:text-slate-400">
        <Loader2 size={16} className="animate-spin" /> 正在加载代码地图…
      </div>
    )
  }

  // M4-1 顶栏三区（VSCode 范式）：左=品牌（AppShell 内置红绿灯），中=项目切换器（居中），右=全局动作
  const topCenter = (
    <div className="relative">
        <button
          onClick={() => setRepoPanelOpen((v) => !v)}
          className={
            backendOnline === false
              ? "flex items-center gap-1.5 rounded-lg border border-amber-300 dark:border-amber-800 bg-amber-50 dark:bg-amber-950/40 px-2.5 py-1 text-[12px] font-semibold text-amber-700 hover:border-amber-400"
              : "flex items-center gap-1.5 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1 text-[12px] font-semibold text-slate-700 dark:text-slate-200 hover:border-blue-300"
          }
          title={backendOnline === false ? "后端不在线：当前为演示数据，点击看详情" : "切换/管理仓库"}
        >
          {backendOnline === false ? (
            <>
              <WifiOff size={11} />
              演示数据 · 后端离线
              <span className="text-amber-400">▾</span>
            </>
          ) : (
            <>
              <span className={`h-2 w-2 rounded-full ${backendOnline === null ? 'animate-pulse bg-slate-300' : 'bg-emerald-500'}`} />
              {backendOnline === null ? '连接后端中…' : backendRepo ?? '未选择仓库'}
              <span className="text-slate-300 dark:text-slate-600">▾</span>
            </>
          )}
        </button>
        {repoPanelOpen && (
          <>
            {/* 点外部关闭（真人测试 Bug#1：此前无 outside-click 处理，跨页面悬浮） */}
            <div className="fixed inset-0 z-30" onClick={() => setRepoPanelOpen(false)} />
            <div className="glass absolute left-0 top-full z-40 mt-1.5 w-80 rounded-xl border border-slate-200 dark:border-slate-700 p-2 shadow-xl">
            {backendOnline === false ? (
              /* 重审 P2：离线态的真相面板——不装成"尚未挂载"（那是在线零仓库的状态） */
              <div className="space-y-1.5 px-1.5 py-1.5">
                <p className="flex items-center gap-1 text-[12px] font-bold text-amber-700">
                  <WifiOff size={12} /> 后端不在线
                </p>
                <p className="text-[11px] leading-4 text-slate-500 dark:text-slate-400">
                  当前画布是内置演示数据（hover-client）。归纳 / 巡检 / 任务 / 仓库管理都需要本地后端在线。
                </p>
                <p className="text-[10px] leading-4 text-slate-400 dark:text-slate-500">
                  应用启动后后端在冷加载？每 5 秒自动重连，恢复后此面板自动可用。
                </p>
              </div>
            ) : (
              <>
            <p className="px-1.5 pb-1.5 text-micro font-semibold text-slate-400 dark:text-slate-500">已挂载仓库</p>
            <div className="max-h-52 space-y-0.5 overflow-y-auto">
              {repos.map((r) => (
                <div key={r.id} className="rounded-lg px-1.5 py-1 hover:bg-slate-50 dark:hover:bg-slate-800/70">
                  <div className="flex items-center gap-1.5">
                    <button
                      className={`min-w-0 flex-1 truncate text-left text-[12px] ${r.id === backendRepo ? 'font-bold text-blue-700' : 'text-slate-700 dark:text-slate-200'}`}
                      onClick={() => {
                        switchRepo(r.id)
                        setRepoPanelOpen(false)
                      }}
                      title={r.name}
                    >
                      {r.name}
                    </button>
                    {confirmRemove === r.id ? (
                      <button
                        onClick={() => setConfirmRemove(null)}
                        className="shrink-0 rounded p-0.5 text-slate-400 dark:text-slate-500 hover:text-slate-600"
                        title="取消"
                      >
                        <X size={12} />
                      </button>
                    ) : (
                      <button
                        onClick={() => setConfirmRemove(confirmRemove === r.id ? null : r.id)}
                        className="shrink-0 rounded p-0.5 text-slate-300 dark:text-slate-600 hover:bg-red-50 dark:hover:bg-red-950/40 hover:text-red-500"
                        title="移除仓库…"
                      >
                        <X size={12} />
                      </button>
                    )}
                  </div>
                  {/* 重审 P1：注销确认双选项——数据保留 or 连同清除（后端 wipe_repo），
                      不再是无差别的 window.confirm（用户不知道数据去了哪） */}
                  {confirmRemove === r.id && (
                    <div className="mt-1 space-y-1 rounded-lg border border-red-100 bg-red-50/50 p-1.5">
                      <p className="text-[10px] leading-4 text-slate-500 dark:text-slate-400">
                        正在运行的任务/归纳会被终止。本地数据怎么处理？
                      </p>
                      <div className="flex gap-1">
                        <button
                          onClick={() => void removeRepo(r.id, false)}
                          className="flex-1 rounded-md border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-1 text-[10px] font-bold text-slate-600 dark:text-slate-300 hover:border-blue-300 hover:text-blue-600"
                          title="任务/会话/巡检历史留在本地库，重新添加仓库后可见"
                        >
                          移除，保留数据
                        </button>
                        <button
                          onClick={() => void removeRepo(r.id, true)}
                          className="flex-1 rounded-md bg-red-500 px-2 py-1 text-[10px] font-bold text-white hover:bg-red-600"
                          title="抹掉该仓库的任务/会话/审批/巡检历史/事件/仓库级设置（不可恢复）"
                        >
                          移除并清除数据
                        </button>
                      </div>
                    </div>
                  )}
                </div>
              ))}
              {repos.length === 0 && <p className="px-1.5 py-2 text-[11px] text-slate-400 dark:text-slate-500">尚未挂载任何仓库</p>}
            </div>
            <button
              onClick={() => {
                setRepoPanelOpen(false)
                addRepo()
              }}
              className="mt-1.5 flex w-full items-center justify-center gap-1 rounded-lg bg-blue-600 px-2 py-1.5 text-[11px] font-semibold text-white hover:bg-blue-700"
            >
              <Plus size={11} />
              打开本地仓库…
            </button>
              </>
            )}
          </div>
          </>
        )}
    </div>
  )
  const topBar = (
      <div className="flex min-w-0 flex-1 items-center gap-2">
        {topCenter}
        <div className="ml-auto flex shrink-0 items-center gap-1.5">
        <button
          onClick={() => setOverlay((o) => (o === 'views' ? null : 'views'))}
          className="flex items-center gap-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-1 text-[12px] font-semibold text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70"
          title="我的视图（对话沉淀的图资产）"
        >
          <LayoutGrid size={12} />
          视图
        </button>
        {backendRepo && (
          <button
            onClick={() => setOverlay((o) => (o === 'suggest' ? null : 'suggest'))}
            className="flex items-center gap-1 rounded-lg border border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-2 py-1 text-[12px] font-semibold text-amber-700 hover:bg-amber-100 dark:hover:bg-amber-900/40"
            title="AI 主动发现优化建议，逐条可发起修复"
          >
            <Lightbulb size={12} />
            优化建议
          </button>
        )}
        {backendRepo && (
          <button
            onClick={startPatrol}
            disabled={patrolling || agentState.found === false}
            className={`flex items-center gap-1 rounded-lg border px-2 py-1 text-[12px] font-semibold transition-colors disabled:opacity-40 ${
              patrolling ? 'border-amber-300 dark:border-amber-800 bg-amber-50 dark:bg-amber-950/40 text-amber-700' : 'border-emerald-200 dark:border-emerald-900/60 bg-emerald-50 dark:bg-emerald-950/40 text-emerald-700 hover:bg-emerald-100'
            }`}
            title={agentState.found === false ? '未检测到执行 agent——先安装或在设置中配置' : '巡检：Supervisor 直调 LLM（带健康基线），产出新地图并落健康历史'}
          >
            <Activity size={12} className={patrolling ? 'animate-pulse' : ''} />
            {patrolling ? '巡检中…' : '巡检'}
          </button>
        )}
        <button
          onClick={() => downloadHealthReport(map)}
          className="flex items-center gap-1 rounded-lg border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2 py-1 text-[12px] font-semibold text-slate-600 dark:text-slate-300 hover:bg-slate-50 dark:hover:bg-slate-800/70"
          title="导出架构健康报告（Markdown，零 token 成本）"
        >
          <FileDown size={12} />
          导出
        </button>
        {/* 主题开关：亮=太阳 / 暗=月亮滑动拨块（2026-10-04） */}
        <ThemeToggle dark={dark} onChange={setDark} />
        {/* 新手引导：帮助入口——重看欢迎页（再点关闭 = 开关语义，2026-10-04 实弹） + 重置上手指引 */}
        <button
          onClick={() => {
            if (welcomeOpen) {
              setWelcomeOpen(false)
              return
            }
            setOnboarding(resetForReview())
            setWelcomeOpen(true)
          }}
          className={`rounded-lg border px-2 py-1 text-[12px] font-semibold transition-colors ${
            welcomeOpen
              ? 'border-blue-300 bg-blue-50 text-blue-700 dark:border-blue-800 dark:bg-blue-950/50 dark:text-blue-300'
              : 'border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70'
          }`}
          title="新手引导（欢迎页 + 上手指引）——再点关闭"
        >
          <CircleHelp size={12} />
        </button>
        <button
          onClick={() => handlePageChange('settings')}
          className={`rounded-lg border px-2 py-1 text-[12px] font-semibold ${
            page === 'settings' ? 'border-blue-300 dark:border-blue-800 bg-blue-50 dark:bg-blue-950/40 text-blue-700' : 'border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70'
          }`}
          title="设置（LLM 服务 / 槽位绑定 / 高级）"
        >
          <Settings size={12} />
        </button>
        </div>
      </div>
  )

  const openRunsSession = useCallback((sessionId: string) => {
    setRunsFocus(sessionId)
    handlePageChange('runs')
  }, [handlePageChange])

  const PAGES: Record<PageId, React.ReactNode> = {
    map: (
      <CanvasBoundary>
        <ReactFlowProvider>
          <Canvas
            map={map}
            backendRepo={backendRepo}
            onPatrollingChange={setPatrolling}
            panelOpen={panelOpen}
            onPanelOpenChange={handlePanelOpenChange}
            panelWidth={panelWidth}
            onPanelWidthChange={handlePanelWidthChange}
            viewRequest={viewRequest}
            onViewRequestConsumed={() => setViewRequest(null)}
            agentReady={agentState.found !== false}
            guide={
              guideDismissed ? undefined : (
                <div className="mx-2 mt-2 flex items-start gap-2 rounded-lg border border-blue-100 bg-blue-50/70 dark:bg-blue-950/40 px-3 py-2">
                  <p className="flex-1 text-[11px] leading-5 text-slate-600 dark:text-slate-300">
                    <span className="font-semibold text-blue-700">界面已整理：</span>
                    详情栏页签为 详情/问题；对话在左侧「任务对话」，任务在「任务」，视图与优化建议在顶栏。
                  </p>
                  <button
                    onClick={() => {
                      setGuideDismissed(true)
                      localStorage.setItem('ev.m4.guide', '1')
                    }}
                    className="shrink-0 rounded-full bg-white dark:bg-slate-900 px-2 py-0.5 text-micro font-semibold text-blue-600 shadow-sm hover:bg-blue-50 dark:hover:bg-blue-950/40"
                  >
                    知道了
                  </button>
                </div>
              )
            }
            onTaskCreated={(id) => {
              setTaskFocus({ id, nonce: Date.now() })
              setOnboarding((prev) => markCheck(prev, 'firstTask'))
              handlePageChange('tasks')
            }}
            onChatAbout={goChatAbout}
            onGoWorkbench={() => handlePageChange('workbench')}
            onInspectEdge={(cardId) => {
              setDepsFocus(cardId)
              handlePageChange('deps')
            }}
            onOpenRuns={openRunsSession}
            onOpenDeps={() => handlePageChange('deps')}
            lensRequest={lensRequest}
            onLensRequestConsumed={() => setLensRequest(null)}
            dark={dark}
          />
        </ReactFlowProvider>
      </CanvasBoundary>
    ),
    tasks: <TaskPage backendRepo={backendRepo} map={map} onCreateTask={openTaskDraft} externalFocus={taskFocus} onGoChat={() => handlePageChange('workbench')} onOpenRuns={openRunsSession} />,
    runs: (
      <RunsPage
        backendRepo={backendRepo}
        initialSessionId={runsFocus}
        onInitialConsumed={() => setRunsFocus(null)}
        resyncKey={queueResyncTick}
      />
    ),
    usage: (
      <UsagePage
        backendRepo={backendRepo}
        map={map}
        onOpenModule={(id) => {
          setViewRequest([id])
          handlePageChange('map')
        }}
        onOpenSession={(sid) => {
          setRunsFocus(sid)
          handlePageChange('runs')
        }}
      />
    ),
    settings: <SettingsPanel backendRepo={backendRepo} onClose={() => handlePageChange('map')} embedded />,
    // v0.2 P1：「任务对话」——对话孵化任务（会话=任务上位容器：计划进度条/内联审批/diff 影响面三栏）
    workbench: (
      <WorkbenchPage
        backendRepo={backendRepo}
        map={map}
        onCreateTask={openTaskDraft}
        onLocateModule={(id) => {
          // v0.2 定位链路升级：选中聚焦 + 切页（此前只切页不选中，地图端找不回模块）
          setViewRequest([id])
          handlePageChange('map')
        }}
        pendingChatContext={pendingChatContext}
        onConsumeChatContext={() => setPendingChatContext(null)}
        initialIdea={page === 'workbench' ? loadTaskIdea() : null}
        onConsumeIdea={clearTaskIdea}
        onNavigate={(p) => handlePageChange(p)}
      />
    ),
    // v4 P1：任务编排/任务工作流两个旧页签删除，统一从「任务」页进入（视图切换）；
    // 旧 id 保留映射，兼容存量回调（合约预警"去评审"等）——落点都是 TaskPage
    todo: <TaskPage backendRepo={backendRepo} map={map} onCreateTask={openTaskDraft} externalFocus={taskFocus} onGoChat={() => handlePageChange('workbench')} onOpenRuns={openRunsSession} />,
    review: <TaskPage backendRepo={backendRepo} map={map} onCreateTask={openTaskDraft} externalFocus={taskFocus} onGoChat={() => handlePageChange('workbench')} onOpenRuns={openRunsSession} />,
    changes: (
      <ChangesPage
        backendRepo={backendRepo}
        map={map}
        /* 重审 P2：变更页 → 任务页流水线（选中该任务）——导航闭环，不再靠用户记任务 ID */
        onOpenTask={(id) => {
          setTaskFocus({ id, nonce: Date.now() })
          handlePageChange('tasks')
        }}
      />
    ),
    drift: <DriftPage />,
    health: <HealthPage backendRepo={backendRepo} map={map} onCreateTask={openTaskDraft} onOpenDeps={() => handlePageChange('deps')} />,
    modules: (
      <ModulesPage
        map={map}
        onOpenMap={() => handlePageChange('map')}
        onCreateTask={openTaskDraft}
        onLocateModule={(id) => {
          setViewRequest([id])
          handlePageChange('map')
        }}
      />
    ),
    deps: (
      <DepsPage
        backendRepo={backendRepo}
        map={map}
        onCreateTask={openTaskDraft}
        onChatAbout={goChatAbout}
        onOpenMap={() => handlePageChange('map')}
        onInspectModule={(id) => {
          // 透镜跳入：选中 + 打开 solo 聚焦（画布已有「✕ 退出聚焦」与底部常驻指示）
          setLensRequest(id)
          handlePageChange('map')
        }}
        focusCardId={depsFocus}
        onFocusConsumed={() => setDepsFocus(null)}
      />
    ),
    git: (
      <GitPage
        backendRepo={backendRepo}
        map={map}
        onOpenChanges={() => handlePageChange('changes')}
        onOpenReview={() => handlePageChange('review')}
      />
    ),
    'kb-docs': <PlaceholderPage title="文档中心" milestone="M4-4" description="知识库三页为 P3 骨架：从已定样式模式派生。" icon={BookOpen} />,
    'kb-decisions': <PlaceholderPage title="决策记录" milestone="M4-4" description="这个仓库做过的重要技术决策及其来龙去脉。" icon={ScrollText} />,
    'kb-apis': <PlaceholderPage title="接口目录" milestone="M4-4" description="全部关键入口（路由/函数/任务）的索引：谁对外提供什么能力。" icon={Plug} />,
  }

  return (
    <>
      <CanvasBoundary>
        <AppShell
          page={page}
          onPageChange={handlePageChange}
          topBar={topBar}
          topCenter={backendRepo ? <SessionBubble backendRepo={backendRepo} resyncKey={queueResyncTick} onOpenRuns={() => handlePageChange('runs')} /> : undefined}
          badges={{ tasks: { alert: pendingApprovals, info: runningCount } }}
          attentionBar={
            /* 优先级：零仓库 > agent 缺失引导（M2）> 审批提醒 */
            backendOnline === true && repos.length === 0 ? (
              <button
                onClick={() => addRepo()}
                className="flex shrink-0 items-center gap-2 border-b border-blue-200 dark:border-blue-900/60 bg-blue-50 dark:bg-blue-950/40 px-4 py-1.5 text-left transition-colors hover:bg-blue-100"
              >
                <FolderOpen size={12} className="shrink-0 text-blue-500" />
                <span className="text-[11px] font-bold text-blue-800">尚未打开任何仓库——当前画布是演示数据</span>
                <span className="min-w-0 flex-1 truncate text-[11px] text-blue-600">
                  归纳 / 巡检 / 任务 / 对话都需要一个本地代码仓库
                </span>
                <span className="shrink-0 rounded-md bg-blue-600 px-2 py-0.5 text-micro font-bold text-white">选择仓库 →</span>
              </button>
            ) : backendOnline === true && agentState.found === false ? (
              /* M2 首跑引导（R5）：检测到可采用的 → 一键采用；全未安装 → 安装指引+一键复制 */
              agentState.detected.length > 0 ? (
                <div className="flex shrink-0 items-center gap-2 border-b border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-4 py-1.5">
                  <Bot size={12} className="shrink-0 text-amber-500" />
                  <span className="text-[11px] font-bold text-amber-800">
                    检测到 {agentState.detected[0].command} 已安装
                  </span>
                  <span className="min-w-0 flex-1 truncate text-[11px] text-amber-600">采用后即可开始归纳 / 任务</span>
                  <button
                    onClick={() => void adoptAgent(agentState.detected[0].command)}
                    className="shrink-0 rounded-md bg-amber-600 px-2 py-0.5 text-micro font-bold text-white hover:bg-amber-700"
                  >
                    采用 {agentState.detected[0].command} →
                  </button>
                  <button onClick={() => handlePageChange('settings')} className="shrink-0 rounded-md border border-amber-300 dark:border-amber-800 px-2 py-0.5 text-micro font-semibold text-amber-700 hover:bg-amber-100 dark:hover:bg-amber-900/40">
                    去设置
                  </button>
                </div>
              ) : (
                <div className="flex shrink-0 items-center gap-2 border-b border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-4 py-1.5">
                  <Bot size={12} className="shrink-0 text-amber-500" />
                  <span className="text-[11px] font-bold text-amber-800">未检测到执行 agent</span>
                  <span className="min-w-0 flex-1 truncate text-[11px] text-amber-600">
                    归纳 / 巡检 / 任务需要本地 CLI agent（支持 claude / codex / opencode）
                  </span>
                  <button onClick={copyInstallCmd} className="shrink-0 rounded-md bg-amber-600 px-2 py-0.5 text-micro font-bold text-white hover:bg-amber-700">
                    复制 claude 安装命令
                  </button>
                  <button onClick={() => handlePageChange('settings')} className="shrink-0 rounded-md border border-amber-300 dark:border-amber-800 px-2 py-0.5 text-micro font-semibold text-amber-700 hover:bg-amber-100 dark:hover:bg-amber-900/40">
                    了解更多
                  </button>
                </div>
              )
            ) : attention && page !== 'tasks' ? (
              <button
                onClick={() => handlePageChange('tasks')}
                className="flex shrink-0 items-center gap-2 border-b border-amber-200 dark:border-amber-900/60 bg-amber-50 dark:bg-amber-950/40 px-4 py-1.5 text-left transition-colors hover:bg-amber-100 dark:hover:bg-amber-900/40"
              >
                <span className="flex h-2 w-2 animate-pulse rounded-full bg-amber-500" />
                <span className="text-[11px] font-bold text-amber-800">
                  {attention.count} 项任务等你审批
                </span>
                <span className="min-w-0 flex-1 truncate text-[11px] text-amber-600">—— {attention.sample}</span>
                <span className="shrink-0 rounded-md bg-amber-600 px-2 py-0.5 text-micro font-bold text-white">去处理 →</span>
              </button>
            ) : undefined
          }
        >
          {/* R3 B3：地图页 keep-alive——切页只隐藏不卸载，保住选中/过滤/展开子图/右栏对话草稿。
              审批典型动线「看 diff → 评审 → 回地图对照」此前每轮都被重置逼着重来 */}
          <div className="h-full" style={page === 'map' ? undefined : { display: 'none' }}>
            {PAGES.map}
          </div>
          {/* 非地图页淡入（key 驱动重触发；地图页常驻 keep-alive 不参与） */}
          {page !== 'map' && (
            <div key={page} className="anim-fade-in-fast h-full">
              {PAGES[page]}
            </div>
          )}
        </AppShell>
      </CanvasBoundary>
      {/* 视图/优化建议：顶栏抽屉（右栏三页签瘦身后的新居所） */}
      {overlay === 'views' && (
        <div className="anim-fade-in-fast fixed inset-0 z-50 flex justify-end bg-slate-900/20" onClick={() => setOverlay(null)}>
          <div className="glass anim-drawer-in flex h-full w-[460px] flex-col shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between border-b border-slate-100 dark:border-slate-800 px-3 py-2">
              <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">我的视图</span>
              <button onClick={() => setOverlay(null)} className="rounded p-1 text-slate-400 dark:text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-600">
                <X size={15} />
              </button>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto">
              <ViewsPanel
                backendRepo={backendRepo}
                onOpenView={(ids) => {
                  setOverlay(null)
                  handlePageChange('map')
                  setViewRequest(ids)
                }}
                validModuleIds={new Set(map.modules.map((m) => m.id))}
              />
            </div>
          </div>
        </div>
      )}
      {overlay === 'suggest' && (
        <div className="anim-fade-in-fast fixed inset-0 z-50 flex justify-end bg-slate-900/20" onClick={() => setOverlay(null)}>
          <div className="glass anim-drawer-in flex h-full w-[460px] flex-col shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between border-b border-slate-100 dark:border-slate-800 px-3 py-2">
              <span className="text-[13px] font-bold text-slate-700 dark:text-slate-200">智能优化建议</span>
              <button onClick={() => setOverlay(null)} className="rounded p-1 text-slate-400 dark:text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-700/70 hover:text-slate-600">
                <X size={15} />
              </button>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto">
              <SuggestPanel backendRepo={backendRepo} map={map} onCreateTask={(d) => { setOverlay(null); openTaskDraft(d) }} />
            </div>
          </div>
        </div>
      )}
      {/* 任务表单（全局：地图/建议/工作区页共用） */}
      {taskDraft && (
        <TaskFormPanel
          key={`app-draft-${draftSeq}`}
          backendRepo={backendRepo}
          draft={taskDraft}
          map={map}
          onClose={() => setTaskDraft(null)}
          onCreated={(id) => {
            setTaskFocus({ id, nonce: Date.now() })
            setOnboarding((prev) => markCheck(prev, 'firstTask'))
            handlePageChange('tasks')
          }}
          onLocateModule={() => handlePageChange('map')}
          agentReady={agentState.found !== false}
        />
      )}
      {/* 新手引导：首启欢迎工作台（零仓库首启自动出现；顶栏 ? 可重看） */}
      {(welcomeOpen || (backendOnline === true && repos.length === 0 && !onboarding.welcomeShown)) && (
        <WelcomePage
          hasRepo={repos.length > 0}
          onAddRepo={async () => {
            const ok = await addRepo()
            if (ok) setOnboarding((prev) => markWelcomeShown(prev))
            return ok
          }}
          onClose={() => {
            setWelcomeOpen(false)
            setOnboarding((prev) => markWelcomeShown(prev))
          }}
        />
      )}
      {/* 新手引导：事件驱动 checklist（Linear 式；完成/关闭后不再打扰） */}
      {backendOnline === true && repos.length > 0 && !onboarding.dismissedAt && !welcomeOpen && (
        <div className="pointer-events-none fixed bottom-4 right-4 z-40">
          <OnboardingChecklist
            state={onboarding}
            onGo={(key) => {
              if (key === 'addRepo') void addRepo()
              else if (key === 'viewMap') handlePageChange('map')
              else if (key === 'viewHealth') handlePageChange('health')
              else if (key === 'firstTask') handlePageChange('workbench')
              else handlePageChange('tasks')
            }}
            onDismiss={() => setOnboarding((prev) => dismiss(prev))}
          />
        </div>
      )}
      <ToastHost />
    </>
  )
}
