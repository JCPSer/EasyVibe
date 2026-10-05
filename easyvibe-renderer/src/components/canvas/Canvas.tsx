import { useEffect, useMemo, useRef, useState, useCallback } from 'react'
import { ReactFlow, Controls, MiniMap, Panel, useReactFlow, type Node } from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import { AlertTriangle, Focus, GitBranch, PanelRightOpen, Play, RefreshCw, WifiOff } from 'lucide-react'

import type { CodeMap, SubMap } from '@/types/map'
import { healthColor } from '@/lib/layout'
import { toast } from '@/lib/toast'
import { DetailPanel, type Selection, type PanelTab } from '@/components/DetailPanel'
import { TaskFormPanel } from '@/components/TaskFormPanel'
import type { TaskDraft } from '@/lib/taskContext'
import { enqueue } from '@/lib/sessionQueue'
import { mergeGrowthEvents, parseGrowthText, isValidGrowthEvent } from '@/lib/growthMerge'
import { onFreshnessEvent, onGrowthEvent, onPatrolFinished, onQueueChanged, onSessionEvent, onSessionOutput, setWsCloseListener } from '@/lib/growthBus'
import { nodeTypes } from './nodeTypes'
import { buildFlow } from './buildFlow'
import { Legend } from './Legend'
import { FilterButton } from './FilterButton'
import { ModuleToolbar } from './ModuleToolbar'
import { GrowthPanel } from './GrowthPanel'
import { MAX_EXPANDED, type Filters, type GrowthState } from './types'

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
