import { useEffect, useMemo, useState, useCallback } from 'react'
import { ReactFlow, Controls, MiniMap, Panel, useReactFlow, type Node } from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { AlertTriangle, Focus, GitBranch, PanelRightOpen, RefreshCw, WifiOff } from 'lucide-react'

import type { CodeMap, SubMap } from '@/types/map'
import { healthColor } from '@/lib/layout'
import { DetailPanel, type Selection } from '@/components/DetailPanel'
import { TaskFormPanel } from '@/components/TaskFormPanel'
import type { TaskDraft } from '@/lib/taskContext'
import { onFreshnessEvent } from '@/lib/growthBus'
import { nodeTypes } from './nodeTypes'
import { buildFlow } from './buildFlow'
import { Legend } from './Legend'
import { FilterButton } from './FilterButton'
import { ModuleToolbar } from './ModuleToolbar'
import { GrowthPanel } from './GrowthPanel'
import { MAX_EXPANDED, type Filters } from './types'
import { useCanvasPanelDrag } from './useCanvasPanelDrag'
import { useSubmaps } from './useSubmaps'
import { useGrowthPlayback } from './useGrowthPlayback'
import { useRepoActivity } from '@/lib/useRepoActivity'
import { toast } from '@/lib/toast'

export function Canvas({
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
  const { tab, setTab, handleTabChange, startPanelDrag } = useCanvasPanelDrag(panelWidth, onPanelWidthChange)
  const [selection, setSelection] = useState<Selection>(null)
  const [filters, setFilters] = useState<Filters>({ violationsOnly: false, issuesOnly: false, solo: false })
  const [expandedIds, setExpandedIds] = useState<string[]>([])
  const [freshness, setFreshness] = useState<string | null>(null)
  const [freshnessInfo, setFreshnessInfo] = useState<{ commitsSinceMap?: number | null }>({})
  const [taskDraft, setTaskDraft] = useState<TaskDraft | null>(null)
  // R3 B2：draft 序号——每次打开新表单 +1 作 key，强制重置组件实例
  const [draftSeq, setDraftSeq] = useState(0)
  const openTaskDraft = (d: TaskDraft) => {
    setDraftSeq((s) => s + 1)
    setTaskDraft(d)
  }

  // 子图分析域（懒加载/深入分析/诚实三态/agent 输出直播）
  const { submaps, submapSessions, submapErrors, agentLines, retrySubmap, analyzeSubmap, ensureSubmapLoading } = useSubmaps(backendRepo)
  // 生长回放域（growth.log 消费 / WS 直播 / 推进定时器 / 重归纳 / 断线退出）
  const onPlaybackStart = useCallback(() => {
    setSelection(null)
    setExpandedIds([])
  }, [])
  const { growth, inducing, arrived, mergedMap, startReinduce, pauseGrowth, restartGrowth, exitGrowth } = useGrowthPlayback(map, backendRepo, {
    onPatrollingChange,
    onPlaybackStart,
  })
  // 跨页共享"归纳进行中"（19:29 实弹：漂移洞察发起的归纳，本页 inducing 仍为 false，
  // chip 的立即归纳可再点 → 重复入队）。任何来源的归纳活动会话/排队都计入。
  const shared = useRepoActivity(backendRepo)
  const inducingAny = inducing || shared.inducing
  const guardReinduce = useCallback(() => {
    if (inducingAny || shared.reinduceQueued) {
      toast(shared.reinduceQueued ? '归纳已排队：当前会话结束后自动接续' : '归纳进行中，无需重复发起', 'info')
      return
    }
    startReinduce()
  }, [inducingAny, shared.reinduceQueued, startReinduce])

  // 2026-10-04 实弹「渲染问题」修复：切仓库必须清掉上一仓库的画布状态——selection 是
  // keep-alive 的，旧仓库的模块 id 在新地图里不存在，nodeDim 的 neighborhood 判定会把
  // 新地图全部模块压到 0.3 透明度（整图洗白像蒙了层纱）；展开子图/过滤同理属于旧仓库上下文
  // （子图/生长态由各自 hook 内部重置，此处只清 Canvas 自有状态）
  useEffect(() => {
    setSelection(null)
    setExpandedIds([])
    setFilters({ violationsOnly: false, issuesOnly: false, solo: false })
  }, [backendRepo])

  // 真人测试反馈#1："只看依赖"是单程票——Esc 退出 + 底部常驻指示条
  useEffect(() => {
    if (!filters.solo) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setFilters((f) => ({ ...f, solo: false }))
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [filters.solo])

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

  const [headerExpanded, setHeaderExpanded] = useState(false)
  const { fitView, setCenter } = useReactFlow()

  const onSelectLayer = useCallback((id: string) => {
    setSelection({ kind: 'layer', id })
    setTab('detail')
    onPanelOpenChange(true)
  }, [])

  const toggleExpand = useCallback((id: string) => {
    setExpandedIds((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id].slice(-MAX_EXPANDED)))
    ensureSubmapLoading(id)
  }, [ensureSubmapLoading])

  const expanded = useMemo(() => {
    const m = new Map<string, SubMap | 'loading' | 'error'>()
    for (const id of expandedIds) if (submaps[id]) m.set(id, submaps[id])
    return m
  }, [expandedIds, submaps])

  const emptyExpanded = useMemo(() => new Map<string, SubMap | 'loading' | 'error'>(), [])
  const effectiveExpanded = growth ? emptyExpanded : expanded
  const growthVisible = growth ? arrived : null

  const { nodes, edges } = useMemo(
    () => buildFlow(mergedMap, selection, filters, effectiveExpanded, onSelectLayer, retrySubmap, growthVisible, analyzeSubmap, (id) => agentLines[submapSessions[id] ?? ''], (id) => submapErrors[id], toggleExpand, onOpenRuns, (id) => submapSessions[id]),
    [mergedMap, selection, filters, effectiveExpanded, onSelectLayer, retrySubmap, growthVisible, analyzeSubmap, agentLines, submapSessions, submapErrors, toggleExpand],
  )

  // S1-1 画布定位收口：pan/zoom 到目标模块 + 选中高亮。
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

  // S1-1 视图打开：定位首模块 + 多模块视图自动 solo 聚焦
  const openView = useCallback(
    (ids: string[]) => {
      if (ids.length === 0) return
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

  // 2026-10-05 依赖透镜跳入：选中 + solo 聚焦
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
      // M4-1.5 叙事重排（陪审团）：进图先给诊断——存在显著风险时聚焦最红的模块
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
      const alreadySelected = selection?.kind === 'module' && selection.id === node.id
      if (node.type === 'module' && alreadySelected) toggleExpand(node.id)
      setSelection({ kind: 'module', id: node.id })
      setTab('detail')
      onPanelOpenChange(true)
    } else if (node.type === 'submodule' && node.parentId) {
      if ((node.data as { loading?: boolean }).loading) return
      const subId = node.id.slice(`sub:${node.parentId}:`.length)
      setSelection({ kind: 'submodule', parentId: node.parentId, subId })
      setTab('detail')
      onPanelOpenChange(true)
    }
  }, [growth, selection, toggleExpand, setTab, onPanelOpenChange])
  const onPaneClick = useCallback(() => setSelection(null), [])

  const toggleFilter = (key: keyof Filters) => setFilters((f) => ({ ...f, [key]: !f[key] }))

  // 2026-10-05 依赖透镜：边浮卡——fixed 定位跟随鼠标
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
                inducing={inducingAny}
                solo={filters.solo}
                onToggleExpand={() => toggleExpand(selModule.id)}
                onToggleSolo={() => toggleFilter('solo')}
                onReinduce={guardReinduce}
                onChat={onChatAbout ? () => onChatAbout({ refId: selModule.id, refName: selModule.name, kind: 'module' }) : undefined}
              />
            </Panel>
          )}

          {/* 底部中央：生长回放控制条 或 全局过滤 */}
          <Panel position="bottom-center" className="mb-2">
            {growth ? (
              <GrowthPanel
                growth={growth}
                onPause={pauseGrowth}
                onRestart={restartGrowth}
                onExit={exitGrowth}
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
                    {backendRepo && !inducingAny && !shared.reinduceQueued && (
                      <button
                        onClick={guardReinduce}
                        className="ml-0.5 flex items-center gap-0.5 rounded-full bg-white/80 dark:bg-slate-800/80 px-1.5 py-px text-micro font-bold text-red-600 dark:text-amber-300 shadow-sm transition-colors hover:bg-white dark:hover:bg-slate-700"
                        title="立即重新归纳：agent 按 v2.2 协议重跑，全程直播"
                      >
                        <RefreshCw size={8} /> 立即归纳
                      </button>
                    )}
                  </span>
                )}
                {inducingAny && (
                  <span className="flex items-center gap-1 font-semibold text-amber-600">
                    <RefreshCw size={10} className="animate-spin" />
                    {shared.reinduceQueued ? '归纳排队中…' : '归纳中…'}
                  </span>
                )}
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
