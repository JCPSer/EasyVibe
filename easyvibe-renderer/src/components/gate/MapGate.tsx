import { useEffect, useState } from 'react'
import { Loader2, Play, RotateCcw } from 'lucide-react'
import { toast } from '@/runtime/toast'
import { InductionWaiting } from './InductionWaiting'
import { mapProgress, reinduce } from '@/api/canvas'

// P0 审查前端#1：地图加载守门员——区分三种真实状态，消灭"出错也转圈"死锁：
// · progress 显示归纳进行中 → 等待页（原行为）
// · 加载出错（5xx/网络/后端离线） → 错误卡（重试 / 开始归纳）
// · 从未生成且无归纳（progress 无文件） → 引导卡（开始归纳）
export function MapGate({ repo, error, onRetry, agentReady }: { repo: string; error: string | null; onRetry: () => void; agentReady: boolean }) {
  const [inducing, setInducing] = useState<boolean | null>(null) // null=探测中
  const [starting, setStarting] = useState(false)

  useEffect(() => {
    let stale = false
    const tick = () => {
      mapProgress(repo)
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
      const r = await reinduce(repo)
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
