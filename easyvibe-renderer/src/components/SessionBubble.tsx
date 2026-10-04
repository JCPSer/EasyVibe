import { useCallback, useEffect, useRef, useState } from 'react'
import { Clock, HeartPulse, Search, Sparkles, X } from 'lucide-react'
import { onQueueChanged, onSessionEvent } from '@/lib/growthBus'
import { toast } from '@/lib/toast'
import { absTime, toMs } from '@/lib/diffStat'
import { formatElapsed, isEmptyState, kindFromLabel, type SessionQueueKind, type SessionQueueSnapshot } from '@/lib/sessionQueue'

const KIND_NAME: Record<SessionQueueKind, string> = { patrol: '巡检', reinduce: '归纳', submap: '深入分析' }

// 运行会话气泡（docs/requirements-session-bubble-queue.md / ui-mockups/运行会话气泡设计-v1.png）：
// 顶栏右区常驻玻璃拟态胶囊——进度环 + 类型图标 + 脉冲状态点 + 「{label} · 进行中」+ 已运行时长；
// 悬停展开详情卡（含取消会话），气泡下方常驻排队卡（含 × 取消排队）。
// 空态（无活动且无排队）不渲染。

export function SessionBubble({ backendRepo, resyncKey = 0 }: { backendRepo: string | null; resyncKey?: number }) {
  const [snap, setSnap] = useState<SessionQueueSnapshot | null>(null)
  // 终态红态：failed → 红态短暂停留 5s + toast（session.statusChanged 事件驱动）
  const [failed, setFailed] = useState<{ label: string } | null>(null)
  const [now, setNow] = useState(() => Date.now())
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
  // 1s 本地 ticker：只在需要展示时长（或红态倒计时观感）时跑
  const needTicker = startedMs !== null || !!failed
  useEffect(() => {
    if (!needTicker) return
    const t = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(t)
  }, [needTicker])

  if (!backendRepo || (isEmptyState(snap) && !failed)) return null

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
    <div className="group relative" data-no-drag>
      {/* 常驻胶囊：玻璃拟态（.glass 已含 blur + 降级）；进度环 2.4s 匀速旋转 */}
      <div
        className={`glass elev-2 flex items-center gap-2 rounded-full border py-1 pl-1.5 pr-3 ${
          failed ? 'border-red-300 dark:border-red-800' : 'border-slate-200 dark:border-slate-700'
        }`}
      >
        <span className="relative flex h-7 w-7 shrink-0 items-center justify-center">
          {active && (
            <svg className="absolute inset-0 animate-spin" style={{ animationDuration: '2.4s' }} viewBox="0 0 28 28" aria-hidden>
              <circle cx="14" cy="14" r="12" fill="none" stroke={failed ? '#fecaca' : '#e0e7ff'} strokeWidth="2.5" />
              <circle
                cx="14"
                cy="14"
                r="12"
                fill="none"
                stroke={failed ? '#ef4444' : '#6366f1'}
                strokeWidth="2.5"
                strokeLinecap="round"
                strokeDasharray="20 55.5"
              />
            </svg>
          )}
          {active ? (
            <Icon size={13} className={failed ? 'text-red-500' : 'text-indigo-500'} />
          ) : (
            <Clock size={13} className="text-amber-500" />
          )}
        </span>
        <span className="flex min-w-0 flex-col leading-none">
          <span className="flex items-center gap-1.5 whitespace-nowrap text-cap font-semibold text-slate-700 dark:text-slate-200">
            <span
              className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                failed ? 'bg-red-500' : active ? 'animate-pulse bg-indigo-500' : 'animate-pulse bg-amber-500'
              }`}
            />
            {failed ? `${failed.label} · 失败` : active ? `${active.label} · 进行中` : `排队中：${queued?.label ?? ''}`}
          </span>
          <span className="tnum mt-0.5 text-micro text-slate-400 dark:text-slate-500">
            {failed ? '会话已结束' : active ? (elapsed !== null ? `已运行 ${elapsed}` : '进行中…') : '当前会话结束后自动开始'}
          </span>
        </span>
      </div>

      {/* 展开区：悬停详情卡（活动会话时）+ 常驻排队卡 */}
      {(active || queued) && (
        <div className="absolute right-0 top-full z-40 mt-1.5 flex w-64 flex-col items-stretch gap-1.5">
          {active && !failed && (
            <div className="glass elev-3 anim-scale-in pointer-events-none invisible rounded-xl border border-slate-200 p-3 opacity-0 transition-opacity duration-150 group-hover:pointer-events-auto group-hover:visible group-hover:opacity-100 dark:border-slate-700">
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
          {queued && (
            <div className="glass elev-3 anim-msg-in flex items-center gap-2 rounded-xl border border-slate-200 px-3 py-2 dark:border-slate-700">
              <Clock size={12} className="shrink-0 text-amber-500" />
              <span className="min-w-0 flex-1 text-cap leading-4 text-slate-600 dark:text-slate-300">
                排队中：{queued.label}（当前会话结束后自动开始）
              </span>
              <button
                onClick={cancelQueue}
                className="shrink-0 rounded-full p-1 text-slate-300 transition-colors hover:bg-slate-100 hover:text-slate-600 dark:text-slate-600 dark:hover:bg-slate-800 dark:hover:text-slate-300"
                title="取消排队"
              >
                <X size={12} />
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  )
}
