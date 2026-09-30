import { useEffect, useState } from 'react'
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
  medium: 'bg-slate-100 text-slate-500',
}

// 智能优化建议：AI 主动发现优化机会（Stub=确定性派生 / LLM=地图注入），逐条可转修复任务
export function SuggestPanel({ backendRepo, map, onCreateTask }: Props) {
  const [items, setItems] = useState<Suggestion[] | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const load = () => {
    if (!backendRepo) return
    setLoading(true)
    setError(null)
    fetch(`/api/repos/${backendRepo}/suggest`, { method: 'POST' })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        return r.json()
      })
      .then((d: { data: Suggestion[] }) => setItems(d.data))
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false))
  }

  useEffect(() => {
    load()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [backendRepo])

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="flex items-center gap-1.5 text-[15px] font-bold text-slate-800">
            <Lightbulb size={15} className="text-amber-500" /> 智能优化建议
          </h2>
          <p className="mt-0.5 text-[10.5px] text-slate-400">AI 主动发现的优化机会，逐条可发起修复任务</p>
        </div>
        <button
          onClick={load}
          disabled={loading || !backendRepo}
          className="flex items-center gap-1 rounded-full border border-slate-200 px-2.5 py-1 text-[10.5px] font-semibold text-slate-500 hover:bg-slate-50 disabled:opacity-40"
        >
          <RefreshCw size={10} className={loading ? 'animate-spin' : ''} /> 刷新
        </button>
      </div>

      {!backendRepo && <p className="py-6 text-center text-[11.5px] text-slate-400">需要本地后端在线</p>}
      {error && <p className="py-6 text-center text-[11.5px] text-red-500">建议生成失败：{error}</p>}
      {loading && (
        <div className="flex items-center justify-center gap-2 py-10 text-[12px] text-slate-400">
          <Loader2 size={14} className="animate-spin" /> 正在分析地图…
        </div>
      )}
      {items && !loading && items.length === 0 && <p className="py-6 text-center text-[11.5px] text-slate-400">当前地图没有可建议的优化项，保持得很好。</p>}

      {items?.map((sg, i) => (
        <div key={i} className="rounded-lg border border-slate-200 p-3">
          <div className="flex items-start gap-2">
            <span className={`mt-0.5 rounded-full px-1.5 py-px text-[9px] font-bold ${PRIORITY_STYLE[sg.priority] ?? PRIORITY_STYLE.medium}`}>
              {sg.priority}
            </span>
            <div className="min-w-0 flex-1">
              <div className="text-[12px] font-semibold leading-5 text-slate-800">{sg.title}</div>
              <p className="mt-1 whitespace-pre-wrap text-[11px] leading-5 text-slate-600">{sg.description}</p>
              <div className="mt-1.5 flex flex-wrap items-center gap-1.5 text-[10px] text-slate-400">
                <span>{sg.rationale}</span>
                {sg.modules.map((id) => {
                  const m = map.modules.find((x) => x.id === id)
                  return m ? (
                    <span key={id} className="rounded-full bg-slate-100 px-1.5 py-px font-mono text-slate-500">{m.name}</span>
                  ) : null
                })}
                <button
                  onClick={() => onCreateTask(buildSuggestionTask(map, sg))}
                  className="ml-auto flex items-center gap-1 rounded-full bg-blue-600 px-2.5 py-1 text-[10px] font-bold text-white hover:bg-blue-700"
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
