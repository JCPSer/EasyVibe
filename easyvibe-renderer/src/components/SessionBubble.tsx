import { useCallback, useEffect, useReducer, useRef, useState } from 'react'
import { Clock, HeartPulse, Search, Sparkles, X } from 'lucide-react'
import { onQueueChanged, onSessionEvent } from '@/lib/growthBus'
import { toast } from '@/lib/toast'
import { absTime, toMs } from '@/lib/diffStat'
import { formatElapsed, isEmptyState, kindFromLabel, type SessionQueueKind, type SessionQueueSnapshot } from '@/lib/sessionQueue'

const KIND_NAME: Record<SessionQueueKind, string> = { patrol: '巡检', reinduce: '归纳', submap: '深入分析' }

// 运行会话指示器（docs/requirements-session-bubble-queue.md / ui-mockups/顶部会话指示器设计-v1.png）：
// 顶栏正中央细条状态丸（2026-10-04 二次改版——右下胶囊挪走后用户要求回顶部，但右侧按钮群拥挤，
// 顶栏正中大片留白是天然位置）：26px 高单行信息——迷你转环+类型图标、脉冲点、label·进行中、
// 已运行时长；排队时紧跟琥珀小 chip（行内 × 取消）。悬停向下弹详情卡（详情+取消）。
// 空态（无活动且无排队）不渲染。

