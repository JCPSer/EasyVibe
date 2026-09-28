import { AlertOctagon, AlertTriangle, ArrowRight, Crosshair, Info } from 'lucide-react'
import type { CodeMap, Concern, Module } from '@/types/map'
import { healthColor, dependentsOf } from '@/lib/layout'
import { Badge } from '@/components/ui/badge'

export interface Issue {
  key: string
  scope: 'arch' | 'module'
  moduleId?: string
  moduleName?: string
  severity: 'critical' | 'high'
  finding: string
  suggestion: string
  impact: number // 影响面：被依赖数（架构级固定为模块总数）
}

// 汇总全库问题：优先用 LLM 提名的 concerns；没有 concerns 的旧数据用 decay_flags + review_note 兜底
export function collectIssues(map: CodeMap): Issue[] {
  const issues: Issue[] = []

  const arch = map.health
  if (arch.concerns?.length) {
    for (const [i, c] of arch.concerns.entries()) {
      issues.push({ key: `arch-${i}`, scope: 'arch', severity: c.severity, finding: c.finding, suggestion: c.suggestion, impact: map.modules.length })
    }
  } else if (arch.decay_flags.length > 0) {
    issues.push({
      key: 'arch-0', scope: 'arch',
      severity: arch.score < 60 ? 'critical' : 'high',
      finding: `架构级腐化：${arch.decay_flags.join('、')}`,
      suggestion: arch.review_note || '见架构级评审意见',
      impact: map.modules.length,
    })
  }

  for (const mod of map.modules) {
    const impact = dependentsOf(map, mod).length
    if (mod.health.concerns?.length) {
      mod.health.concerns.forEach((c: Concern, i: number) => {
        issues.push({ key: `${mod.id}-${i}`, scope: 'module', moduleId: mod.id, moduleName: mod.name, severity: c.severity, finding: c.finding, suggestion: c.suggestion, impact })
      })
    } else if (mod.health.decay_flags.length > 0) {
      issues.push({
        key: `${mod.id}-0`, scope: 'module', moduleId: mod.id, moduleName: mod.name,
        severity: mod.health.score < 60 ? 'critical' : 'high',
        finding: `${mod.name}：${mod.health.decay_flags.join('、')}`,
        suggestion: mod.health.review_note || '见模块评审意见',
        impact,
      })
    }
  }

  const rank = { critical: 0, high: 1 }
  return issues.sort((a, b) => rank[a.severity] - rank[b.severity] || b.impact - a.impact)
}

export function isIssueModule(mod: Module): boolean {
  return mod.health.decay_flags.length > 0 || (mod.health.score ?? 100) < 75 || (mod.health.concerns?.length ?? 0) > 0
}

function SeverityChip({ severity }: { severity: 'critical' | 'high' }) {
  const critical = severity === 'critical'
  return (
    <Badge className={critical ? 'bg-red-100 font-normal text-red-700 hover:bg-red-100' : 'bg-amber-100 font-normal text-amber-700 hover:bg-amber-100'}>
      {critical ? (
        <AlertOctagon size={10} className="mr-1" />
      ) : (
        <AlertTriangle size={10} className="mr-1" />
      )}
      {severity}
    </Badge>
  )
}

export function IssuesList({ map, onLocate }: { map: CodeMap; onLocate: (moduleId: string) => void }) {
  const issues = collectIssues(map)

  return (
    <div className="space-y-5">
      <div>
        <h2 className="text-[15px] font-bold text-slate-800">问题清单</h2>
        <p className="mt-1 text-[11px] text-slate-400">
          共 {issues.length} 项 · 按严重度排序，同档按影响面（被依赖数）排序
        </p>
      </div>

      {issues.length === 0 && (
        <p className="flex items-center gap-1.5 rounded-lg bg-emerald-50 px-3 py-2.5 text-[11.5px] text-emerald-700">
          <Info size={13} /> 当前地图没有检出问题，保持健康。
        </p>
      )}

      <div className="space-y-2.5">
        {issues.map((issue) => (
          <div
            key={issue.key}
            className={`rounded-lg border p-3 transition-colors ${
              issue.moduleId ? 'cursor-pointer hover:border-blue-300 hover:bg-blue-50/40' : 'border-red-200 bg-red-50/40'
            }`}
            style={issue.moduleId ? { borderColor: '#e2e8f0' } : undefined}
            onClick={() => issue.moduleId && onLocate(issue.moduleId!)}
          >
            <div className="flex items-center gap-2">
              <SeverityChip severity={issue.severity} />
              {issue.scope === 'arch' ? (
                <span className="text-[11px] font-bold text-red-600">架构级</span>
              ) : (
                <span className="flex items-center gap-1 text-[11px] font-semibold text-slate-600">
                  {issue.moduleName}
                  <span
                    className="h-1.5 w-1.5 rounded-full"
                    style={{ background: healthColor(map.modules.find((m) => m.id === issue.moduleId)!.health.score) }}
                  />
                </span>
              )}
              <span className="ml-auto text-[10px] text-slate-400">影响 {issue.impact} 模块</span>
            </div>

            <p className="mt-1.5 text-[11.5px] leading-5 text-slate-700">{issue.finding}</p>

            <p className="mt-1 flex items-start gap-1 text-[11px] leading-5 text-slate-500">
              <ArrowRight size={11} className="mt-1 shrink-0 text-emerald-600" />
              {issue.suggestion}
            </p>

            {issue.moduleId && (
              <p className="mt-1.5 flex items-center gap-1 text-[10px] font-medium text-blue-500">
                <Crosshair size={10} /> 点击定位到画布
              </p>
            )}
          </div>
        ))}
      </div>

      <p className="flex items-start gap-1.5 text-[10.5px] leading-4 text-slate-400">
        <Info size={11} className="mt-0.5 shrink-0" />
        严重度来自 LLM 问题提名（concerns）；影响面为确定性统计（被依赖数）。旧版地图数据无 concerns 字段时，本列表由腐化标记与评审意见兜底生成。
      </p>
    </div>
  )
}
