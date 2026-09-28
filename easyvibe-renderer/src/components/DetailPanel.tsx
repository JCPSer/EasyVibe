import { X, FileCode2, KeyRound, Flag, StickyNote, ArrowDownToLine, ArrowUpFromLine, Boxes, Info } from 'lucide-react'
import type { CodeMap, Layer, Module } from '@/types/map'
import { healthColor, healthLabel, dependentsOf } from '@/lib/layout'
import { Badge } from '@/components/ui/badge'
import { Separator } from '@/components/ui/separator'
import { IssuesList } from '@/components/IssuesList'

export type Selection = { kind: 'module' | 'layer'; id: string } | null
export type PanelTab = 'issues' | 'detail'

interface Props {
  map: CodeMap
  selection: Selection
  tab: PanelTab
  onTabChange: (tab: PanelTab) => void
  onLocateModule: (moduleId: string) => void
  onClose: () => void
}

function Row({ icon, label, children }: { icon: React.ReactNode; label: string; children: React.ReactNode }) {
  return (
    <div>
      <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-slate-400">
        {icon}
        {label}
      </div>
      {children}
    </div>
  )
}

function ModuleView({ map, mod }: { map: CodeMap; mod: Module }) {
  const color = healthColor(mod.health.score)
  const deps = mod.dependencies.map((id) => map.modules.find((m) => m.id === id)).filter(Boolean) as Module[]
  const dependents = dependentsOf(map, mod)
    .map((id) => map.modules.find((m) => m.id === id))
    .filter(Boolean) as Module[]

  return (
    <>
      <div>
        <div className="flex items-center gap-2">
          <h2 className="text-[15px] font-bold text-slate-800">{mod.name}</h2>
          <Badge variant="outline" className="border-slate-200 text-slate-500">
            {mod.id}
          </Badge>
        </div>
        <p className="mt-1 text-[12px] leading-5 text-slate-500">{mod.responsibility}</p>
        <p className="mt-1 text-[11px] text-slate-400">
          所属层：{map.layers.find((l) => l.id === mod.layer)?.name ?? mod.layer}
        </p>
      </div>

      <div className="rounded-lg border p-3" style={{ borderColor: `${color}55`, background: `${color}0d` }}>
        <div className="flex items-center justify-between">
          <span className="text-[11px] font-semibold" style={{ color }}>
            {healthLabel(mod.health.score)} · {mod.health.score}/100
          </span>
          <span className="text-[10.5px] text-slate-400">
            coupling {mod.health.coupling} · complexity {mod.health.complexity} · churn {mod.health.churn}
          </span>
        </div>
        {mod.health.review_note && <p className="mt-2 text-[11.5px] leading-5 text-slate-600">{mod.health.review_note}</p>}
      </div>

      <Row icon={<KeyRound size={12} />} label="关键入口">
        <div className="space-y-1.5">
          {mod.key_entries.slice(0, 6).map((k) => (
            <div key={k.file + k.symbol} className="rounded-md bg-slate-50 px-2.5 py-1.5">
              <div className="font-mono text-[11px] font-medium text-slate-700">{k.symbol}</div>
              <div className="truncate font-mono text-[10px] text-slate-400">{k.file}</div>
            </div>
          ))}
          {mod.key_entries.length === 0 && <p className="text-[11px] text-slate-400">（无）</p>}
        </div>
      </Row>

      <Row icon={<FileCode2 size={12} />} label="文件归属">
        <div className="space-y-1">
          {mod.files.map((f) => (
            <div key={f} className="truncate font-mono text-[10.5px] text-slate-500">
              {f}
            </div>
          ))}
        </div>
      </Row>

      <Row icon={<ArrowDownToLine size={12} />} label={`依赖（${deps.length}）`}>
        <div className="flex flex-wrap gap-1.5">
          {deps.map((d) => (
            <Badge key={d.id} variant="secondary" className="bg-slate-100 text-slate-600">
              {d.name}
            </Badge>
          ))}
          {deps.length === 0 && <p className="text-[11px] text-slate-400">（无）</p>}
        </div>
      </Row>

      <Row icon={<ArrowUpFromLine size={12} />} label={`被依赖（${dependents.length}）`}>
        <div className="flex flex-wrap gap-1.5">
          {dependents.map((d) => (
            <Badge key={d.id} variant="secondary" className="bg-slate-100 text-slate-600">
              {d.name}
            </Badge>
          ))}
          {dependents.length === 0 && <p className="text-[11px] text-slate-400">（无）</p>}
        </div>
      </Row>

      {mod.health.decay_flags.length > 0 && (
        <Row icon={<Flag size={12} />} label="腐化标记">
          <div className="flex flex-wrap gap-1.5">
            {mod.health.decay_flags.map((f) => (
              <Badge key={f} className="border-red-200 bg-red-50 font-normal text-red-600">
                {f}
              </Badge>
            ))}
          </div>
        </Row>
      )}

      <Separator />

      <Row icon={<StickyNote size={12} />} label="说明">
        <p className="text-[11px] leading-5 text-slate-400">
          健康度为 LLM 巡检评估结果，仅供架构演进参考；关键入口与文件归属来自语义代码地图（{map.meta.generator}）。
        </p>
      </Row>
    </>
  )
}

