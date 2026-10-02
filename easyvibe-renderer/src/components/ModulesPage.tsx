import { useMemo, useState } from 'react'
import { ArrowDown, ArrowUp } from 'lucide-react'
import type { CodeMap } from '@/types/map'
import { healthColor } from '@/lib/layout'

// M4-1.5 模块目录页（从占位转正）：以模块为行的表格视图——一览全部模块的职责、
// 健康与文件归属；兼任画布的无障碍列表模式（画布是图形，目录是语义等价物）。
// 列可排序；全部数据来自主地图，零额外请求。
type SortKey = 'score' | 'name' | 'coupling' | 'files'

const COUPLING_RANK: Record<string, number> = { low: 0, medium: 1, high: 2, critical: 3 }

const FLAG_LABEL: Record<string, string> = {
  god_module: '上帝模块',
  god_object: '上帝对象',
  coupling_high: '耦合过高',
  circular_dep: '循环依赖',
  layer_violation: '分层违规',
  responsibility_overlap: '职责重叠',
  duplicated_protocol: '协议重复',
  ref_plumbing: '引用缠绕',
  doc_drift: '文档漂移',
}

export function ModulesPage({ map, onOpenMap }: { map: CodeMap; onOpenMap: () => void }) {
  const [sortKey, setSortKey] = useState<SortKey>('score')
  const [asc, setAsc] = useState(true)

  const rows = useMemo(() => {
    const mods = [...map.modules]
    mods.sort((a, b) => {
      let cmp = 0
      if (sortKey === 'score') cmp = a.health.score - b.health.score
      else if (sortKey === 'name') cmp = a.name.localeCompare(b.name, 'zh')
      else if (sortKey === 'coupling') cmp = (COUPLING_RANK[a.health.coupling] ?? 0) - (COUPLING_RANK[b.health.coupling] ?? 0)
      else cmp = a.files.length - b.files.length
      return asc ? cmp : -cmp
    })
    return mods
  }, [map, sortKey, asc])

  const layerName = (id: string) => map.layers.find((l) => l.id === id)?.name ?? id

  const Th = ({ k, children }: { k: SortKey; children: React.ReactNode }) => (
    <button
      onClick={() => {
        if (sortKey === k) setAsc((v) => !v)
        else {
          setSortKey(k)
          setAsc(true)
        }
      }}
      className={`flex items-center gap-0.5 text-[11px] font-semibold ${sortKey === k ? 'text-blue-600' : 'text-slate-400 hover:text-slate-600'}`}
    >
      {children}
      {sortKey === k && (asc ? <ArrowUp size={10} /> : <ArrowDown size={10} />)}
    </button>
  )

  return (
    <div className="h-full overflow-y-auto p-5">
      <div className="mb-3 flex items-center justify-between">
        <div>
          <h2 className="text-[15px] font-bold text-slate-800">模块目录</h2>
          <p className="mt-0.5 text-[11px] text-slate-400">
            全部 <span className="tnum">{map.modules.length}</span> 个模块的职责与健康一览——画布的列表模式
          </p>
        </div>
        <button
          onClick={onOpenMap}
          className="rounded-lg border border-slate-200 bg-white px-3 py-1.5 text-[11px] font-semibold text-slate-600 hover:bg-slate-50"
        >
          打开架构地图 →
        </button>
      </div>
      <div className="overflow-hidden rounded-xl border border-slate-200 bg-white">
        <table className="w-full text-left">
          <thead>
            <tr className="border-b border-slate-100 bg-slate-50/60 text-slate-400">
              <th className="px-3 py-2"><Th k="name">模块</Th></th>
              <th className="px-3 py-2 text-[11px] font-semibold">层</th>
              <th className="px-3 py-2"><Th k="score">健康</Th></th>
              <th className="px-3 py-2"><Th k="coupling">耦合</Th></th>
              <th className="px-3 py-2 text-[11px] font-semibold">腐化标记</th>
              <th className="px-3 py-2"><Th k="files">文件</Th></th>
              <th className="px-3 py-2 text-[11px] font-semibold">职责</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((m) => (
              <tr key={m.id} className="border-b border-slate-50 last:border-0 hover:bg-slate-50/60">
                <td className="px-3 py-2">
                  <p className="text-[12px] font-semibold text-slate-700">{m.name}</p>
                  <p className="mono text-micro text-slate-300">{m.id}</p>
                </td>
                <td className="px-3 py-2 text-[12px] text-slate-500">{layerName(m.layer)}</td>
                <td className="px-3 py-2">
                  <span className="tnum text-[13px] font-bold" style={{ color: healthColor(m.health.score) }}>
                    {m.health.score}
                  </span>
                </td>
                <td className="px-3 py-2 text-[12px] text-slate-500">{m.health.coupling}</td>
                <td className="px-3 py-2">
                  <div className="flex flex-wrap gap-1">
                    {m.health.decay_flags.slice(0, 3).map((f) => (
                      <span key={f} className="rounded-full border border-red-200 bg-red-50 px-1.5 py-px text-micro leading-4 text-red-600">
                        {FLAG_LABEL[f] ?? f}
                      </span>
                    ))}
                  </div>
                </td>
                <td className="px-3 py-2"><span className="tnum text-[12px] text-slate-500">{m.files.length}</span></td>
                <td className="max-w-[280px] px-3 py-2 text-[11px] leading-4 text-slate-500">{m.responsibility}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}
