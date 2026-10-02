import { memo } from 'react'
import type { NodeProps, Node } from '@xyflow/react'
import { Boxes, ChevronRight } from 'lucide-react'
import type { Layer } from '@/types/map'
import type { BandBox } from '@/lib/layout'
import { healthColor } from '@/lib/layout'

export interface LayerStats {
  count: number
  avgScore: number
  violations: number
}

export type BandNodeType = Node<
  { layer: Layer; box: BandBox; index: number; stats: LayerStats; selected: boolean; onSelect: (layerId: string) => void },
  'band'
>

// 层横带：左侧层标签列（可点击选中该层）+ 右侧带状区域。
// 注意：层健康（stats）为成员模块的聚合指标，LLM 独立评估只有模块级与架构级两级。
export const BandNode = memo(function BandNode({ data }: NodeProps<BandNodeType>) {
  const { layer, box, index, stats, selected, onSelect } = data
  const color = healthColor(stats.avgScore)

  return (
    <div className="flex" style={{ width: box.width, height: box.height, opacity: 0.98, pointerEvents: 'none' }}>
      {/* 层标签列：可交互 */}
      <button
        onClick={() => onSelect(layer.id)}
        className="flex h-full flex-col rounded-lg border bg-white/95 text-left shadow-sm transition-colors hover:border-blue-300"
        style={{
          width: 190,
          marginTop: 10,
          marginBottom: 10,
          marginLeft: 16,
          padding: '16px 12px 12px 14px',
          borderColor: selected ? '#2563eb' : '#e2e8f0',
          boxShadow: selected ? '0 0 0 2px rgba(37,99,235,.25)' : undefined,
          cursor: 'pointer',
          pointerEvents: 'auto',
        }}
        title="点击选中该层"
      >
        {/* 改进#5：层序徽标 L0/L1…——分层"可数可辨"，远看知道几层、谁在上谁在下 */}
        <div className="flex items-center gap-1.5 text-slate-700">
          <span
            className="rounded px-1 py-px font-mono text-micro font-bold"
            style={{ background: `${color}22`, color }}
          >
            L{index}
          </span>
          <Boxes size={15} strokeWidth={1.8} />
          <span className="text-[13px] font-bold tracking-wide">{layer.name}</span>
          <ChevronRight size={13} className="ml-auto text-slate-300" />
        </div>
        {/* 层健康色条：整层体温一眼可见 */}
        <div className="mt-1.5 h-1 w-full overflow-hidden rounded-full bg-slate-100">
          <div className="h-full rounded-full" style={{ width: `${stats.avgScore}%`, background: color }} />
        </div>
        <div className="mt-1 text-micro leading-4 text-slate-400">{layer.description}</div>

        {/* 层聚合健康 */}
        <div className="mt-auto flex items-center gap-1.5 pt-2">
          <span className="h-2 w-2 rounded-full" style={{ background: color }} />
          <span className="text-micro font-semibold" style={{ color }}>
            {stats.avgScore}
          </span>
          <span className="text-micro text-slate-400">
            · {stats.count} 模块{stats.violations > 0 && ` · ${stats.violations} 逆向`}
          </span>
        </div>
      </button>

      {/* 横带主体：交替底色 + 实线边界，让分层一目了然 */}
      <div
        className="h-full flex-1 rounded-lg border"
        style={{
          margin: '10px 16px 10px 10px',
          pointerEvents: 'none',
          background: index % 2 === 0 ? 'rgba(226,232,240,0.5)' : 'rgba(241,245,249,0.6)',
          borderColor: selected ? '#93c5fd' : '#e2e8f0',
          borderLeftWidth: 3,
          borderLeftColor: selected ? '#2563eb' : `${color}99`,
        }}
      />
    </div>
  )
})
