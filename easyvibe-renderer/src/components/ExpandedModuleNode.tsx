import { memo } from 'react'
import { Handle, Position, type NodeProps, type Node } from '@xyflow/react'
import { ChevronDown, Loader2, RotateCcw, UnfoldVertical, AlertTriangle, Search} from 'lucide-react'
import type { Module } from '@/types/map'
import { healthColor } from '@/lib/layout'

export type ExpandedModuleNodeType = Node<
  {
    module: Module
    inCount: number
    outCount: number
    loading: boolean
    error: boolean
    subCount: number
    onRetry?: () => void
    onAnalyze?: () => void
    /** 改进#2：agent 过程直播——最近输出 */
    agentLines?: string[]
    /** M4-1 诚实三态：失败原因（启动失败/会话失败/超时），必须说人话 */
    analyzeError?: string
  },
  'moduleExpanded'
>

const MAX_HANDLES = 18

// 展开后的模块容器：头部 + 子模块占位区；模块级连线仍连到容器的顶/底 handle
export const ExpandedModuleNode = memo(function ExpandedModuleNode({ data }: NodeProps<ExpandedModuleNodeType>) {
  const mod = data.module
  const color = healthColor(mod.health.score)

  const nIn = Math.min(Math.max(data.inCount, 1), MAX_HANDLES)
  const nOut = Math.min(Math.max(data.outCount, 1), MAX_HANDLES)

  return (
    <div
      className="h-full w-full rounded-xl border-2 border-dashed bg-white/60"
      style={{ borderColor: `${color}88` }}
    >
      {/* 模块级连线 handle：顶/底均匀分布 */}
      {Array.from({ length: nIn }, (_, i) => (
        <Handle
          key={`t${i}`}
          id={`t${i}`}
          type="target"
          position={Position.Top}
          style={{ left: `${((i + 1) * 100) / (nIn + 1)}%`, opacity: 0 }}
        />
      ))}
      {Array.from({ length: nOut }, (_, i) => (
        <Handle
          key={`s${i}`}
          id={`s${i}`}
          type="source"
          position={Position.Bottom}
          style={{ left: `${((i + 1) * 100) / (nOut + 1)}%`, opacity: 0 }}
        />
      ))}

      {/* 头部：模块名 + 展开中状态 */}
      <div className="flex h-12 items-center gap-2 border-b border-slate-200/70 px-4">
        <UnfoldVertical size={14} className="text-blue-500" />
        <span className="text-[13px] font-bold text-slate-800 dark:text-slate-100">{mod.name}</span>
        <span className="text-micro text-slate-400 dark:text-slate-500">内部结构</span>
        <span className="ml-1 h-2 w-2 rounded-full" style={{ background: color }} />
        <span className="text-cap font-semibold" style={{ color }}>
          {mod.health.score}
        </span>
        {data.loading ? (
          <span className="ml-auto flex items-center gap-1.5 text-cap text-slate-400 dark:text-slate-500">
            <Loader2 size={12} className="animate-spin" /> 归纳子模块中…（约 1-3 分钟）
          </span>
        ) : data.error ? (
          <span className="ml-auto flex flex-col items-end gap-1">
            <span className="flex items-center gap-1.5 text-cap text-red-500">
              <AlertTriangle size={11} />
              {data.onAnalyze ? '暂无内部结构分析' : '子图加载失败'}
              {data.onAnalyze && (
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    data.onAnalyze!()
                  }}
                  className="flex items-center gap-0.5 rounded-full border border-blue-200 bg-blue-50 dark:bg-blue-950/40 px-1.5 py-0.5 font-semibold text-blue-600 hover:bg-blue-100"
                  title="派 agent 深入扫描该模块的文件，生成内部结构子图（约 1-3 分钟）"
                >
                  <Search size={9} /> 深入分析
                </button>
              )}
              {data.onRetry && (
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    data.onRetry!()
                  }}
                  className="flex items-center gap-0.5 rounded-full border border-red-200 bg-white dark:bg-slate-900 px-1.5 py-0.5 font-semibold hover:bg-red-50"
                  title="重新加载子图（若从未生成过，请用「深入分析」）"
                >
                  <RotateCcw size={9} /> 重试
                </button>
              )}
            </span>
            {/* M4-1 诚实三态：失败原因必须显式呈现，不允许只给"失败"两个字 */}
            {data.analyzeError && (
              <span className="max-w-[420px] text-right text-micro leading-4 text-red-400">{data.analyzeError}</span>
            )}
          </span>
        ) : (
          <span className="ml-auto flex items-center gap-1 text-cap text-slate-400 dark:text-slate-500">
            <ChevronDown size={11} /> {data.subCount} 个子模块 · 点工具栏可收起
          </span>
        )}
      </div>
      {/* 改进#2：agent 过程直播——分析中显示它正在输出的内容（来自会话 stdout 流） */}
      {data.loading && data.agentLines && data.agentLines.length > 0 && (
        <div className="mt-1.5 space-y-0.5 border-t border-slate-100 dark:border-slate-800 pt-1.5">
          {data.agentLines.slice(-3).map((l, i) => (
            <p key={i} className="truncate font-mono text-micro leading-3.5 text-slate-400 dark:text-slate-500">
              <span className="text-emerald-500">›</span> {l}
            </p>
          ))}
        </div>
      )}
    </div>
  )
})
