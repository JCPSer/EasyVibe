import { ToastHost, toast } from '@/lib/toast'
import { useEffect, useRef, useState, useCallback } from 'react'
import { Loader2 } from 'lucide-react'

import { markWelcomeShown, dismiss, resetForReview } from '@/lib/onboarding'
import { AppShell, type PageId } from '@/components/AppShell'
import { TopBar } from '@/components/shell/TopBar'
import { AttentionBar } from '@/components/shell/AttentionBar'
import { buildPages } from '@/pages/routes'
import { SessionBubble } from '@/components/SessionBubble'
import { OnboardingChecklist } from '@/components/OnboardingChecklist'
import { MapGate } from '@/components/gate/MapGate'
import { CanvasBoundary } from '@/components/canvas/CanvasBoundary'
import { ViewsDrawer } from '@/components/overlays/ViewsDrawer'
import { SuggestDrawer } from '@/components/overlays/SuggestDrawer'
import { WelcomeOverlay } from '@/components/overlays/WelcomeOverlay'
import { TaskDraftOverlay } from '@/components/overlays/TaskDraftOverlay'
import type { TaskDraft } from '@/lib/taskContext'
import { isTauriRuntime } from '@/lib/env'
import { downloadHealthReport } from '@/lib/healthReport'
import { initUpdater } from '@/lib/updater'
import { connectWs } from '@/lib/ws'
import { useAgentState } from '@/hooks/useAgentState'
import { useOnboarding } from '@/hooks/useOnboarding'
import { useBackendConnection } from '@/hooks/useBackendConnection'
import { useTheme } from '@/hooks/useTheme'
import { useAttention } from '@/hooks/useAttention'
import { usePatrol } from '@/hooks/usePatrol'
import { useUiPrefs } from '@/hooks/useUiPrefs'
import { useSystemNotifications } from '@/hooks/useSystemNotifications'
export default function App() {
  // 数据源装配域（后端探测 / 仓库管理 / 地图拉取 / 版本锚点）抽为 hook
  const {
    map,
    error,
    backendRepo,
    repos,
    backendOnline,
    setReloadTick,
    serverVersionRef,
    addRepo,
    removeRepo,
    switchRepo,
  } = useBackendConnection()
  // 运行会话气泡重拉信号：WS onopen（重连）时递增，作为 resyncKey 传给 SessionBubble（I5②）
  const [queueResyncTick, setQueueResyncTick] = useState(0)

  // M2 引导与降级：执行 agent 状态（探测 + 30s 轮询 + 采用 + 安装命令）抽为 hook
  const { agentState, adoptAgent, copyInstallCmd } = useAgentState(backendOnline)
  // D5-2：桌面壳自动更新（仅 Tauri 环境生效，浏览器 no-op）
  useEffect(() => {
    initUpdater()
  }, [])

  // 应用壳徽标/注意力条数据 + 主题（各自 hook）
  const { pendingApprovals, runningCount, attention } = useAttention(backendRepo)
  const { dark, setDark } = useTheme()
  // M4-1 应用壳状态：页面 / 顶栏抽屉 / 仓库管理面板 / 引导卡 / 任务表单 / 视图定位请求
  const [page, setPage] = useState<PageId>('map')
  // v0.2：地图页「就此对话」跨页上下文（消费即清；后写覆盖先写，竞态语义自然）
  const [pendingChatContext, setPendingChatContext] = useState<{ refId: string; refName: string; kind: 'module' | 'layer' } | null>(null)
  /** 2026-10-05 依赖体检：画布边浮卡跳入时的聚焦卡片 id */
  const [depsFocus, setDepsFocus] = useState<string | null>(null)
  /** 2026-10-05 依赖透镜：DepsPage「在画布上看」→ 画布选中并开 solo（filters 状态本体在 Canvas） */
  const [lensRequest, setLensRequest] = useState<string | null>(null)
  /** 2026-10-05 用量页/任务页/画布 → 运行页会话 deeplink */
  const [runsFocus, setRunsFocus] = useState<string | null>(null)
  // 新手引导状态机（B5）抽为 hook：版本化状态 + 欢迎页开合 + 事件驱动勾选 + firstApproval 轮询 + 收尾
  const { onboarding, setOnboarding, welcomeOpen, setWelcomeOpen, markFirstTask } = useOnboarding({ page, hasMap: !!map, repoCount: repos.length, backendRepo })
  // ui-test P1：表单创建成功 → 任务页流水线选中该任务（nonce 区分多次跳入）
  const [taskFocus, setTaskFocus] = useState<{ id: string; nonce: number } | null>(null)
  const [overlay, setOverlay] = useState<'views' | 'suggest' | null>(null)
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
  // 巡检触发（顶栏"巡检"按钮由壳层持有）
  const { patrolling, setPatrolling, startPatrol } = usePatrol(backendRepo)

  // 每项目 UI 偏好（右栏宽度/开合持久化 + ui.* 写入）
  const resetToMap = useCallback(() => setPage('map'), [])
  const { saveUiPref, panelOpen, panelWidth, handlePanelOpenChange, handlePanelWidthChange } = useUiPrefs(backendRepo, resetToMap)

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

  // 系统级通知（会话失败 / 排队任务已开始）
  useSystemNotifications(backendRepo)

  // v0.2：「就此对话」统一收口——带上下文跳「任务对话」页（Canvas 工具栏/详情视图/占位页签共用）
  const goChatAbout = useCallback(
    (target: { refId: string; refName: string; kind: 'module' | 'layer' }) => {
      setPendingChatContext(target)
      handlePageChange('workbench')
    },
    [handlePageChange],
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

  // WS 订阅：连接/重连/协议翻译下沉 lib/ws.ts；App 只把产品事件接进数据源
  // 闭包陷阱对策：repo 经 ref 读取（effect 仍随 backendRepo 重连，语义与拆分前一致）
  const repoRef = useRef(backendRepo)
  repoRef.current = backendRepo
  useEffect(() => {
    if (!backendRepo) return
    return connectWs({
      getRepo: () => repoRef.current,
      versionRef: serverVersionRef,
      onMapChanged: () => setReloadTick((t) => t + 1),
      onReconnected: () => {
        setReloadTick((t) => t + 1) // 全量重同步（覆盖断线期间的变更）
        setQueueResyncTick((t) => t + 1) // I5②：运行会话气泡重拉队列快照（resyncKey 递增）
      },
      onVersionChange: (prev, v) => toast(`后端已更新（${prev} → ${v}），刷新页面以加载新界面`, 'error'),
      onGoTasks: () => handlePageChange('tasks'),
    })
    // handlePageChange 仅用于事件回调内的瞬态动作（跳页），不应重启 WS 连接
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [backendRepo])

  // P0 审查前端#1：地图加载失败不再一律渲染"归纳中"——
  const openRunsSession = useCallback((sessionId: string) => {
    setRunsFocus(sessionId)
    handlePageChange('runs')
  }, [handlePageChange])

  if (error && !backendRepo) {
    return (
      <div className="flex h-screen items-center justify-center text-[13px] text-red-500">
        静态数据加载失败（/data/map.json）：{error}
      </div>
    )
  }


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
  const topBar = (
    <TopBar
      backendOnline={backendOnline}
      backendRepo={backendRepo}
      repos={repos}
      onSwitchRepo={switchRepo}
      onAddRepo={addRepo}
      onRemoveRepo={removeRepo}
      onToggleViews={() => setOverlay((o) => (o === 'views' ? null : 'views'))}
      onToggleSuggest={() => setOverlay((o) => (o === 'suggest' ? null : 'suggest'))}
      patrolling={patrolling}
      onStartPatrol={startPatrol}
      agentReady={agentState.found !== false}
      onExport={() => downloadHealthReport(map)}
      dark={dark}
      onToggleDark={setDark}
      welcomeOpen={welcomeOpen}
      onToggleWelcome={() => {
        if (welcomeOpen) {
          setWelcomeOpen(false)
          return
        }
        setOnboarding(resetForReview())
        setWelcomeOpen(true)
      }}
      page={page}
      onOpenSettings={() => handlePageChange('settings')}
    />
  )

  const PAGES = buildPages({
    map,
    backendRepo,
    onPatrollingChange: setPatrolling,
    panelOpen,
    onPanelOpenChange: handlePanelOpenChange,
    panelWidth,
    onPanelWidthChange: handlePanelWidthChange,
    viewRequest,
    onViewRequestConsumed: () => setViewRequest(null),
    agentReady: agentState.found !== false,
    guideDismissed,
    onDismissGuide: () => {
      setGuideDismissed(true)
      localStorage.setItem('ev.m4.guide', '1')
    },
    onTaskCreated: (id) => {
      setTaskFocus({ id, nonce: Date.now() })
      markFirstTask()
      handlePageChange('tasks')
    },
    onOpenTask: openTaskDraft,
    onChatAbout: goChatAbout,
    onGoWorkbench: () => handlePageChange('workbench'),
    onInspectEdge: (cardId) => {
      setDepsFocus(cardId)
      handlePageChange('deps')
    },
    onOpenRuns: openRunsSession,
    onOpenDeps: () => handlePageChange('deps'),
    lensRequest,
    onLensRequestConsumed: () => setLensRequest(null),
    dark,
    page,
    taskFocus,
    runsFocus,
    onRunsFocusConsumed: () => setRunsFocus(null),
    queueResyncTick,
    onOpenChangesToTasks: (id) => {
      setTaskFocus({ id, nonce: Date.now() })
      handlePageChange('tasks')
    },
    onRequestView: (ids) => {
      setViewRequest(ids)
      handlePageChange('map')
    },
    onRequestLens: (id) => {
      setLensRequest(id)
      handlePageChange('map')
    },
    onDepsFocusConsumed: () => setDepsFocus(null),
    depsFocus,
    pendingChatContext,
    onConsumeChatContext: () => setPendingChatContext(null),
    onNavigate: handlePageChange,
  })

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
            <AttentionBar
              backendOnline={backendOnline}
              repoCount={repos.length}
              agentState={agentState}
              attention={attention}
              page={page}
              onAddRepo={() => addRepo()}
              onAdoptAgent={(cmd) => void adoptAgent(cmd)}
              onCopyInstallCmd={copyInstallCmd}
              onOpenSettings={() => handlePageChange('settings')}
              onGoTasks={() => handlePageChange('tasks')}
            />
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
        <ViewsDrawer
          backendRepo={backendRepo}
          map={map}
          onClose={() => setOverlay(null)}
          onOpenView={(ids) => {
            setOverlay(null)
            handlePageChange('map')
            setViewRequest(ids)
          }}
        />
      )}
      {overlay === 'suggest' && (
        <SuggestDrawer
          backendRepo={backendRepo}
          map={map}
          onClose={() => setOverlay(null)}
          onCreateTask={(d) => {
            setOverlay(null)
            openTaskDraft(d)
          }}
        />
      )}
      {/* 任务表单（全局：地图/建议/工作区页共用） */}
      {taskDraft && (
        <TaskDraftOverlay
          backendRepo={backendRepo}
          taskDraft={taskDraft}
          draftSeq={draftSeq}
          map={map}
          agentReady={agentState.found !== false}
          onClose={() => setTaskDraft(null)}
          onCreated={(id) => {
            setTaskFocus({ id, nonce: Date.now() })
            markFirstTask()
            handlePageChange('tasks')
          }}
          onLocateModule={() => handlePageChange('map')}
        />
      )}
      {/* 新手引导：首启欢迎工作台（零仓库首启自动出现；顶栏 ? 可重看） */}
      {(welcomeOpen || (backendOnline === true && repos.length === 0 && !onboarding.welcomeShown)) && (
        <WelcomeOverlay
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
