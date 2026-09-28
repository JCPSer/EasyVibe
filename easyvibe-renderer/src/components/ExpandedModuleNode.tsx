import { memo } from 'react'
import { Handle, Position, type NodeProps, type Node } from '@xyflow/react'
import { ChevronDown, Loader2, UnfoldVertical } from 'lucide-react'
import type { Module } from '@/types/map'
import { healthColor } from '@/lib/layout'

export type ExpandedModuleNodeType = Node<
  { module: Module; inCount: number; outCount: number; loading: boolean; subCount: number },
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
        <span className="text-[13px] font-bold text-slate-800">{mod.name}</span>
        <span className="text-[10px] text-slate-400">内部结构</span>
        <span className="ml-1 h-2 w-2 rounded-full" style={{ background: color }} />
        <span className="text-[10.5px] font-semibold" style={{ color }}>
          {mod.health.score}
        </span>
        {data.loading ? (
          <span className="ml-auto flex items-center gap-1.5 text-[10.5px] text-slate-400">
            <Loader2 size={12} className="animate-spin" /> 归纳子模块中…
          </span>
        ) : (
          <span className="ml-auto flex items-center gap-1 text-[10.5px] text-slate-400">
            <ChevronDown size={11} /> {data.subCount} 个子模块 · 点工具栏可收起
          </span>
        )}
      </div>
    </div>
  )
})
