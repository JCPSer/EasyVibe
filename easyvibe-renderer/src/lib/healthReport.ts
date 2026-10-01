// D8：健康报告导出——把当前地图的健康评估拼装为 Markdown 并触发浏览器下载。
// 纯前端实现：数据全部来自 map.json（健康度/concerns/decay_flags 均为巡检产物），
// 不额外打后端接口，零token成本，随时可导出。
import type { CodeMap, Module } from '@/types/map'

const CN = { low: '低', medium: '中', high: '高', critical: '严重' } as const

function layerName(map: CodeMap, layerId: string): string {
  return map.layers.find((l) => l.id === layerId)?.name ?? layerId
}

function moduleRow(m: Module, map: CodeMap): string {
  const flags = m.health.decay_flags.length ? m.health.decay_flags.join(', ') : '—'
  return `| ${m.name} | ${layerName(map, m.layer)} | ${m.health.score} | ${CN[m.health.coupling]} | ${CN[m.health.complexity]} | ${flags} |`
}

export function buildHealthReport(map: CodeMap): string {
  const h = map.health
  const lines: string[] = []
  lines.push(`# 架构健康报告 · ${map.meta.repo}`)
  lines.push('')
  lines.push(`- 地图生成时间：${map.meta.generated_at}`)
  if (map.meta.last_patrol_at) lines.push(`- 上次巡检：${map.meta.last_patrol_at}`)
  if (map.meta.stats) {
    const s = map.meta.stats
    lines.push(
      `- 文件覆盖：${s.files_covered ?? '—'} / ${s.files_total ?? '—'}（${s.coverage_ratio != null ? Math.round(s.coverage_ratio * 100) + '%' : '—'}）`,
    )
  }
  lines.push('')

  lines.push('## 架构健康（独立评估，模块全绿 ≠ 架构健康）')
  lines.push('')
  lines.push(`- **综合健康分：${h.score}**`)
  lines.push(`- 耦合度：${CN[h.coupling]}　复杂度：${CN[h.complexity]}${h.churn ? `　变更频率：${CN[h.churn]}` : ''}`)
  if (h.decay_flags.length) lines.push(`- 腐化标记：${h.decay_flags.join(', ')}`)
  if (h.review_note) lines.push(`- 总评：${h.review_note}`)
  if (h.concerns?.length) {
    lines.push('')
    lines.push('### 架构级重点关注')
    for (const c of h.concerns) {
      lines.push(`- **[${c.severity === 'critical' ? '严重' : '高'}]** ${c.finding}`)
      lines.push(`  - 建议：${c.suggestion}`)
    }
  }
  lines.push('')

  const sorted = [...map.modules].sort((a, b) => a.health.score - b.health.score)
  lines.push('## 模块健康一览（按分数升序）')
  lines.push('')
  lines.push('| 模块 | 层 | 分数 | 耦合 | 复杂度 | 腐化标记 |')
  lines.push('| --- | --- | --- | --- | --- | --- |')
  for (const m of sorted) lines.push(moduleRow(m, map))
  lines.push('')

  const withConcerns = sorted.filter((m) => m.health.concerns?.length)
  if (withConcerns.length) {
    lines.push('## 模块级问题与建议')
    lines.push('')
    for (const m of withConcerns) {
      lines.push(`### ${m.name}（${m.health.score} 分）`)
      for (const c of m.health.concerns!) {
        lines.push(`- **[${c.severity === 'critical' ? '严重' : '高'}]** ${c.finding}`)
        lines.push(`  - 建议：${c.suggestion}`)
      }
      lines.push('')
    }
  }

  const violations = map.edges.filter((e) => e.direction_violation)
  if (violations.length) {
    lines.push('## 逆向依赖（分层违规信号）')
    lines.push('')
    for (const e of violations) {
      const from = map.modules.find((m) => m.id === e.from)?.name ?? e.from
      const to = map.modules.find((m) => m.id === e.to)?.name ?? e.to
      lines.push(`- ${from} → ${to}（${e.type}${e.label ? `：${e.label}` : ''}）`)
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
