import { useEffect, useState } from 'react'
import { toast } from '@/runtime/toast'
import { patrolRuns } from '@/api/repos'
import { createTask } from '@/api/task'
import { AlertOctagon, AlertTriangle, ArrowRight, Crosshair, Info, Loader2, Wrench, Zap } from 'lucide-react'
import { buildConcernTask, type TaskDraft } from '@/shared/logic/taskContext'
import { useLang, t as tStatic } from '@/runtime/i18n'
import type { CodeMap, Concern, Module } from '@/types/map'
import { healthColor, dependentsOf } from '@/shared/logic/layout'
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

// 汇总全库问题：优先用 LLM 提名的 concerns；没有 concerns 的旧数据用 decay_flags + review_note 兜底。
// tx：文案翻译注入（英文化第二批）——collectIssues 也被纯逻辑测试直接调用，缺省回退中文。
export function collectIssues(map: CodeMap, tx?: (key: string, vars?: Record<string, string | number>) => string): Issue[] {
  const or = (key: string, vars: Record<string, string | number>, fallback: string) => {
    if (!tx) return fallback
    const s = tx(key, vars)
    return s === key ? fallback : s
  }
  const issues: Issue[] = []

  const arch = map.health
  if (arch.concerns?.length) {
    for (const [i, c] of arch.concerns.entries()) {
      issues.push({ key: `arch-${i}`, scope: 'arch', severity: c.severity, finding: c.finding, suggestion: c.suggestion, impact: map.modules.length })
    }
  } else if (arch.decay_flags.length > 0) {
    const flags = arch.decay_flags.join('、')
    issues.push({
      key: 'arch-0', scope: 'arch',
      severity: arch.score < 60 ? 'critical' : 'high',
      finding: or('canvas.issues.archFinding', { flags }, `架构级腐化：${flags}`),
      suggestion: arch.review_note || or('canvas.issues.archSuggestionFallback', {}, '见架构级评审意见'),
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
      const flags = mod.health.decay_flags.join('、')
      issues.push({
        key: `${mod.id}-0`, scope: 'module', moduleId: mod.id, moduleName: mod.name,
        severity: mod.health.score < 60 ? 'critical' : 'high',
        finding: or('canvas.issues.moduleFinding', { name: mod.name, flags }, `${mod.name}：${flags}`),
        suggestion: mod.health.review_note || or('canvas.issues.moduleSuggestionFallback', {}, '见模块评审意见'),
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
    <Badge className={critical ? 'bg-red-100 font-normal text-red-700 hover:bg-red-100' : 'bg-amber-100 font-normal text-amber-700 hover:bg-amber-100 dark:hover:bg-amber-900/40'}>
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
  const { t } = useLang()
  // 改进#6：问题卡"自动修复"一键直达（测试员路径优化：5 点击→2 点击）
  const [quickBusy, setQuickBusy] = useState<string | null>(null)
  const [quickDone, setQuickDone] = useState<string | null>(null)
  // 2026-10-05 巡检新旧对照：最新一轮成功巡检的「修复/新增/持续」——
  // map prop 随巡检写回而变化，以此作为刷新信号（PatrolFinished 后前端重载地图）
  const [patrolDiff, setPatrolDiff] = useState<{ fixed: { finding: string }[]; new: { finding: string }[]; persisted: number; moduleGone: { module?: string; finding: string }[] } | null>(null)
  useEffect(() => {
    if (!backendRepo) return
    let dead = false
    patrolRuns(backendRepo)
      .then((r) => (r.ok ? r.json() : null))
      .then((d: { data?: { runs?: { status: string; concernsDiff?: unknown }[] } } | null) => {
        if (dead) return
        const hit = (d?.data?.runs ?? []).find((x) => x.status === 'succeeded' && x.concernsDiff)
        const cd = hit?.concernsDiff as typeof patrolDiff
        setPatrolDiff(cd && (cd.fixed.length || cd.new.length || cd.persisted || cd.moduleGone.length) ? cd : null)
      })
      .catch(() => {})
    return () => {
      dead = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [backendRepo, map])

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
        title: tStatic('canvas.issues.taskTitle', { finding: issue.finding.slice(0, 24) }),
        description: tStatic('canvas.issues.taskDesc', { finding: issue.finding, suggestion: issue.suggestion }),
        modules: worst.map((x) => x.id),
        acceptance: tStatic('canvas.issues.taskAcceptance'),
        source: 'concern',
        context: { inject: { archConcern: { severity: issue.severity, finding: issue.finding, suggestion: issue.suggestion }, modules: worst.map((m) => ({ id: m.id, name: m.name, score: m.health.score })) } },
      }
    }
    setQuickBusy(issue.key)
    // R5 清债：架构级问题（影响面最大）默认走 supervised 风险预评估，不直通 auto
    createTask(backendRepo, { ...draft, trust: issue.moduleId ? 'auto' : 'supervised' })
      .then((r) => {
        if (!r.ok) throw new Error(String(r.status))
        setQuickDone(issue.key)
        setTimeout(() => setQuickDone(null), 2500)
      })
      .catch(() => toast(tStatic('canvas.issues.createFailed'), 'error'))
      .finally(() => setQuickBusy(null))
  }
  // 选区收敛：有 scope 时只留该模块的问题（架构级问题属全局，收敛视图下不混入）
  const all = collectIssues(map, t)
  const issues = scopeId ? all.filter((i) => i.moduleId === scopeId) : all

  return (
    <div className="space-y-5">
      <div>
        <h2 className="text-[15px] font-bold text-slate-800 dark:text-slate-100">{scopeName ? t('canvas.issues.titleScoped', { name: scopeName }) : t('canvas.issues.titleAll')}</h2>
        <p className="mt-1 text-[11px] text-slate-400 dark:text-slate-500">
          {scopeName ? t('canvas.issues.metaScoped', { count: issues.length }) : t('canvas.issues.metaAll', { count: issues.length })} · {t('canvas.issues.metaSort')}
        </p>
        {/* 2026-10-05 巡检新旧对照（全量视图专属；选区收敛下数据口径不同不显示——评审#Q2） */}
        {!scopeName && patrolDiff && (
          <p
            className="mt-1.5 flex flex-wrap items-center gap-x-2 text-[11px]"
            title={[
              ...patrolDiff.fixed.map((f) => t('canvas.issues.diffFixedTip', { finding: f.finding })),
              ...patrolDiff.new.map((n) => t('canvas.issues.diffNewTip', { finding: n.finding })),
              ...patrolDiff.moduleGone.map((m) => t('canvas.issues.diffGoneTip', { finding: m.finding })),
            ].join('\n') || undefined}
          >
            <span className="font-semibold text-slate-500 dark:text-slate-400">{t('canvas.issues.diffCompare')}</span>
            {patrolDiff.fixed.length > 0 && <span className="font-bold text-emerald-600">{t('canvas.issues.diffFixed', { count: patrolDiff.fixed.length })}</span>}
            {patrolDiff.new.length > 0 && <span className="font-bold text-amber-600">{t('canvas.issues.diffNew', { count: patrolDiff.new.length })}</span>}
            <span className="text-slate-400 dark:text-slate-500">{t('canvas.issues.diffPersisted', { count: patrolDiff.persisted })}</span>
            {patrolDiff.moduleGone.length > 0 && <span className="text-slate-400 dark:text-slate-500">{t('canvas.issues.diffGone', { count: patrolDiff.moduleGone.length })}</span>}
          </p>
        )}
      </div>

      {issues.length === 0 && (
        <p className="flex items-center gap-1.5 rounded-lg bg-emerald-50 dark:bg-emerald-950/40 px-3 py-2.5 text-[12px] text-emerald-700">
          <Info size={13} /> {t('canvas.issues.empty')}
        </p>
      )}

      <div className="space-y-2.5">
        {issues.map((issue) => (
          <div
            key={issue.key}
            className={`rounded-lg border p-3 transition-colors ${
              issue.moduleId ? 'cursor-pointer hover:border-blue-300 hover:bg-blue-50/40' : 'border-red-200 dark:border-red-900/60 bg-red-50/40 dark:bg-red-950/30'
            }`}
            style={issue.moduleId ? { borderColor: '#e2e8f0' } : undefined}
            onClick={() => issue.moduleId && onLocate(issue.moduleId!)}
          >
            <div className="flex items-center gap-2">
              <SeverityChip severity={issue.severity} />
              {issue.scope === 'arch' ? (
                <span className="text-[11px] font-bold text-red-600">{t('canvas.issues.scopeArch')}</span>
              ) : (
                <span className="flex items-center gap-1 text-[11px] font-semibold text-slate-600 dark:text-slate-300">
                  {issue.moduleName}
                  <span
                    className="h-1.5 w-1.5 rounded-full"
                    style={{ background: healthColor(map.modules.find((m) => m.id === issue.moduleId)!.health.score) }}
                  />
                </span>
              )}
              <span className="ml-auto text-micro text-slate-400 dark:text-slate-500">{t('canvas.issues.impact', { count: issue.impact })}</span>
            </div>

            <p className="mt-1.5 text-[12px] leading-5 text-slate-700 dark:text-slate-200">{issue.finding}</p>

            <p className="mt-1 flex items-start gap-1 text-[11px] leading-5 text-slate-500 dark:text-slate-400">
              <ArrowRight size={11} className="mt-1 shrink-0 text-emerald-600" />
              {issue.suggestion}
            </p>

            <div className="mt-1.5 flex items-center gap-2">
              {issue.moduleId && (
                <span className="flex items-center gap-1 text-micro font-medium text-blue-500">
                  <Crosshair size={10} /> {t('canvas.issues.locate')}
                </span>
              )}
              <button
                onClick={(e) => quickFix(e, issue)}
                disabled={quickBusy !== null || !backendRepo}
                className="flex items-center gap-1 rounded-full bg-emerald-600 px-2 py-0.5 text-micro font-bold text-white hover:bg-emerald-700 disabled:opacity-40"
                title={t('canvas.issues.quickTip')}
              >
                {quickBusy === issue.key ? <Loader2 size={9} className="animate-spin" /> : <Zap size={9} />}
                {quickDone === issue.key ? t('canvas.issues.quickDone') : t('canvas.issues.quick')}
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
                      title: tStatic('canvas.issues.taskTitle', { finding: issue.finding.slice(0, 24) }),
                      description: tStatic('canvas.issues.taskDesc', { finding: issue.finding, suggestion: issue.suggestion }),
                      modules: worst.map((m) => m.id),
                      acceptance: tStatic('canvas.issues.taskAcceptance'),
                      source: 'concern',
                      context: { inject: { archConcern: { severity: issue.severity, finding: issue.finding, suggestion: issue.suggestion }, modules: worst.map((m) => ({ id: m.id, name: m.name, score: m.health.score })) } },
                    })
                  }
                }}
                className="ml-auto flex items-center gap-1 rounded-full border border-blue-200 dark:border-blue-900/60 bg-blue-50 dark:bg-blue-950/40 px-2 py-0.5 text-micro font-bold text-blue-600 hover:bg-blue-100"
                title={t('canvas.issues.fixTip')}
              >
                <Wrench size={9} /> {t('canvas.issues.fix')}
              </button>
            </div>
          </div>
        ))}
      </div>

      <p className="flex items-start gap-1.5 text-cap leading-4 text-slate-400 dark:text-slate-500">
        <Info size={11} className="mt-0.5 shrink-0" />
        {t('canvas.issues.footnote')}
      </p>
    </div>
  )
}
