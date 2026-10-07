import { useEffect, useMemo, useState } from 'react'
import { FileCode2, KeyRound, Flag, StickyNote, ArrowDownToLine, ArrowUpFromLine, Boxes, Info, Wrench, MessagesSquare, Gauge, Waypoints } from 'lucide-react'
import { buildLayerTask, buildModuleTask, buildSubmoduleTask, type TaskDraft } from '@/shared/logic/taskContext'
import type { CodeMap, Layer, Module, SubMap, SubModule } from '@/types/map'
import { healthColor, healthLabel, dependentsOf } from '@/shared/logic/layout'
import { couplingAnalysis } from '@/shared/logic/depsAnalysis'
import { Badge } from '@/components/ui/badge'
import { Separator } from '@/components/ui/separator'
import { healthHistory } from '@/api/canvas'
import { usage } from '@/api/repos'
import { useLang } from '@/runtime/i18n'
import type { ChatAboutTarget } from '@/shared/contract/chat'

// 右栏详情页签的三个选中对象视图（模块/架构层/子模块）+ 健康趋势 + 治理账单。
// 英文化第二批从 DetailPanel 抽出：DetailPanel 贴 componentGuard 红线（LEGACY 628），
// 文案迁移改为 t() 后行数净增，抽成独立件两边都回到阈值内。

export function Row({ icon, label, children }: { icon: React.ReactNode; label: string; children: React.ReactNode }) {
  return (
    <div>
      <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-slate-400 dark:text-slate-500">
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
  const { t } = useLang()
  const [rows, setRows] = useState<{ score: number; runId: string }[] | null>(null)
  useEffect(() => {
    let stale = false
    healthHistory(backendRepo, moduleId)
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
      <span className="text-micro font-semibold text-slate-400 dark:text-slate-500">{t('canvas.panel.trendLabel')}</span>
      <div className="flex h-4 items-end gap-0.5">
        {rows.slice(-12).map((r, i) => (
          <div
            key={i}
            className="w-1.5 rounded-sm"
            style={{ height: `${Math.max(12, r.score)}%`, background: healthColor(r.score) }}
            title={t('canvas.panel.trendScoreTip', { score: r.score })}
          />
        ))}
      </div>
      <span className={`text-micro font-bold ${delta >= 0 ? 'text-emerald-600' : 'text-red-500'}`}>
        {delta >= 0 ? '+' : ''}
        {delta}
      </span>
    </div>
  )
}

// L1 治理账单：模块累计 agent 成本（用量页按模块归因的详情端落点）。
// 数据：GET /usage?days=30 的 byModule + sessions 过滤本模块；修复成效取 health-history 首末分差。
function GovernanceBill({ backendRepo, moduleId }: { backendRepo: string | null; moduleId: string }) {
  const { t } = useLang()
  const [bill, setBill] = useState<{ cost: number | null; sessions: number; failed: number; recent: { id: string; label?: string | null; kind: string; costUsd?: number | null; status: string; startedAt: string }[] } | null>(null)
  const [scoreDelta, setScoreDelta] = useState<number | null>(null)
  useEffect(() => {
    if (!backendRepo) return
    let stale = false
    usage(backendRepo, 30)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { byModule?: { name: string; sessions: number; cost?: number | null; failed?: number | null }[]; sessions?: { id: string; label?: string | null; kind: string; moduleId?: string | null; costUsd?: number | null; status: string; startedAt: string }[] } } | null) => {
        if (stale || !d?.data) return
        const row = d.data.byModule?.find((m) => m.name === moduleId)
        const recent = (d.data.sessions ?? []).filter((s) => s.moduleId === moduleId).slice(0, 3)
        setBill({ cost: row?.cost ?? null, sessions: row?.sessions ?? 0, failed: row?.failed ?? 0, recent })
      })
      .catch(() => {})
    healthHistory(backendRepo, moduleId)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { score: number }[] } | null) => {
        if (stale) return
        const rows = d?.data ?? []
        if (rows.length >= 2) setScoreDelta(rows[rows.length - 1].score - rows[0].score)
      })
      .catch(() => {})
    return () => {
      stale = true
    }
  }, [backendRepo, moduleId])

  if (!backendRepo || !bill || (bill.sessions === 0 && bill.cost == null)) return null
  // 会话 kind → 短标签（字典 canvas.panel.kind.*；未知 kind 回退 label/kind 原值）
  const kindLabel = (kind: string, label?: string | null) => {
    const key = `canvas.panel.kind.${kind}`
    const s = t(key)
    return s === key ? label ?? kind : s
  }
  return (
    <Row icon={<Gauge size={12} />} label={t('canvas.panel.billLabel')}>
      <div className="space-y-1.5 rounded-lg border border-slate-100 dark:border-slate-800 bg-slate-50 dark:bg-slate-950/70 p-2.5">
        <div className="flex items-center gap-3 text-cap">
          <span className="tnum font-bold text-slate-700 dark:text-slate-200">{bill.cost == null ? '—' : `$${bill.cost.toFixed(2)}`}</span>
          <span className="text-slate-400 dark:text-slate-500">·</span>
          <span className="tnum text-slate-500 dark:text-slate-400">{t('canvas.panel.billSessions', { count: bill.sessions })}</span>
          {bill.failed > 0 && (
            <>
              <span className="text-slate-400 dark:text-slate-500">·</span>
              <span className="tnum text-red-500">{t('canvas.panel.billFailed', { count: bill.failed })}</span>
            </>
          )}
          {scoreDelta !== null && (
            <>
              <span className="text-slate-400 dark:text-slate-500">·</span>
              <span className={`tnum font-semibold ${scoreDelta >= 0 ? 'text-emerald-600' : 'text-red-500'}`}>
                {t('canvas.panel.billDelta', { delta: `${scoreDelta >= 0 ? '+' : ''}${scoreDelta}` })}
              </span>
            </>
          )}
        </div>
        {bill.recent.length > 0 && (
          <div className="space-y-0.5">
            {bill.recent.map((r) => (
              <div key={r.id} className="flex items-center gap-1.5 text-micro text-slate-400 dark:text-slate-500">
                <span className={`h-1 w-1 rounded-full ${r.status === 'failed' ? 'bg-red-400' : 'bg-emerald-400'}`} />
                <span className="text-slate-500 dark:text-slate-400">{kindLabel(r.kind, r.label)}</span>
                <span className="tnum">{r.costUsd == null ? '—' : `$${r.costUsd.toFixed(2)}`}</span>
                <span className="ml-auto">{r.startedAt.slice(5, 16).replace('T', ' ')}</span>
              </div>
            ))}
          </div>
        )}
        <p className="text-micro leading-4 text-slate-400 dark:text-slate-500">{t('canvas.panel.billFootnote')}</p>
      </div>
    </Row>
  )
}

