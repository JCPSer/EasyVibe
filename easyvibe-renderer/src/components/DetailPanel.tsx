import { useEffect, useState } from 'react'
import { X, FileCode2, KeyRound, Flag, StickyNote, ArrowDownToLine, ArrowUpFromLine, Boxes, Info, Wrench, ListChecks, MessagesSquare} from 'lucide-react'
import { buildLayerTask, buildModuleTask, type TaskDraft } from '@/lib/taskContext'
import type { CodeMap, Layer, Module, SubMap, SubModule } from '@/types/map'
import { healthColor, healthLabel, dependentsOf } from '@/lib/layout'
import { Badge } from '@/components/ui/badge'
import { Separator } from '@/components/ui/separator'
import { IssuesList } from '@/components/IssuesList'
import { ChatPanel } from '@/components/ChatPanel'

export type Selection =
  | { kind: 'module' | 'layer'; id: string }
  | { kind: 'submodule'; parentId: string; subId: string }
  | null
export type PanelTab = 'detail' | 'issues' | 'chat'

interface Props {
  map: CodeMap
  selection: Selection
  tab: PanelTab
  onTabChange: (tab: PanelTab) => void
  submaps: Record<string, SubMap | 'loading' | 'error'>
  backendRepo: string | null
  onCreateTask: (draft: TaskDraft) => void
  onLocateModule: (moduleId: string) => void
  /** S1-1：打开视图（多模块时画布 solo 聚焦） */
  onOpenView?: (ids: string[]) => void
  onClose: () => void
  /** 改进#4：右栏可调宽（测试员 IA 反馈的非破坏性验证——宽度够不够先看数据） */
  width?: number
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

// S2：健康趋势条——module_health_history 自 M2-4 落库以来的第一个消费者。
// 分数序列（旧→新）条形图：让"这模块在变好还是腐烂"可见，复检闭环的可视化判据。
function HealthTrend({ backendRepo, moduleId }: { backendRepo: string; moduleId: string }) {
  const [rows, setRows] = useState<{ score: number; runId: string }[] | null>(null)
  useEffect(() => {
    let stale = false
    fetch(`/api/repos/${backendRepo}/modules/${encodeURIComponent(moduleId)}/health-history`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d: { data: { score: number; runId: string }[] }) => {
        if (!stale) setRows(d.data.slice().reverse()) // 旧→新
      })
      .catch(() => !stale && setRows([]))
    return () => {
      stale = true
    }
  }, [backendRepo, moduleId])

  if (rows === null) return null
  if (rows.length < 2) return null // 单点无趋势，不渲染（避免噪音）
  const latest = rows[rows.length - 1].score
  const prev = rows[rows.length - 2].score
  const delta = latest - prev
  return (
    <div className="mt-2 flex items-center gap-2">
      <span className="text-[9.5px] font-semibold text-slate-400">健康趋势</span>
      <div className="flex h-4 items-end gap-0.5">
        {rows.slice(-12).map((r, i) => (
          <div
            key={i}
            className="w-1.5 rounded-sm"
            style={{ height: `${Math.max(12, r.score)}%`, background: healthColor(r.score) }}
            title={`${r.score} 分`}
          />
        ))}
      </div>
      <span className={`text-[9.5px] font-bold ${delta >= 0 ? 'text-emerald-600' : 'text-red-500'}`}>
        {delta >= 0 ? '+' : ''}
        {delta}
      </span>
    </div>
  )
}

