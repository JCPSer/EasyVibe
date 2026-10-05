import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { CodeMap } from '@/types/map'
import { toast } from '@/lib/toast'
import { enqueue } from '@/lib/sessionQueue'
import { isValidGrowthEvent, mergeGrowthEvents, parseGrowthText } from '@/lib/growthMerge'
import { onGrowthEvent, onPatrolFinished, onQueueChanged, onSessionEvent, setWsCloseListener } from '@/lib/growthBus'
import type { GrowthState } from './types'

/**
 * 生长回放域：growth.log 消费、WS 直播追加、500ms 推进定时器、重归纳直播、断线退出。
 * 从 Canvas 抽出，行为与依赖数组逐处保持；selection/expandedIds 的重置经 onPlaybackStart 回调外置。
 */
export function useGrowthPlayback(
  map: CodeMap,
  backendRepo: string | null,
  opts: { onPatrollingChange: (v: boolean) => void; onPlaybackStart: () => void },
) {
  const { onPatrollingChange, onPlaybackStart } = opts
  const [growth, setGrowth] = useState<GrowthState | null>(null)
  const [liveActivity, setLiveActivity] = useState(false)
  const [inducing, setInducing] = useState(false)
  const growthRef = useRef<GrowthState | null>(null)
  // 直播会话标记：终态时诚实收尾（空事件 → 收起，不装播完）
  const growthFromReinduce = useRef(false)

  useEffect(() => {
    growthRef.current = growth
  }, [growth])

  // 切仓库清掉上一仓库的生长态（原 Canvas 重置 effect 的子集）
  useEffect(() => {
    setGrowth(null)
  }, [backendRepo])

  // R2：WS 断线 → 退出生长模式（重连后由用户重新进入，startGrowth 拉全量对齐）
  useEffect(() => setWsCloseListener(() => setGrowth(null)), [])

  // 会话终态（succeeded/failed）解除"归纳中"/"巡检中"（patrol 会话以 patrol- 前缀区分）
  useEffect(
    () =>
      onSessionEvent((evt) => {
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

  // 归纳完成：短暂展示后自动退出回放条。live 生长条的唯一来源是归纳/排队接续，
  // 停在"归纳完成 xx%"等用户手动关是死胡同（2026-10-05 实弹）；新地图经 map.changed 自动刷新
  useEffect(() => {
    if (!growth?.done) return
    const t = setTimeout(() => setGrowth(null), 1600)
    return () => clearTimeout(t)
  }, [growth?.done])

  const startGrowth = useCallback(() => {
    const url = backendRepo ? `/api/repos/${backendRepo}/growth` : '/data/growth.log'
    fetch(url)
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.text()
      })
      .then((text) => {
        const events = parseGrowthText(text, !!backendRepo) // 坏行/坏消息跳过（Y1/R1）
        onPlaybackStart()
        setLiveActivity(false)
        setGrowth({ events, index: 0, playing: true, done: false })
      })
      .catch(() => setGrowth(null))
  }, [backendRepo, onPlaybackStart])

  // 写路径（M2-3）：触发重新归纳 → 后端 spawn agent 按 v2.2 执行；自动进入直播模式看生长
  const startReinduce = useCallback(() => {
    if (!backendRepo || inducing) return
    // R7 清债：重新归纳 = spawn 全仓库 agent（数分钟 + LLM 成本），先确认
    if (!window.confirm('重新归纳将 spawn agent 全量分析仓库（通常数分钟），期间地图数据会被刷新。继续？')) return
    setInducing(true)
    fetch(`/api/repos/${backendRepo}/reinduce`, { method: 'POST' })
      .then((r) => {
        // S2：必须抛 Response 本体——catch 里要读 status/body 判别 409（与 analyzeSubmap 同因）
        if (!r.ok) throw r
        // 2026-10-04 实弹修复：改为开空直播会话，WS 的 growth.event 随 agent 产出实时追加，
        // 直到 done 事件真正到达（此前回放 growth.log 旧快照，误以为归纳结束）
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

  // I3：排队任务被排空——reinduce=空直播会话+归纳中；patrol=巡检中（真实解除仍靠 patrol.finished）
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
        }
      }),
    [backendRepo, onPatrollingChange],
  )

  // 直播合并：生长事件的模块/层/边合入基准地图——生长事件本身是"进行中模块"的事实源
  const mergedMap = useMemo(
    () => (growth ? mergeGrowthEvents(map, growth.events) : map),
    [map, growth?.events],
  )

  // 回放控制条动作（GrowthPanel）
  const pauseGrowth = useCallback(() => setGrowth((g) => (g ? { ...g, playing: !g.playing } : g)), [])
  const restartGrowth = useCallback(() => setGrowth((g) => (g ? { ...g, index: 0, playing: true, done: false } : g)), [])
  const exitGrowth = useCallback(() => setGrowth(null), [])

  return { growth, liveActivity, inducing, arrived, mergedMap, startGrowth, startReinduce, pauseGrowth, restartGrowth, exitGrowth }
}