export function ModuleView({ map, mod, onCreateTask, backendRepo, onChatAbout, onOpenDeps }: { map: CodeMap; mod: Module; onCreateTask: (d: TaskDraft) => void; backendRepo: string | null; onChatAbout?: (target: ChatAboutTarget) => void; onOpenDeps?: () => void }) {
  const { t } = useLang()
  const color = healthColor(mod.health.score)
  const deps = mod.dependencies.map((id) => map.modules.find((m) => m.id === id)).filter(Boolean) as Module[]
  const dependents = dependentsOf(map, mod)
    .map((id) => map.modules.find((m) => m.id === id))
    .filter(Boolean) as Module[]
  // 2026-10-05 依赖体检入口：耦合概览（≤3 行 = 计数 + 首条违规 + 看全部）
  const analysis = useMemo(() => couplingAnalysis(map), [map])
  const nameOf = (id: string) => analysis.moduleById.get(id)?.name ?? id
  const myViolations = analysis.violations.filter((e) => e.from === mod.id || e.to === mod.id)
  const firstViolation = myViolations[0]

  return (
    <>
      <div>
        <div className="flex items-center gap-2">
          <h2 className="text-[15px] font-bold text-slate-800 dark:text-slate-100">{mod.name}</h2>
          <Badge variant="outline" className="border-slate-200 dark:border-slate-700 text-slate-500 dark:text-slate-400">
            {mod.id}
          </Badge>
        </div>
        <p className="mt-1 text-[12px] leading-5 text-slate-500 dark:text-slate-400">{mod.responsibility}</p>
        <p className="mt-1 text-[11px] text-slate-400 dark:text-slate-500">
          {t('canvas.panel.layerOf', { name: map.layers.find((l) => l.id === mod.layer)?.name ?? mod.layer })}
        </p>
      </div>

      <div className="rounded-lg border p-3" style={{ borderColor: `${color}55`, background: `${color}0d` }}>
        {/* M4-1 真人测试 Bug#3：原 justify-between 三元素挤一行，按钮压住指标文案——改两行布局 */}
        <div className="flex items-center justify-between">
          <span className="text-[11px] font-semibold" style={{ color }}>
            {healthLabel(mod.health.score)} · {mod.health.score}/100
          </span>
          <div className="flex items-center gap-1.5">
            {/* v0.2：对话入口（带模块上下文跳「任务对话」，@芯片+预填文本） */}
            {onChatAbout && (
              <button
                onClick={() => onChatAbout({ refId: mod.id, refName: mod.name, kind: 'module' })}
                className="flex items-center gap-1 rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1 text-micro font-bold text-slate-600 dark:text-slate-300 hover:border-blue-300 hover:text-blue-600"
                title={t('canvas.panel.chatModuleTip')}
              >
                <MessagesSquare size={10} /> {t('canvas.panel.chat')}
              </button>
            )}
            <button
              onClick={() => onCreateTask(buildModuleTask(map, mod.id))}
              className="flex items-center gap-1 rounded-full bg-blue-600 px-2.5 py-1 text-micro font-bold text-white hover:bg-blue-700"
              title={t('canvas.panel.fixModuleTip')}
            >
              <Wrench size={10} /> {t('canvas.panel.fix')}
            </button>
          </div>
        </div>
        <p className="mt-1 text-cap text-slate-400 dark:text-slate-500">
          coupling {mod.health.coupling} · complexity {mod.health.complexity} · churn {mod.health.churn ?? 'n/a'}
        </p>
        {backendRepo && <HealthTrend backendRepo={backendRepo} moduleId={mod.id} />}
        {mod.health.review_note && <p className="mt-2 text-[12px] leading-5 text-slate-600 dark:text-slate-300">{mod.health.review_note}</p>}
      </div>

      <Row icon={<KeyRound size={12} />} label={t('canvas.panel.entries')}>
        <div className="space-y-1.5">
          {mod.key_entries.slice(0, 6).map((k) => (
            <div key={k.file + k.symbol} className="rounded-md bg-slate-50 dark:bg-slate-950/70 px-2.5 py-1.5">
              <div className="font-mono text-[11px] font-medium text-slate-700 dark:text-slate-200">{k.symbol}</div>
              <div className="truncate font-mono text-micro text-slate-400 dark:text-slate-500">{k.file}</div>
            </div>
          ))}
          {mod.key_entries.length === 0 && <p className="text-[11px] text-slate-400 dark:text-slate-500">{t('canvas.panel.empty')}</p>}
        </div>
      </Row>

      <Row icon={<FileCode2 size={12} />} label={t('canvas.panel.files')}>
        <div className="space-y-1">
          {mod.files.map((f) => (
            <div key={f} className="truncate font-mono text-cap text-slate-500 dark:text-slate-400">
              {f}
            </div>
          ))}
        </div>
      </Row>

      <Row icon={<ArrowDownToLine size={12} />} label={t('canvas.panel.deps', { count: deps.length })}>
        <div className="flex flex-wrap gap-1.5">
          {deps.map((d) => (
            <Badge key={d.id} variant="secondary" className="bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300">
              {d.name}
            </Badge>
          ))}
          {deps.length === 0 && <p className="text-[11px] text-slate-400 dark:text-slate-500">{t('canvas.panel.empty')}</p>}
        </div>
      </Row>

      <Row icon={<ArrowUpFromLine size={12} />} label={t('canvas.panel.dependents', { count: dependents.length })}>
        <div className="flex flex-wrap gap-1.5">
          {dependents.map((d) => (
            <Badge key={d.id} variant="secondary" className="bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300">
              {d.name}
            </Badge>
          ))}
          {dependents.length === 0 && <p className="text-[11px] text-slate-400 dark:text-slate-500">{t('canvas.panel.empty')}</p>}
        </div>
      </Row>

      <GovernanceBill backendRepo={backendRepo} moduleId={mod.id} />

      <Row icon={<Waypoints size={12} />} label={t('canvas.panel.coupling')}>
        <div className="space-y-1.5 rounded-lg border border-slate-100 dark:border-slate-800 bg-slate-50 dark:bg-slate-950/70 p-2.5">
          <p className="tnum text-cap text-slate-500 dark:text-slate-400">
            {t('canvas.panel.fan', { out: analysis.fanOut.get(mod.id) ?? 0, in: analysis.fanIn.get(mod.id) ?? 0, violations: myViolations.length })}
            {analysis.cycleModuleIds.has(mod.id) && <span className="text-amber-500">{t('canvas.panel.inCycle')}</span>}
          </p>
          {firstViolation && (
            <p className="text-cap leading-4 text-red-500">
              {t('deps.card.violation.title', { from: nameOf(firstViolation.from), to: nameOf(firstViolation.to) })}
            </p>
          )}
          {onOpenDeps && (
            <button onClick={onOpenDeps} className="text-micro font-bold text-blue-600 hover:text-blue-700">
              {t('canvas.panel.viewAll')}
            </button>
          )}
        </div>
      </Row>

      {mod.health.decay_flags.length > 0 && (
        <Row icon={<Flag size={12} />} label={t('canvas.panel.decay')}>
          <div className="flex flex-wrap gap-1.5">
            {mod.health.decay_flags.map((f) => (
              <Badge key={f} className="border-red-200 dark:border-red-900/60 bg-red-50 dark:bg-red-950/40 font-normal text-red-600">
                {t(`canvas.flags.${f}`)}
              </Badge>
            ))}
          </div>
        </Row>
      )}

      <Separator />

      <Row icon={<StickyNote size={12} />} label={t('canvas.panel.note')}>
        <p className="text-[11px] leading-5 text-slate-400 dark:text-slate-500">
          {t('canvas.panel.moduleNote', { generator: map.meta.generator })}
        </p>
      </Row>
    </>
  )
}