function LayerView({ map, layer }: { map: CodeMap; layer: Layer }) {
  const mods = map.modules.filter((m) => m.layer === layer.id)
  const avg = Math.round(mods.reduce((s, m) => s + m.health.score, 0) / Math.max(mods.length, 1))
  const color = healthColor(avg)
  const modIds = new Set(mods.map((m) => m.id))
  const violations = map.edges.filter(
    (e) => e.direction_violation && (modIds.has(e.from) || modIds.has(e.to)),
  )
  const downViolations = violations.filter((e) => modIds.has(e.from) && !modIds.has(e.to)).length

  return (
    <>
      <div>
        <div className="flex items-center gap-2">
          <Boxes size={16} className="text-slate-500" />
          <h2 className="text-[15px] font-bold text-slate-800">{layer.name}</h2>
          <Badge variant="outline" className="border-slate-200 text-slate-500">
            {layer.id}
          </Badge>
        </div>
        <p className="mt-1 text-[12px] leading-5 text-slate-500">{layer.description}</p>
        <p className="mt-1 text-[11px] text-slate-400">层序：L{layer.order}（0 为最顶层 / 入口侧）</p>
      </div>

      <div className="rounded-lg border p-3" style={{ borderColor: `${color}55`, background: `${color}0d` }}>
        <div className="flex items-center justify-between">
          <span className="text-[11px] font-semibold" style={{ color }}>
            层健康（聚合） · {avg}/100
          </span>
          <span className="text-[10.5px] text-slate-400">
            {mods.length} 模块 · 逆向依赖 {violations.length} 条
          </span>
        </div>
        <p className="mt-2 flex items-start gap-1.5 text-[10.5px] leading-4 text-slate-500">
          <Info size={11} className="mt-0.5 shrink-0 text-slate-400" />
          层健康是成员模块分数的聚合参考，并非 LLM 独立评估；LLM 评估仅模块级与架构级两级。
        </p>
      </div>

      <Row icon={<Flag size={12} />} label={`层间逆向依赖（${violations.length}）`}>
        <div className="space-y-1.5">
          {violations.slice(0, 8).map((e, i) => (
            <div key={i} className="flex items-center gap-1.5 rounded-md bg-red-50 px-2 py-1.5 text-[10.5px] text-red-600">
              <span className="font-mono font-medium">{e.from}</span>
              <span className="text-red-400">→</span>
              <span className="font-mono font-medium">{e.to}</span>
              <span className="ml-auto truncate text-red-400">{e.label ?? e.type}</span>
            </div>
          ))}
          {violations.length === 0 && <p className="text-[11px] text-slate-400">（无，该层方向约束良好）</p>}
        </div>
      </Row>

      <Row icon={<Boxes size={12} />} label={`成员模块（${mods.length}）`}>
        <div className="space-y-1.5">
          {mods.map((m) => (
            <div key={m.id} className="flex items-center gap-2 rounded-md bg-slate-50 px-2.5 py-2">
              <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: healthColor(m.health.score) }} />
              <div className="min-w-0">
                <div className="text-[11.5px] font-medium text-slate-700">{m.name}</div>
                <div className="truncate text-[10px] text-slate-400">{m.responsibility}</div>
              </div>
              <span className="ml-auto text-[10.5px] font-semibold" style={{ color: healthColor(m.health.score) }}>
                {m.health.score}
              </span>
            </div>
          ))}
        </div>
      </Row>

      {downViolations > 0 && (
        <p className="text-[10.5px] leading-4 text-slate-400">
          其中 {downViolations} 条为指向更上层的逆向依赖（本层模块主动引用上层）。
        </p>
      )}
    </>
  )
}

export function DetailPanel({ map, selection, tab, onTabChange, onLocateModule, onClose }: Props) {
  const module = selection?.kind === 'module' ? map.modules.find((m) => m.id === selection.id) : undefined
  const layer = selection?.kind === 'layer' ? map.layers.find((l) => l.id === selection.id) : undefined

  return (
    <aside className="flex w-[340px] shrink-0 flex-col border-l border-slate-200 bg-white">
      <div className="flex items-center justify-between border-b border-slate-100 px-3 py-2">
        <div className="flex gap-1">
          {(
            [
              ['issues', '问题清单'],
              ['detail', module ? '模块详情' : layer ? '架构层详情' : '选中详情'],
            ] as const
          ).map(([key, label]) => (
            <button
              key={key}
              onClick={() => onTabChange(key)}
              className={`rounded-md px-2.5 py-1.5 text-[12px] font-semibold transition-colors ${
                tab === key ? 'bg-blue-50 text-blue-600' : 'text-slate-400 hover:text-slate-600'
              }`}
            >
              {label}
            </button>
          ))}
        </div>
        <button onClick={onClose} className="rounded p-1 text-slate-400 hover:bg-slate-100 hover:text-slate-600">
          <X size={16} />
        </button>
      </div>
      <div className="flex-1 space-y-5 overflow-y-auto p-4">
        {tab === 'issues' && <IssuesList map={map} onLocate={(id) => onLocateModule(id)} />}
        {tab === 'detail' && module && <ModuleView map={map} mod={module} />}
        {tab === 'detail' && !module && layer && <LayerView map={map} layer={layer} />}
        {tab === 'detail' && !module && !layer && (
          <p className="pt-8 text-center text-[11.5px] leading-5 text-slate-400">
            点击画布中的模块卡片或层标签查看详情；
            <br />
            切换到「问题清单」查看全库最需要关注的问题。
          </p>
        )}
      </div>
    </aside>
  )
}
