import { useState } from 'react'
import { toast } from '@/lib/toast'
import { AlertOctagon, AlertTriangle, ArrowRight, Crosshair, Info, Loader2, Wrench, Zap } from 'lucide-react'
import { buildConcernTask, type TaskDraft } from '@/lib/taskContext'
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

export function IssuesList({ map, onLocate, onCreateTask, backendRepo, scopeId, scopeName }: { map: CodeMap; onLocate: (moduleId: string) => void; onCreateTask: (d: TaskDraft) => void; backendRepo?: string | null; /** M4-1.5：选区收敛——模块/子模块选中时只显示该模块的问题 */ scopeId?: string; scopeName?: string }) {
  // 改进#6：问题卡"自动修复"一键直达（测试员路径优化：5 点击→2 点击）
  const [quickBusy, setQuickBusy] = useState<string | null>(null)
  const [quickDone, setQuickDone] = useState<string | null>(null)

  const quickFix = (e: React.MouseEvent, issue: Issue) => {
    e.stopPropagation()
    if (!backendRepo || quickBusy) return
    let draft: TaskDraft
    if (issue.moduleId) {
      const mod = map.modules.find((x) => x.id === issue.moduleId)
      const concern = mod?.health.concerns?.find((c) => c.finding === issue.finding)
      if (mod) {
        draft = concern
          ? buildConcernTask(map, mod.id, concern, 0)
          : buildConcernTask(map, mod.id, { severity: issue.severity, finding: issue.finding, suggestion: issue.suggestion }, 0)
      } else {
        return
      }
    } else {
      const worst = [...map.modules].sort((a, b) => a.health.score - b.health.score).slice(0, 3)
      draft = {
        title: `架构治理：${issue.finding.slice(0, 24)}`,
        description: `【架构级问题】${issue.finding}\n建议：${issue.suggestion}`,
        modules: worst.map((x) => x.id),
        acceptance: '按建议完成后，架构级问题不再复现（复检可验证趋势）',
        source: 'concern',
        context: { inject: { archConcern: { severity: issue.severity, finding: issue.finding, suggestion: issue.suggestion }, modules: worst.map((m) => ({ id: m.id, name: m.name, score: m.health.score })) } },
      }
    }
    setQuickBusy(issue.key)
    fetch(`/api/repos/${backendRepo}/tasks`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      // R5 清债：架构级问题（影响面最大）默认走 supervised 风险预评估，不直通 auto
      body: JSON.stringify({ ...draft, trust: issue.moduleId ? 'auto' : 'supervised' }),
    })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        setQuickDone(issue.key)
        setTimeout(() => setQuickDone(null), 2500)
      })
      .catch(() => toast('创建任务失败（需要本地后端在线）', 'error'))
      .finally(() => setQuickBusy(null))
  }
  // 选区收敛：有 scope 时只留该模块的问题（架构级问题属全局，收敛视图下不混入）
  const all = collectIssues(map)
  const issues = scopeId ? all.filter((i) => i.moduleId === scopeId) : all

  return (
    <div className="space-y-5">
      <div>
        <h2 className="text-[15px] font-bold text-slate-800">{scopeName ? `${scopeName} 的问题` : '问题清单'}</h2>
        <p className="mt-1 text-[11px] text-slate-400">
          {scopeName ? `选区收敛 · 该模块 ${issues.length} 项` : `共 ${issues.length} 项`} · 按严重度排序，同档按影响面（被依赖数）排序
        </p>
      </div>

      {issues.length === 0 && (
        <p className="flex items-center gap-1.5 rounded-lg bg-emerald-50 px-3 py-2.5 text-[12px] text-emerald-700">
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
              <span className="ml-auto text-micro text-slate-400">影响 {issue.impact} 模块</span>
            </div>

            <p className="mt-1.5 text-[12px] leading-5 text-slate-700">{issue.finding}</p>

            <p className="mt-1 flex items-start gap-1 text-[11px] leading-5 text-slate-500">
              <ArrowRight size={11} className="mt-1 shrink-0 text-emerald-600" />
              {issue.suggestion}
            </p>

            <div className="mt-1.5 flex items-center gap-2">
              {issue.moduleId && (
                <span className="flex items-center gap-1 text-micro font-medium text-blue-500">
                  <Crosshair size={10} /> 点击定位到画布
                </span>
              )}
              <button
                onClick={(e) => quickFix(e, issue)}
                disabled={quickBusy !== null || !backendRepo}
                className="flex items-center gap-1 rounded-full bg-emerald-600 px-2 py-0.5 text-micro font-bold text-white hover:bg-emerald-700 disabled:opacity-40"
                title="跳过表单直接自动修复（自动模式直通执行，全程留痕）——90% 的场景不需要调整范围"
              >
                {quickBusy === issue.key ? <Loader2 size={9} className="animate-spin" /> : <Zap size={9} />}
                {quickDone === issue.key ? '已创建 ✓' : '自动修复'}
              </button>
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  if (issue.moduleId) {
                    const mod = map.modules.find((m) => m.id === issue.moduleId)
                    const concern = mod?.health.concerns?.find((c) => c.finding === issue.finding)
                    if (mod) onCreateTask(concern ? buildConcernTask(map, mod.id, concern, 0) : buildConcernTask(map, mod.id, { severity: issue.severity, finding: issue.finding, suggestion: issue.suggestion }, 0))
                  } else {
                    // 架构级问题：以最低分模块为切入点 + 注入架构级 finding（试用反馈#1：架构级没有修复按钮）
                    const worst = [...map.modules].sort((a, b) => a.health.score - b.health.score).slice(0, 3)
                    onCreateTask({
                      title: `架构治理：${issue.finding.slice(0, 24)}`,
                      description: `【架构级问题】${issue.finding}\n建议：${issue.suggestion}`,
                      modules: worst.map((m) => m.id),
                      acceptance: '按建议完成后，架构级问题不再复现（复检可验证趋势）',
                      source: 'concern',
                      context: { inject: { archConcern: { severity: issue.severity, finding: issue.finding, suggestion: issue.suggestion }, modules: worst.map((m) => ({ id: m.id, name: m.name, score: m.health.score })) } },
                    })
                  }
                }}
                className="ml-auto flex items-center gap-1 rounded-full border border-blue-200 bg-blue-50 px-2 py-0.5 text-micro font-bold text-blue-600 hover:bg-blue-100"
                title="指哪打哪：以该问题为上下文发起修复任务"
              >
                <Wrench size={9} /> 修复
              </button>
            </div>
          </div>
        ))}
      </div>

      <p className="flex items-start gap-1.5 text-cap leading-4 text-slate-400">
        <Info size={11} className="mt-0.5 shrink-0" />
        严重度来自 LLM 问题提名（concerns）；影响面为确定性统计（被依赖数）。旧版地图数据无 concerns 字段时，本列表由腐化标记与评审意见兜底生成。
      </p>
    </div>
  )
}
