import { memo } from 'react'
import { Handle, Position, type NodeProps, type Node } from '@xyflow/react'
import { Loader2, FileCode2 } from 'lucide-react'
import type { SubModule } from '@/types/map'
import { healthColor, healthLabel, SUB_W, SUB_H } from '@/lib/layout'

// 腐化标记中文标签（与主图 ModuleNode 同表 + 子图内部专属）
const SUB_FLAG_LABEL: Record<string, string> = {
  god_object: '上帝对象', internal_circular_dep: '内部循环依赖', circular_dep: '循环依赖',
  coupling_high: '耦合过高', layer_violation: '分层违规', single_file_module: '单文件模块',
  file_too_large: '文件过大', part_file_coupling: '部分文件耦合', duplicated_protocol: '协议重复',
  ref_plumbing: '引用缠绕', doc_drift: '文档漂移',
}

export type SubmoduleNodeType = Node<
  { sub?: SubModule; loading: boolean; inCount: number; outCount: number; parentName: string },
  'submodule'
>

const MAX_HANDLES = 10

// 子模块卡片（模块内部连线图的节点）；加载中显示骨架
export const SubmoduleNode = memo(function SubmoduleNode({ data, selected }: NodeProps<SubmoduleNodeType>) {
  const nIn = Math.min(Math.max(data.inCount, 1), MAX_HANDLES)
  const nOut = Math.min(Math.max(data.outCount, 1), MAX_HANDLES)

  if (data.loading || !data.sub) {
    return (
      <div
        className="flex h-full w-full items-center justify-center gap-2 rounded-lg border border-dashed border-slate-300 bg-slate-50 text-[11px] text-slate-400"
        style={{ width: SUB_W, height: SUB_H }}
      >
        <Loader2 size={13} className="animate-spin" /> 分析中…
      </div>
    )
  }

  const sub = data.sub
  const color = healthColor(sub.health.score)

  return (
    <div
      className="rounded-lg border bg-white shadow-sm"
      style={{
        width: SUB_W,
        height: SUB_H,
        borderColor: selected ? '#2563eb' : '#e2e8f0',
        boxShadow: selected ? '0 0 0 2px rgba(37,99,235,.25)' : undefined,
        padding: '10px 12px',
      }}
    >
      {Array.from({ length: nIn }, (_, i) => (
        <Handle key={`t${i}`} id={`t${i}`} type="target" position={Position.Top}
          style={{ left: `${((i + 1) * 100) / (nIn + 1)}%`, opacity: 0 }} />
      ))}
      {Array.from({ length: nOut }, (_, i) => (
        <Handle key={`s${i}`} id={`s${i}`} type="source" position={Position.Bottom}
          style={{ left: `${((i + 1) * 100) / (nOut + 1)}%`, opacity: 0 }} />
      ))}

      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="truncate text-[12px] font-semibold leading-4 text-slate-800">{sub.name}</div>
          <div className="mt-0.5 line-clamp-2 text-[10px] leading-3.5 text-slate-500">{sub.responsibility}</div>
        </div>
        <span
          className="shrink-0 rounded-full px-1.5 py-0.5 text-[9px] font-bold"
          style={{ background: `${color}18`, color }}
          title={healthLabel(sub.health.score)}
        >
          {sub.health.score}
        </span>
      </div>

      <div className="mt-1.5 flex items-center gap-2 text-[9.5px] text-slate-400">
        <span className="flex items-center gap-0.5">
          <FileCode2 size={9} /> {sub.files.length} 文件
        </span>
        {sub.health.decay_flags.length > 0 && (
          <span className="truncate text-red-500">{sub.health.decay_flags.map((f) => SUB_FLAG_LABEL[f] ?? f).join(' · ')}</span>
        )}
      </div>
    </div>
  )
})