export function SessionBubble({ backendRepo, resyncKey = 0 }: { backendRepo: string | null; resyncKey?: number }) {
  const [snap, setSnap] = useState<SessionQueueSnapshot | null>(null)
  // 终态红态：failed → 记住 label，红态 5s 后消失
  const [failed, setFailed] = useState<{ label: string } | null>(null)
  const snapRef = useRef<SessionQueueSnapshot | null>(null)
  useEffect(() => {
    snapRef.current = snap
  }, [snap])

  const pull = useCallback(() => {
    if (!backendRepo) return // 无仓库时组件整体不渲染（return null），无需清快照
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/session-queue`)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: SessionQueueSnapshot } | null) => setSnap(d?.data ?? null))
      .catch(() => {})
  }, [backendRepo])

  // I5 刷新三挂钩：① session.statusChanged / queue.changed 事件；② WS onopen 重连（resyncKey）；
  // ③ 20s 低频定时硬兜底。backendRepo 变化（切仓库）自然重拉。
  useEffect(() => {
    pull()
    const offQ = onQueueChanged((e) => {
      if (e.repo !== backendRepo) return
      // B2 裁决：drain 启动失败必须可见——不留死信也不只留 warn 日志
      if (e.type === 'failed') toast(`排队任务启动失败：${e.error ?? '未知原因'}`, 'error')
      pull()
    })
    const offS = onSessionEvent((e) => {
      if (e.repo !== backendRepo) return
      pull()
    })
    const t = window.setInterval(pull, 20000)
    return () => {
      offQ()
      offS()
      window.clearInterval(t)
    }
  }, [backendRepo, pull, resyncKey])

  // 会话终态：failed → 记住 label，红态 5s 后消失
  useEffect(
    () =>
      onSessionEvent((e) => {
        if (e.repo !== backendRepo || e.status !== 'failed') return
        const label = snapRef.current?.active?.label ?? e.sessionId
        setFailed({ label })
        toast(`「${label}」执行失败`, 'error')
        window.setTimeout(() => setFailed((f) => (f?.label === label ? null : f)), 5000)
      }),
    [backendRepo],
  )

  const startedMs = snap?.active?.startedAt ? toMs(snap.active.startedAt) : null
  // 2026-10-04 实弹修复：此前 now 是 useState 快照 + 1s interval 推——定时器被 webview
  // 节流/挂起时快照过期，elapsed 会从 0 附近重新计数（截图实证：启动 21:44 却显示已运行 0:17）。
  // 改为 interval 只负责强制重渲染，每次渲染从 Date.now() 现取 wall clock——
  // 即使渲染被冻结，停住的也是"正确值"，恢复后第一时间追上真实时长。
  const [, forceRender] = useReducer((x: number) => x + 1, 0)
  const needTicker = startedMs !== null || !!failed
  useEffect(() => {
    if (!needTicker) return
    const t = window.setInterval(forceRender, 1000)
    return () => window.clearInterval(t)
  }, [needTicker])

  if (!backendRepo || (isEmptyState(snap) && !failed)) return null

  // 每次渲染现取 wall clock（非 hook，可在 early return 之后）——计时永不依赖可能过期的快照
  // eslint-disable-next-line react-hooks/purity -- 时长必须在渲染时读真实时钟：定时器被 webview 节流时状态快照会过期导致"从0重计"（2026-10-04 实弹修复，见 git log）
  const now = Date.now()
  const active = failed ? null : snap?.active
  const queued = snap?.queued ?? null
  const kind: SessionQueueKind = active
    ? kindFromLabel(active.label)
    : queued
      ? queued.kind
      : kindFromLabel(failed!.label)
  const Icon = kind === 'patrol' ? HeartPulse : kind === 'submap' ? Search : Sparkles
  const elapsed = startedMs !== null ? formatElapsed(now - startedMs) : null

  const kill = () => {
    if (!backendRepo || !active) return
    if (!window.confirm(`确定取消「${active.label}」？该操作不可撤销。`)) return
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/sessions/${encodeURIComponent(active.sessionId)}/kill`, {
      method: 'POST',
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        toast(`已取消「${active.label}」`)
        pull()
      })
      .catch(() => toast('取消失败（请确认后端在线后重试）。', 'error'))
  }

  const cancelQueue = () => {
    if (!backendRepo) return
    fetch(`/api/repos/${encodeURIComponent(backendRepo)}/session-queue`, { method: 'DELETE' })
      .then(async (r) => {
        if (r.status === 404) {
          const b = (await r.json().catch(() => null)) as { error?: string } | null
          toast(b?.error ?? '没有排队任务', 'error')
        } else if (!r.ok) {
          toast(`取消排队失败（HTTP ${r.status}）`, 'error')
        } else {
          toast('已取消排队')
        }
        pull()
      })
      .catch(() => toast('取消排队失败（请确认后端在线后重试）。', 'error'))
  }

  return (
    <div className="group relative select-none" data-no-drag>
      {/* 顶部细条状态丸：26px 高单行——迷你转环 + 脉冲点 + label·进行中 + 已运行时长 */}
      <div
        className={`glass elev-2 flex h-[26px] items-center gap-1.5 rounded-full border pl-1 pr-2.5 ${
          failed ? 'border-red-300 dark:border-red-800' : 'border-slate-200 dark:border-slate-700'
        }`}
      >
        <span className="relative flex h-[18px] w-[18px] shrink-0 items-center justify-center">
          {active && (
            <svg className="absolute inset-0 animate-spin" style={{ animationDuration: '2.4s' }} viewBox="0 0 18 18" aria-hidden>
              <circle cx="9" cy="9" r="7.5" fill="none" stroke={failed ? '#fecaca' : '#e0e7ff'} strokeWidth="2" />
              <circle
                cx="9"
                cy="9"
                r="7.5"
                fill="none"
                stroke={failed ? '#ef4444' : '#6366f1'}
                strokeWidth="2"
                strokeLinecap="round"
                strokeDasharray="12 35"
              />
            </svg>
          )}
          {active ? (
            <Icon size={10} className={failed ? 'text-red-500' : 'text-indigo-500'} />
          ) : (
            <Clock size={10} className="text-amber-500" />
          )}
        </span>
        <span
          className={`h-1.5 w-1.5 shrink-0 rounded-full ${
            failed ? 'bg-red-500' : active ? 'animate-pulse bg-indigo-500' : 'animate-pulse bg-amber-500'
          }`}
        />
        <span className="max-w-[200px] truncate whitespace-nowrap text-cap font-semibold leading-none text-slate-700 dark:text-slate-200">
          {failed ? `${failed.label} · 失败` : active ? `${active.label} · 进行中` : `排队中：${queued?.label ?? ''}`}
        </span>
        <span className="tnum whitespace-nowrap text-micro leading-none text-slate-400 dark:text-slate-500">
          {failed ? '会话已结束' : active ? (elapsed !== null ? `已运行 ${elapsed}` : '进行中…') : '结束后自动开始'}
        </span>
        {/* 排队 chip：琥珀小胶囊，行内 × 随时取消 */}
        {queued && (
          <span className="flex items-center gap-1 rounded-full border border-amber-200 bg-amber-50 py-0.5 pl-1.5 pr-0.5 text-micro font-medium leading-none text-amber-700 dark:border-amber-900/60 dark:bg-amber-950/50 dark:text-amber-300">
            <Clock size={9} />
            <span className="max-w-[110px] truncate">排队:{queued.label}</span>
            <button
              onClick={cancelQueue}
              className="rounded-full p-0.5 text-amber-400 transition-colors hover:bg-amber-100 hover:text-amber-700 dark:text-amber-600 dark:hover:bg-amber-900/50 dark:hover:text-amber-300"
              title="取消排队"
            >
              <X size={9} />
            </button>
          </span>
        )}
      </div>

      {/* 悬停详情卡：从状态丸向下弹出（活动会话时）；含会话详情 + 取消 */}
      {active && !failed && (
        <div className="glass elev-3 anim-scale-in pointer-events-none invisible absolute left-1/2 top-full z-50 mt-1.5 w-64 -translate-x-1/2 rounded-xl border border-slate-200 p-3 opacity-0 transition-opacity duration-150 group-hover:pointer-events-auto group-hover:visible group-hover:opacity-100 dark:border-slate-700">
          <p className="text-cap font-bold text-slate-700 dark:text-slate-200">{active.label}</p>
          <dl className="mt-1.5 space-y-0.5 text-micro text-slate-500 dark:text-slate-400">
            <div className="flex justify-between">
              <dt>类型</dt>
              <dd className="font-semibold text-slate-600 dark:text-slate-300">{KIND_NAME[kind]}</dd>
            </div>
            <div className="flex justify-between">
              <dt>会话</dt>
              <dd className="mono">{active.sessionId}</dd>
            </div>
            {startedMs !== null && (
              <div className="flex justify-between">
                <dt>启动时间</dt>
                <dd className="tnum">{absTime(new Date(startedMs).toISOString()).slice(5)}</dd>
              </div>
            )}
            {elapsed !== null && (
              <div className="flex justify-between">
                <dt>已运行</dt>
                <dd className="tnum">{elapsed}</dd>
              </div>
            )}
          </dl>
          <button
            onClick={kill}
            className="mt-2 flex w-full items-center justify-center gap-1 rounded-lg border border-red-200 bg-red-50 px-2 py-1 text-micro font-semibold text-red-600 transition-colors hover:bg-red-100 dark:border-red-900/60 dark:bg-red-950/40 dark:text-red-300 dark:hover:bg-red-900/40"
            title="取消该会话（二次确认）"
          >
            <X size={10} />
            取消该会话
          </button>
        </div>
      )}
    </div>
  )
}
