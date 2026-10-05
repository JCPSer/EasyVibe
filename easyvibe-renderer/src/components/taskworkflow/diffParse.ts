// TaskWorkflowPage 拆分产物：diff/影响面/时长 纯函数（可单测，无 React 依赖）。
// 拆自 TaskWorkflowPage.tsx（2026-10-05 防膨胀）。
import { toMs } from '@/lib/diffStat'
import type { TaskItem } from './types'

/** diff 全文 → 变更文件列表（`diff --git a/x b/x` 与 `+++ b/x` 双形态） */
export function splitDiffFiles(diffFull: string | null): string[] {
  if (!diffFull) return []
  const list: string[] = []
  for (const line of diffFull.split('\n')) {
    const m = line.match(/^diff --git a\/(.+?) b\//)
    if (m) list.push(m[1])
    else if (line.startsWith('+++ b/')) list.push(line.slice(6).trim())
  }
  return [...new Set(list)]
}

/** 选中文件 → 该文件的 diff 分块（无选中/无命中回退全文） */
export function pickActiveDiff(diffFull: string | null, activeFile: string | null): string | null {
  if (!diffFull) return null
  if (!activeFile) return diffFull
  const chunks = diffFull.split(/(?=^diff --git )/m)
  return chunks.find((c) => c.includes(` a/${activeFile} `) || c.includes(` b/${activeFile}`)) ?? diffFull
}

/** diff --stat 摘要 → 影响面行（adds/dels/files） */
export function parseImpact(diffStat: string | null): { adds: number; dels: number; files: number }[] {
  if (!diffStat) return []
  let adds = 0
  let dels = 0
  let n = 0
  for (const line of diffStat.split('\n')) {
    const m = line.match(/^\s*.+?\s*\|\s*\d+\s*([+-]*)\s*$/)
    if (!m) continue
    n += 1
    adds += (m[1].match(/\+/g) ?? []).length
    dels += (m[1].match(/-/g) ?? []).length
  }
  return [{ adds, dels, files: n }]
}

/** 任务运行时长（createdAt→updatedAt 人类可读） */
export function taskDuration(t: TaskItem): string {
  const a = toMs(t.createdAt ?? '')
  const b = toMs(t.updatedAt ?? '')
  if (!a || !b) return '—'
  const min = Math.floor(Math.max(0, b - a) / 60000)
  if (min < 1) return '刚刚'
  if (min < 60) return `${min} 分钟`
  return `${Math.floor(min / 60)} 小时 ${min % 60} 分`
}