export function LayerView({ map, layer, onCreateTask, onChatAbout }: { map: CodeMap; layer: Layer; onCreateTask: (d: TaskDraft) => void; onChatAbout?: (target: ChatAboutTarget) => void }) {
  const { t } = useLang()
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
          <Boxes size={16} className="text-slate-500 dark:text-slate-400" />
          <h2 className="text-[15px] font-bold text-slate-800 dark:text-slate-100">{layer.name}</h2>
          <Badge variant="outline" className="border-slate-200 dark:border-slate-700 text-slate-500 dark:text-slate-400">
            {layer.id}
          </Badge>
        </div>
        <p className="mt-1 text-[12px] leading-5 text-slate-500 dark:text-slate-400">{layer.description}</p>
        <p className="mt-1 text-[11px] text-slate-400 dark:text-slate-500">{t('canvas.panel.layerOrder', { order: layer.order })}</p>
      </div>

      <div className="rounded-lg border p-3" style={{ borderColor: `${color}55`, background: `${color}0d` }}>
        {/* M4-1.5 陪审团硬缺陷：三元素挤一行压字断行——两行布局 */}
        <div className="flex items-center justify-between">
          <span className="text-[11px] font-semibold" style={{ color }}>
            {t('canvas.panel.layerHealth')}<span className="tnum">{avg}</span>/100
          </span>
          <div className="flex items-center gap-1.5">
            {onChatAbout && (
              <button
                onClick={() => onChatAbout({ refId: layer.id, refName: layer.name, kind: 'layer' })}
                className="flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 px-2.5 py-1 text-micro font-bold text-slate-600 dark:text-slate-300 hover:border-blue-300 hover:text-blue-600"
                title={t('canvas.panel.chatLayerTip')}
              >
                <MessagesSquare size={10} /> {t('canvas.panel.chat')}
              </button>
            )}
            <button
              onClick={() => onCreateTask(buildLayerTask(map, layer.id))}
              className="flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full bg-blue-600 px-2.5 py-1 text-micro font-bold text-white hover:bg-blue-700"
              title={t('canvas.panel.fixLayerTip')}
            >
              <Wrench size={10} /> {t('canvas.panel.fixLayer')}
            </button>
          </div>
        </div>
        <p className="mt-1 text-cap text-slate-400 dark:text-slate-500">
          {t('canvas.panel.layerStats', { count: mods.length, violations: violations.length })}
        </p>
        <p className="mt-2 flex items-start gap-1.5 text-cap leading-4 text-slate-500 dark:text-slate-400">
          <Info size={11} className="mt-0.5 shrink-0 text-slate-400 dark:text-slate-500" />
          {t('canvas.panel.layerNote')}
        </p>
      </div>

      <Row icon={<Flag size={12} />} label={t('canvas.panel.layerViolations', { count: violations.length })}>
        <div className="space-y-1.5">
          {violations.slice(0, 8).map((e, i) => (
            <div key={i} className="flex items-center gap-1.5 rounded-md bg-red-50 dark:bg-red-950/40 px-2 py-1.5 text-cap text-red-600">
              <span className="font-mono font-medium">{e.from}</span>
              <span className="text-red-400">→</span>
              <span className="font-mono font-medium">{e.to}</span>
              <span className="ml-auto truncate text-red-400">{e.label ?? e.type}</span>
            </div>
          ))}
          {violations.length === 0 && <p className="text-[11px] text-slate-400 dark:text-slate-500">{t('canvas.panel.layerViolationsEmpty')}</p>}
        </div>
      </Row>

      <Row icon={<Boxes size={12} />} label={t('canvas.panel.layerMembers', { count: mods.length })}>
        <div className="space-y-1.5">
          {mods.map((m) => (
            <div key={m.id} className="flex items-center gap-2 rounded-md bg-slate-50 dark:bg-slate-950/70 px-2.5 py-2">
              <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: healthColor(m.health.score) }} />
              <div className="min-w-0">
                <div className="text-[12px] font-medium text-slate-700 dark:text-slate-200">{m.name}</div>
                <div className="truncate text-micro text-slate-400 dark:text-slate-500">{m.responsibility}</div>
              </div>
              <span className="ml-auto text-cap font-semibold" style={{ color: healthColor(m.health.score) }}>
                {m.health.score}
              </span>
            </div>
          ))}
        </div>
      </Row>

      {downViolations > 0 && (
        <p className="text-cap leading-4 text-slate-400 dark:text-slate-500">
          {t('canvas.panel.downViolations', { count: downViolations })}
        </p>
      )}
    </>
  )
}