function ModuleView({ map, mod, onCreateTask, backendRepo }: { map: CodeMap; mod: Module; onCreateTask: (d: TaskDraft) => void; backendRepo: string | null }) {
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
        {/* M4-1 真人测试 Bug#3：原 justify-between 三元素挤一行，按钮压住指标文案——改两行布局 */}
        <div className="flex items-center justify-between">
          <span className="text-[11px] font-semibold" style={{ color }}>
            {healthLabel(mod.health.score)} · {mod.health.score}/100
          </span>
          <button
            onClick={() => onCreateTask(buildModuleTask(map, mod.id))}
            className="flex items-center gap-1 rounded-full bg-blue-600 px-2.5 py-1 text-[10px] font-bold text-white hover:bg-blue-700"
            title="指哪打哪：以本模块为上下文发起修复任务"
          >
            <Wrench size={10} /> 发起修复
          </button>
        </div>
        <p className="mt-1 text-[10.5px] text-slate-400">
          coupling {mod.health.coupling} · complexity {mod.health.complexity} · churn {mod.health.churn ?? 'n/a'}
        </p>
        {backendRepo && <HealthTrend backendRepo={backendRepo} moduleId={mod.id} />}
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

function LayerView({ map, layer, onCreateTask }: { map: CodeMap; layer: Layer; onCreateTask: (d: TaskDraft) => void }) {
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
        {/* M4-1.5 陪审团硬缺陷：三元素挤一行压字断行——两行布局 */}
        <div className="flex items-center justify-between">
          <span className="text-[11px] font-semibold" style={{ color }}>
            层健康（聚合） · <span className="tnum">{avg}</span>/100
          </span>
          <button
            onClick={() => onCreateTask(buildLayerTask(map, layer.id))}
            className="flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full bg-blue-600 px-2.5 py-1 text-[10px] font-bold text-white hover:bg-blue-700"
            title="指哪打哪：以本层为上下文发起治理任务"
          >
            <Wrench size={10} /> 发起治理
          </button>
        </div>
        <p className="mt-1 text-[10.5px] text-slate-400">
          {mods.length} 模块 · 逆向依赖 <span className="tnum">{violations.length}</span> 条
        </p>
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

// 子模块详情（§8 子图 drill-down 层）
function SubmoduleView({ parent, sub, submap }: { parent: Module; sub: SubModule; submap: SubMap }) {
  const color = healthColor(sub.health.score)
  const siblings = new Map(submap.sub_modules.map((s) => [s.id, s]))
  const deps = sub.dependencies.map((id) => siblings.get(id)).filter(Boolean) as SubModule[]
  const dependents = submap.edges.filter((e) => e.to === sub.id).map((e) => siblings.get(e.from)).filter(Boolean) as SubModule[]

  return (
    <>
      <div>
        <div className="flex items-center gap-2">
          <h2 className="text-[15px] font-bold text-slate-800">{sub.name}</h2>
          <Badge variant="outline" className="border-slate-200 text-slate-500">
            {sub.id}
          </Badge>
        </div>
        <p className="mt-1 text-[12px] leading-5 text-slate-500">{sub.responsibility}</p>
        <p className="mt-1 text-[11px] text-slate-400">
          所属模块：{parent.name}（内部结构，无层概念）
        </p>
      </div>

      <div className="rounded-lg border p-3" style={{ borderColor: `${color}55`, background: `${color}0d` }}>
        <div className="flex items-center justify-between">
          <span className="text-[11px] font-semibold" style={{ color }}>
            {healthLabel(sub.health.score)} · {sub.health.score}/100
          </span>
          <span className="text-[10.5px] text-slate-400">
            coupling {sub.health.coupling} · complexity {sub.health.complexity} · churn {sub.health.churn ?? 'n/a'}
          </span>
        </div>
        {sub.health.review_note && <p className="mt-2 text-[11.5px] leading-5 text-slate-600">{sub.health.review_note}</p>}
      </div>

      <Row icon={<KeyRound size={12} />} label="关键入口">
        <div className="space-y-1.5">
          {sub.key_entries.slice(0, 6).map((k) => (
            <div key={k.file + k.symbol} className="rounded-md bg-slate-50 px-2.5 py-1.5">
              <div className="font-mono text-[11px] font-medium text-slate-700">{k.symbol}</div>
              <div className="truncate font-mono text-[10px] text-slate-400">{k.file}</div>
            </div>
          ))}
        </div>
      </Row>

      <Row icon={<FileCode2 size={12} />} label="文件归属">
        <div className="space-y-1">
          {sub.files.map((f) => (
            <div key={f} className="truncate font-mono text-[10.5px] text-slate-500">{f}</div>
          ))}
        </div>
      </Row>

      <Row icon={<ArrowDownToLine size={12} />} label={`内部依赖（${deps.length}）`}>
        <div className="flex flex-wrap gap-1.5">
          {deps.map((d) => (
            <Badge key={d.id} variant="secondary" className="bg-slate-100 text-slate-600">{d.name}</Badge>
          ))}
          {deps.length === 0 && <p className="text-[11px] text-slate-400">（无）</p>}
        </div>
      </Row>

      <Row icon={<ArrowUpFromLine size={12} />} label={`被内部依赖（${dependents.length}）`}>
        <div className="flex flex-wrap gap-1.5">
          {dependents.map((d) => (
            <Badge key={d.id} variant="secondary" className="bg-slate-100 text-slate-600">{d.name}</Badge>
          ))}
          {dependents.length === 0 && <p className="text-[11px] text-slate-400">（无）</p>}
        </div>
      </Row>

      {sub.health.decay_flags.length > 0 && (
        <Row icon={<Flag size={12} />} label="腐化标记">
          <div className="flex flex-wrap gap-1.5">
            {sub.health.decay_flags.map((f) => (
              <Badge key={f} className="border-red-200 bg-red-50 font-normal text-red-600">{f}</Badge>
            ))}
          </div>
        </Row>
      )}

      <Separator />

      <Row icon={<Info size={12} />} label="说明">
        <p className="text-[11px] leading-5 text-slate-400">
          子模块健康为<b>展开时的派生评估</b>（父模块内部的实现质量），不回流父模块分、不写入主地图文件；
          LLM 独立评估仍只有模块级与架构级两级。生成者：{submap.generator}。
        </p>
      </Row>
    </>
  )
}

export function DetailPanel({ map, selection, tab, onTabChange, submaps, backendRepo, onCreateTask, onLocateModule, onClose, width }: Props) {
  const module = selection?.kind === 'module' ? map.modules.find((m) => m.id === selection.id) : undefined
  const layer = selection?.kind === 'layer' ? map.layers.find((l) => l.id === selection.id) : undefined
  const parent = selection?.kind === 'submodule' ? map.modules.find((m) => m.id === selection.parentId) : undefined
  const submap = selection?.kind === 'submodule' ? submaps[selection.parentId] : undefined
  const smLoaded = submap && submap !== 'loading' && submap !== 'error' ? submap : undefined
  const sub = selection?.kind === 'submodule' && smLoaded
    ? smLoaded.sub_modules.find((s) => s.id === selection.subId)
    : undefined

  const detailLabel = module ? '模块详情' : layer ? '架构层详情' : sub ? '子模块详情' : '选中详情'

  return (
    <aside className="anim-panel-in flex h-full shrink-0 flex-col border-l border-slate-200 bg-white" style={{ width: width ?? 340 }}>
      <div className="flex items-center justify-between border-b border-slate-100 px-3 py-2">
        <div className="flex gap-1">
          {/* M4-1 瘦身：右栏只留 详情/问题/对话 三页签（v3 定稿顺序）；建议/视图移至顶栏抽屉，任务移至工作区页 */}
          {(
            [
              ['detail', detailLabel, <Boxes key="d" size={13} />],
              ['issues', '问题', <ListChecks key="i" size={13} />],
              ['chat', '对话', <MessagesSquare key="c" size={13} />],
            ] as const
          ).map(([key, label, icon]) => (
            <button
              key={key as string}
              onClick={() => onTabChange(key as typeof tab)}
              className={`flex items-center gap-1 rounded-md px-2 py-1.5 text-[11.5px] font-semibold transition-colors ${
                tab === key ? 'bg-blue-50 text-blue-600' : 'text-slate-400 hover:text-slate-600'
              }`}
            >
              {icon}
              {label as string}
            </button>
          ))}
        </div>
        <button onClick={onClose} className="rounded p-1 text-slate-400 hover:bg-slate-100 hover:text-slate-600">
          <X size={16} />
        </button>
      </div>
      <div className="flex-1 space-y-5 overflow-y-auto p-4">
        {tab === 'issues' && <IssuesList map={map} onLocate={(id) => onLocateModule(id)} onCreateTask={onCreateTask} backendRepo={backendRepo} />}
        {/* 常驻挂载 + CSS 隐藏：切页签不清空对话状态 */}
        <div className={tab === 'chat' ? 'h-full' : 'hidden h-full'}>
          <ChatPanel backendRepo={backendRepo} map={map} onLocateModule={onLocateModule} onCreateTask={onCreateTask} />
        </div>
        {tab === 'detail' && module && <ModuleView map={map} mod={module} onCreateTask={onCreateTask} backendRepo={backendRepo} />}
        {tab === 'detail' && !module && layer && <LayerView map={map} layer={layer} onCreateTask={onCreateTask} />}
        {tab === 'detail' && !module && !layer && sub && parent && smLoaded && (
          <SubmoduleView parent={parent} sub={sub} submap={smLoaded} />
        )}
        {tab === 'detail' && !module && !layer && !sub && selection?.kind === 'submodule' && (
          <p className="pt-8 text-center text-[11.5px] leading-5 text-slate-400">
            子模块数据不存在或仍在加载中，请稍候再点击。
          </p>
        )}
        {/* M4-1 详情空选态 = 地图级摘要（设计师要求：禁止空白，三态规范最高频面板落地） */}
        {tab === 'detail' && !module && !layer && !sub && selection?.kind !== 'submodule' && (
          <div className="space-y-3 pt-2">
            <p className="text-[11px] font-semibold text-slate-400">地图概览</p>
            <div className="grid grid-cols-2 gap-2">
              {[
                ['模块', `${map.modules.length}`],
                ['依赖边', `${map.edges.length}`],
                ['架构健康分', `${map.health.score}`],
                ['耦合度', map.health.coupling],
              ].map(([k, v]) => (
                <div key={k} className="rounded-lg border border-slate-100 bg-slate-50 px-3 py-2">
                  <p className="text-[10px] text-slate-400">{k}</p>
                  <p className="text-[15px] font-bold text-slate-700">{v}</p>
                </div>
              ))}
            </div>
            {map.health.review_note && (
              <p className="rounded-lg border border-slate-100 bg-slate-50 px-3 py-2 text-[11px] leading-5 text-slate-500">
                {map.health.review_note}
              </p>
            )}
            <p className="text-[10.5px] leading-5 text-slate-400">
              点击画布中的模块卡片、层标签或展开的子模块查看详情；选中模块后顶部工具栏可展开内部结构。
            </p>
          </div>
        )}
      </div>
    </aside>
  )
}
