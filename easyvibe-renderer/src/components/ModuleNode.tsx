import { memo } from 'react'
import { Handle, Position, type NodeProps, type Node } from '@xyflow/react'
import { FileCode2 } from 'lucide-react'
import type { Module } from '@/types/map'
import { healthColor, healthLabel, NODE_W, NODE_H } from '@/lib/layout'
// M4-1 真人测试 Bug#4：decay_flags 是内部英文 id，直接渲染用户看不懂——统一中文标签
const FLAG_LABEL: Record<string, string> = {
  god_module: '上帝模块',
  god_object: '上帝对象',
  coupling_high: '耦合过高',
  circular_dep: '循环依赖',
  internal_circular_dep: '内部循环依赖',
  layer_violation: '分层违规',
  layering_mismatch: '分层错位',
  responsibility_overlap: '职责重叠',
  duplicated_protocol: '协议重复',
  ref_plumbing: '引用缠绕',
  closure_staleness_workaround: '过期兼容',
  doc_drift: '文档漂移',
}


export type ModuleNodeType = Node<{ module: Module; inCount: number; outCount: number }, 'module'>

const MAX_HANDLES = 18

// 沿节点顶/底边均匀分布多个连接点，避免大量连线汇聚到中心一点
function spreadHandles(count: number, pos: Position, prefix: string) {
  const n = Math.min(Math.max(count, 1), MAX_HANDLES)
  return Array.from({ length: n }, (_, i) => (
    <Handle
      key={`${prefix}${i}`}
      id={`${prefix}${i}`}
      type={pos === Position.Top ? 'target' : 'source'}
      position={pos}
      style={{ left: `${((i + 1) * 100) / (n + 1)}%`, opacity: 0 }}
    />
  ))
}

// 模块卡片：文件图标 + 名称 + 职责 + 右上角健康状态环（参照主界面示意图）
export const ModuleNode = memo(function ModuleNode({ data, selected }: NodeProps<ModuleNodeType>) {
  const mod = data.module
  const color = healthColor(mod.health.score)
  const R = 8
  const C = 2 * Math.PI * R

  return (
    <div
      className="lift rounded-xl border bg-white dark:bg-slate-900 shadow-sm transition-shadow"
      style={{
        width: NODE_W,
        height: NODE_H,
        borderColor: selected ? '#2563eb' : '#e2e8f0',
        boxShadow: selected ? '0 0 0 2px rgba(37,99,235,.25)' : undefined,
        padding: '12px 14px',
      }}
    >
      {spreadHandles(data.inCount, Position.Top, 't')}
      {spreadHandles(data.outCount, Position.Bottom, 's')}

      <div className="flex items-start gap-2.5">
        <FileCode2 size={18} className="mt-0.5 shrink-0 text-slate-400 dark:text-slate-500" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13px] font-semibold leading-5 text-slate-800 dark:text-slate-100">{mod.name}</div>
          <div className="mt-0.5 line-clamp-2 text-[11px] leading-4 text-slate-500 dark:text-slate-400">{mod.responsibility}</div>
        </div>
        {/* 健康状态环 */}
        <div className="relative shrink-0" title={`${healthLabel(mod.health.score)} · ${mod.health.score}`}>
          <svg width="24" height="24" viewBox="0 0 24 24">
            <circle cx="12" cy="12" r={R} fill="none" stroke="#e2e8f0" strokeWidth="3" />
            <circle
              cx="12"
              cy="12"
              r={R}
              fill="none"
              stroke={color}
              strokeWidth="3"
              strokeLinecap="round"
              strokeDasharray={C}
              strokeDashoffset={C * (1 - mod.health.score / 100)}
              transform="rotate(-90 12 12)"
            />
          </svg>
          <span className="absolute inset-0 flex items-center justify-center text-[8px] font-bold" style={{ color }}>
            {mod.health.score}
          </span>
        </div>
      </div>


      {mod.health.decay_flags.length > 0 && (
        <div className="mt-1.5 flex flex-wrap gap-1">
          {mod.health.decay_flags.slice(0, 3).map((f) => (
            <span
              key={f}
              className="rounded-full border border-red-200 bg-red-50 dark:bg-red-950/40 px-1.5 py-px text-micro leading-4 text-red-600"
            >
              {FLAG_LABEL[f] ?? f}
            </span>
          ))}
        </div>
      )}
    </div>
  )
})