// 子模块详情（§8 子图 drill-down 层）
export function SubmoduleView({ parent, sub, submap, onCreateTask }: { parent: Module; sub: SubModule; submap: SubMap; onCreateTask: (d: TaskDraft) => void }) {
  const { t } = useLang()
  const color = healthColor(sub.health.score)
  const siblings = new Map(submap.sub_modules.map((s) => [s.id, s]))
  const deps = sub.dependencies.map((id) => siblings.get(id)).filter(Boolean) as SubModule[]
  const dependents = submap.edges.filter((e) => e.to === sub.id).map((e) => siblings.get(e.from)).filter(Boolean) as SubModule[]

  return (
    <>
      <div>
        <div className="flex items-center gap-2">
          <h2 className="text-[15px] font-bold text-slate-800 dark:text-slate-100">{sub.name}</h2>
          {/* M4-1.5：子模块补上修复入口（模块/层都有，此前独缺） */}
          <button
            onClick={() => onCreateTask(buildSubmoduleTask(parent, sub))}
            className="ml-auto flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full bg-blue-600 px-2.5 py-1 text-micro font-bold text-white hover:bg-blue-700"
            title={t('canvas.panel.fixModuleTip')}
          >
            <Wrench size={10} /> {t('canvas.panel.fix')}
          </button>
          <Badge variant="outline" className="border-slate-200 dark:border-slate-700 text-slate-500 dark:text-slate-400">
            {sub.id}
          </Badge>
        </div>
        <p className="mt-1 text-[12px] leading-5 text-slate-500 dark:text-slate-400">{sub.responsibility}</p>
        <p className="mt-1 text-[11px] text-slate-400 dark:text-slate-500">
          {t('canvas.panel.subParent', { name: parent.name })}
        </p>
      </div>

      <div className="rounded-lg border p-3" style={{ borderColor: `${color}55`, background: `${color}0d` }}>
        <div className="flex items-center justify-between">
          <span className="text-[11px] font-semibold" style={{ color }}>
            {healthLabel(sub.health.score)} · {sub.health.score}/100
          </span>
          <span className="text-cap text-slate-400 dark:text-slate-500">
            coupling {sub.health.coupling} · complexity {sub.health.complexity} · churn {sub.health.churn ?? 'n/a'}
          </span>
        </div>
        {sub.health.review_note && <p className="mt-2 text-[12px] leading-5 text-slate-600 dark:text-slate-300">{sub.health.review_note}</p>}
      </div>

      <Row icon={<KeyRound size={12} />} label={t('canvas.panel.entries')}>
        <div className="space-y-1.5">
          {sub.key_entries.slice(0, 6).map((k) => (
            <div key={k.file + k.symbol} className="rounded-md bg-slate-50 dark:bg-slate-950/70 px-2.5 py-1.5">
              <div className="font-mono text-[11px] font-medium text-slate-700 dark:text-slate-200">{k.symbol}</div>
              <div className="truncate font-mono text-micro text-slate-400 dark:text-slate-500">{k.file}</div>
            </div>
          ))}
        </div>
      </Row>

      <Row icon={<FileCode2 size={12} />} label={t('canvas.panel.files')}>
        <div className="space-y-1">
          {sub.files.map((f) => (
            <div key={f} className="truncate font-mono text-cap text-slate-500 dark:text-slate-400">{f}</div>
          ))}
        </div>
      </Row>

      <Row icon={<ArrowDownToLine size={12} />} label={t('canvas.panel.subDeps', { count: deps.length })}>
        <div className="flex flex-wrap gap-1.5">
          {deps.map((d) => (
            <Badge key={d.id} variant="secondary" className="bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300">{d.name}</Badge>
          ))}
          {deps.length === 0 && <p className="text-[11px] text-slate-400 dark:text-slate-500">{t('canvas.panel.empty')}</p>}
        </div>
      </Row>

      <Row icon={<ArrowUpFromLine size={12} />} label={t('canvas.panel.subDependents', { count: dependents.length })}>
        <div className="flex flex-wrap gap-1.5">
          {dependents.map((d) => (
            <Badge key={d.id} variant="secondary" className="bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-300">{d.name}</Badge>
          ))}
          {dependents.length === 0 && <p className="text-[11px] text-slate-400 dark:text-slate-500">{t('canvas.panel.empty')}</p>}
        </div>
      </Row>

      {sub.health.decay_flags.length > 0 && (
        <Row icon={<Flag size={12} />} label={t('canvas.panel.decay')}>
          <div className="flex flex-wrap gap-1.5">
            {sub.health.decay_flags.map((f) => (
              <Badge key={f} className="border-red-200 dark:border-red-900/60 bg-red-50 dark:bg-red-950/40 font-normal text-red-600">{t(`canvas.flags.${f}`)}</Badge>
            ))}
          </div>
        </Row>
      )}

      <Separator />

      <Row icon={<Info size={12} />} label={t('canvas.panel.note')}>
        <p className="text-[11px] leading-5 text-slate-400 dark:text-slate-500">
          {t('canvas.panel.subNote1')}<b>{t('canvas.panel.subNoteBold')}</b>{t('canvas.panel.subNote2', { generator: submap.generator })}
        </p>
      </Row>
    </>
  )
}
