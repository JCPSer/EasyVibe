// D8：健康报告导出——把当前地图的健康评估拼装为 Markdown 并触发浏览器下载。
// 纯前端实现：数据全部来自 map.json（健康度/concerns/decay_flags 均为巡检产物），
// 不额外打后端接口，零token成本，随时可导出。
// i18n 第三批：报告文案用户可见（导出文件内容），经模块级 t 自译（同 canvas.induce 先例）。
import { t } from '@/runtime/i18n'
import type { CodeMap, Module } from '@/types/map'

function layerName(map: CodeMap, layerId: string): string {
  return map.layers.find((l) => l.id === layerId)?.name ?? layerId
}

function moduleRow(m: Module, map: CodeMap): string {
  const flags = m.health.decay_flags.length ? m.health.decay_flags.join(', ') : '—'
  return `| ${m.name} | ${layerName(map, m.layer)} | ${m.health.score} | ${t(`report.level.${m.health.coupling}`)} | ${t(`report.level.${m.health.complexity}`)} | ${flags} |`
}

export function buildHealthReport(map: CodeMap): string {
  const h = map.health
  const lines: string[] = []
  lines.push(t('report.title', { repo: map.meta.repo }))
  lines.push('')
  lines.push(t('report.generatedAt', { t: map.meta.generated_at }))
  if (map.meta.last_patrol_at) lines.push(t('report.lastPatrol', { t: map.meta.last_patrol_at }))
  if (map.meta.stats) {
    const s = map.meta.stats
    lines.push(
      t('report.coverage', {
        covered: s.files_covered ?? '—',
        total: s.files_total ?? '—',
        pct: s.coverage_ratio != null ? Math.round(s.coverage_ratio * 100) + '%' : '—',
      }),
    )
  }
  lines.push('')

  lines.push(t('report.archTitle'))
  lines.push('')
  lines.push(t('report.score', { n: h.score }))
  lines.push(
    t('report.couplingLine', {
      c: t(`report.level.${h.coupling}`),
      x: t(`report.level.${h.complexity}`),
      churnPart: h.churn ? t('report.churnPart', { churn: t(`report.level.${h.churn}`) }) : '',
    }),
  )
  if (h.decay_flags.length) lines.push(`- ${t('report.decayFlags', { flags: h.decay_flags.join(', ') })}`)
  if (h.review_note) lines.push(`- ${t('report.reviewNote', { note: h.review_note })}`)
  if (h.concerns?.length) {
    lines.push('')
    lines.push(t('report.archConcerns'))
    for (const c of h.concerns) {
      lines.push(`- **[${c.severity === 'critical' ? t('report.severityCritical') : t('report.severityHigh')}]** ${c.finding}`)
      lines.push(`  - ${t('report.suggestion', { s: c.suggestion })}`)
    }
  }
  lines.push('')

  const sorted = [...map.modules].sort((a, b) => a.health.score - b.health.score)
  lines.push(t('report.modulesTitle'))
  lines.push('')
  lines.push(t('report.tableHeader'))
  lines.push('| --- | --- | --- | --- | --- | --- |')
  for (const m of sorted) lines.push(moduleRow(m, map))
  lines.push('')

  const withConcerns = sorted.filter((m) => m.health.concerns?.length)
  if (withConcerns.length) {
    lines.push(t('report.moduleConcernsTitle'))
    lines.push('')
    for (const m of withConcerns) {
      lines.push(t('report.moduleScore', { name: m.name, score: m.health.score }))
      for (const c of m.health.concerns!) {
        lines.push(`- **[${c.severity === 'critical' ? t('report.severityCritical') : t('report.severityHigh')}]** ${c.finding}`)
        lines.push(`  - ${t('report.suggestion', { s: c.suggestion })}`)
      }
      lines.push('')
    }
  }

  const violations = map.edges.filter((e) => e.direction_violation)
  if (violations.length) {
    lines.push(t('report.violationsTitle'))
    lines.push('')
    for (const e of violations) {
      const from = map.modules.find((m) => m.id === e.from)?.name ?? e.from
      const to = map.modules.find((m) => m.id === e.to)?.name ?? e.to
      lines.push(
        t('report.violationLine', {
          from,
          to,
          type: e.type,
          labelPart: e.label ? t('report.labelPart', { label: e.label }) : '',
        }),
      )
    }
    lines.push('')
  }

  return lines.join('\n')
}

export function downloadHealthReport(map: CodeMap): void {
  const md = buildHealthReport(map)
  const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-')
  const blob = new Blob([md], { type: 'text/markdown;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `easyvibe-health-${map.meta.repo}-${stamp}.md`
  a.click()
  URL.revokeObjectURL(url)
}
