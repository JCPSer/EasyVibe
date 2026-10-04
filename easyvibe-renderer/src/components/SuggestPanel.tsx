import { useRef, useState } from 'react'
import { Loader2, Wrench, RefreshCw, Lightbulb } from 'lucide-react'
import type { CodeMap } from '@/types/map'
import type { Suggestion } from '@/lib/taskContext'
import { buildSuggestionTask, type TaskDraft } from '@/lib/taskContext'

interface Props {
  backendRepo: string | null
  map: CodeMap
  onCreateTask: (d: TaskDraft) => void
}

const PRIORITY_STYLE: Record<string, string> = {
  critical: 'bg-red-100 text-red-700',
  high: 'bg-amber-100 text-amber-700',
  medium: 'bg-slate-100 dark:bg-slate-800 text-slate-500 dark:text-slate-400',
}

// 智能优化建议：AI 主动发现优化机会（Stub=确定性派生 / LLM=地图注入），逐条可转修复任务
export function SuggestPanel({ backendRepo, map, onCreateTask }: Props) {
  const [items, setItems] = useState<Suggestion[] | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  // 试用反馈#2：可打断——AbortController；后端 LLM 调用仍跑完（成本已发生），前端不再等待
  const abortRef = useRef<AbortController | null>(null)

  const stop = () => {
    abortRef.current?.abort()
    abortRef.current = null
    setLoading(false)
  }

  const load = () => {
    if (!backendRepo || loading) return
    abortRef.current?.abort()
    const ac = new AbortController()
    abortRef.current = ac
    setLoading(true)
    setError(null)
    fetch(`/api/repos/${backendRepo}/suggest`, { method: 'POST', signal: ac.signal })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.json()
      })
      .then((d: { data: Suggestion[] }) => setItems(d.data))
      .catch((e) => {
        if (!ac.signal.aborted) setError(String(e))
      })
      .finally(() => {
        if (abortRef.current === ac) {
          abortRef.current = null
          setLoading(false)
        }
      })
  }

  // M4-1 真人测试 Bug#5：打开抽屉即自动烧 token 分析——改为用户显式点击开始
  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="flex items-center gap-1.5 text-[15px] font-bold text-slate-800 dark:text-slate-100">
            <Lightbulb size={15} className="text-amber-500" /> 智能优化建议
          </h2>
          <p className="mt-0.5 text-cap text-slate-400 dark:text-slate-500">AI 主动发现的优化机会，逐条可发起修复任务</p>
        </div>
        <div className="flex items-center gap-1.5">
          {loading && (
            <button
              onClick={stop}
              className="flex items-center gap-1 rounded-full border border-red-200 dark:border-red-900/60 px-2.5 py-1 text-cap font-semibold text-red-600 hover:bg-red-50 dark:hover:bg-red-950/40"
              title="停止等待（后端 LLM 调用已发出，成本已发生；不再等待结果）"
            >
              停止
            </button>
          )}
          {items !== null && (
            <button
              onClick={load}
              disabled={loading || !backendRepo}
              className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 px-2.5 py-1 text-cap font-semibold text-slate-500 dark:text-slate-400 hover:bg-slate-50 dark:hover:bg-slate-800/70 disabled:opacity-40"
            >
              <RefreshCw size={10} className={loading ? 'animate-spin' : ''} /> {loading ? '分析中' : '刷新'}
            </button>
          )}
        </div>
      </div>

      {!backendRepo && <p className="py-6 text-center text-[12px] text-slate-400 dark:text-slate-500">需要本地后端在线</p>}
      {error && <p className="py-6 text-center text-[12px] text-red-500">建议生成失败：{error}</p>}
      {/* M4-1 Bug#5：显式开始——分析消耗 LLM token，用户点头才开始 */}
      {backendRepo && items === null && !loading && !error && (
        <div className="flex flex-col items-center gap-2 py-10">
          <p className="text-[12px] text-slate-400 dark:text-slate-500">分析将调用 LLM 扫描全图，找出优化机会（约 1 分钟，有 token 成本）。</p>
          <button
            onClick={load}
            className="flex items-center gap-1 rounded-lg bg-blue-600 px-4 py-2 text-[12px] font-semibold text-white hover:bg-blue-700"
          >
            <Lightbulb size={12} /> 开始分析
          </button>
        </div>
      )}
      {loading && items === null && (
        <div className="flex items-center justify-center gap-2 py-10 text-[12px] text-slate-400 dark:text-slate-500">
          <Loader2 size={14} className="animate-spin" /> 正在分析地图…（结果会保留，切页签不会重分析）
        </div>
      )}
      {items && !loading && items.length === 0 && <p className="py-6 text-center text-[12px] text-slate-400 dark:text-slate-500">当前地图没有可建议的优化项，保持得很好。</p>}

      {items?.map((sg, i) => (
        <div key={i} className="rounded-lg border border-slate-200 dark:border-slate-700 p-3">
          <div className="flex items-start gap-2">
            <span className={`mt-0.5 rounded-full px-1.5 py-px text-micro font-bold ${PRIORITY_STYLE[sg.priority] ?? PRIORITY_STYLE.medium}`}>
              {sg.priority}
            </span>
            <div className="min-w-0 flex-1">
              <div className="text-[12px] font-semibold leading-5 text-slate-800 dark:text-slate-100">{sg.title}</div>
              <p className="mt-1 whitespace-pre-wrap text-[11px] leading-5 text-slate-600 dark:text-slate-300">{sg.description}</p>
              <div className="mt-1.5 flex flex-wrap items-center gap-1.5 text-micro text-slate-400 dark:text-slate-500">
                <span>{sg.rationale}</span>
                {sg.modules.map((id) => {
                  const m = map.modules.find((x) => x.id === id)
                  return m ? (
                    <span key={id} className="rounded-full bg-slate-100 dark:bg-slate-800 px-1.5 py-px font-mono text-slate-500 dark:text-slate-400">{m.name}</span>
                  ) : null
                })}
                <button
                  onClick={() => onCreateTask(buildSuggestionTask(map, sg))}
                  className="ml-auto flex items-center gap-1 rounded-full bg-blue-600 px-2.5 py-1 text-micro font-bold text-white hover:bg-blue-700"
                >
                  <Wrench size={9} /> 发起修复
                </button>
              </div>
            </div>
          </div>
        </div>
      ))}
    </div>
  )
}
