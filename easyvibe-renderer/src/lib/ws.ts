// WS 连接层（R3）：连接 / 指数退避重连 / onopen 全量重同步与版本比对 /
// 10 类事件名 → growthBus 分发 / onclose 断线通知。
// 与 growthBus（进程内分发层，13 个页面消费）分离：本文件只负责网络与协议翻译。
// 闭包陷阱对策：所有时变值经回调/ref 注入，不捕获调用方 state 快照（沿用既有 serverVersionRef 口径）。
import { dismissToast, toast } from '@/lib/toast'
import { track } from '@/lib/analytics'
import { sysNotify } from '@/lib/notify'
import { pushTerminalLine } from '@/lib/terminalBuffer'
import {
  emitFreshnessEvent,
  emitGrowthEvent,
  emitPatrolFinished,
  emitQueueChanged,
  emitSessionEvent,
  emitSessionOutput,
  emitTaskEvent,
  notifyWsClosed,
} from '@/lib/growthBus'

export type ConnectWsOptions = {
  /** 当前仓库（ref 读取，规避 onopen/onmessage 闭包读初值） */
  getRepo: () => string | null
  /** 后端版本比对锚点（跨重连保留，由调用方持有） */
  versionRef: { current: string | null }
  /** map.changed：触发数据源重取 */
  onMapChanged: () => void
  /** 连接建立/重连：全量重同步（地图重取 + 队列快照重拉） */
  onReconnected: () => void
  /** 后端版本变化：提示刷新（prev → next） */
  onVersionChange: (prev: string, next: string) => void
  /** 任务类事件「去处理」跳转 */
  onGoTasks: () => void
  /** 可注入的 WebSocket 工厂（单测打桩用；默认 globalThis.WebSocket） */
  createSocket?: (url: string) => WebSocket
}

export function connectWs(opts: ConnectWsOptions): () => void {
  let closed = false
  let ws: WebSocket | null = null
  let retry = 0
  let timer: number | undefined
  const create = opts.createSocket ?? ((url: string) => new WebSocket(url))

  const connect = () => {
    if (closed) return
    const proto = location.protocol === 'https:' ? 'wss' : 'ws'
    ws = create(`${proto}://${location.host}/ws`)
    ws.onopen = () => {
      retry = 0
      // 全量重同步（覆盖断线期间的变更）+ 队列快照重拉
      opts.onReconnected()
      // Y7：重连点比对后端版本——热重启/更新后前端是旧契约，提示刷新
      fetch('/api/health')
        .then((r) => r.json())
        .then((h: { data?: { version?: string } }) => {
          const v = h.data?.version
          const prev = opts.versionRef.current
          if (v && prev && v !== prev) {
            opts.onVersionChange(prev, v)
          }
          if (v) {
            opts.versionRef.current = v
          }
        })
        .catch(() => {})
    }
    ws.onmessage = (e) => {
      try {
        const msg = JSON.parse(e.data)
        const repo = opts.getRepo()
        if (msg.name === 'map.changed' && msg.data?.repo === repo) opts.onMapChanged()
        if (msg.name === 'growth.event' && msg.data?.repo === repo) emitGrowthEvent(msg.data.event)
        if (msg.name === 'session.statusChanged') {
          const d = msg.data
          if (d?.repo === repo) emitSessionEvent({ repo: d.repo, sessionId: d.sessionId, status: d.status })
        }
        if (msg.name === 'queue.changed') {
          // 会话排队变更（入队/替换/取消/排空/失败）——SessionBubble 与 drained 接管逻辑消费
          const d = msg.data
          if (d?.repo === repo)
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
        if (msg.name === 'freshness.changed' && msg.data?.repo === repo) {
          emitFreshnessEvent({
            repo: msg.data.repo,
            status: msg.data.status,
            latestCommitAt: msg.data.latestCommitAt,
            commitsSinceMap: msg.data.commitsSinceMap,
          })
        }
        if (msg.name === 'task.contractAlert') {
          // L2 过程预警：任务执行中哨兵抓到的新增越界——比终态红线早 N 分钟到达
          const d = msg.data
          if (d?.repo !== repo) return
          track(d.repo, 'ui.contractAlert.shown', { taskId: d.taskId })
          toast(`影响面预警：有任务正在越界改动（${(d.files ?? []).slice(0, 2).join('、')}${(d.files?.length ?? 0) > 2 ? ' 等' : ''}）`, 'info', {
            label: '去处理',
            onClick: () => {
              track(d.repo, 'ui.contractAlert.click', { taskId: d.taskId })
              opts.onGoTasks()
            },
          }, true, `contract:${d.taskId}`)
          if (document.hidden) void sysNotify('EasyVibe · 影响面预警', `有任务正在越界：${(d.files ?? []).slice(0, 3).join('、')}`)
        }
        if (msg.name === 'task.contractViolated') {
          // R2 裂缝#3：auto/supervised 任务无审批关——越界经 WS 主动送达（与审批通知同双通道）
          const d = msg.data
          if (d?.repo !== repo) return
          track(d.repo, 'ui.contractViolated.shown', { taskId: d.taskId })
          toast(`影响面合约：有任务越界改动 ${d.files?.length ?? 0} 个文件`, 'error', {
            label: '去处理',
            onClick: () => {
              track(d.repo, 'ui.contractViolated.click', { taskId: d.taskId })
              opts.onGoTasks()
            },
          }, true, `contract:${d.taskId}`)
          if (document.hidden) void sysNotify('EasyVibe · 影响面越界', `有任务越界 ${d.files?.length ?? 0} 个文件`)
        }
        if (msg.name === 'task.statusChanged') {
          const d = msg.data
          if (d?.repo !== repo) return
          emitTaskEvent({ repo: d.repo, taskId: d.taskId, status: d.status, gate: d.gate })
          // P0 对标缺口#3：审批零通知——manual 任务在计划关等批，用户不盯窗口就卡死。
          // 系统通知（惰性申请权限）+ 页内 toast 双通道；仅窗口隐藏时弹系统通知防打扰
          if (d?.status === 'awaiting_approval') {
            toast('有任务等待你的审批', 'info')
            if (document.hidden) void sysNotify('EasyVibe · 待审批', `有任务已到达审批关${d.gate ? `（${d.gate}）` : ''}`)
          }
          if (d?.status === 'failed') {
            toast('有任务执行失败', 'error')
            if (document.hidden) void sysNotify('EasyVibe · 任务失败', '有任务执行失败，回应用查看详情')
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
}
