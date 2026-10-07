import { useMemo } from 'react'
import { Sparkles } from 'lucide-react'
import type { GitFile, TaskLite } from './types'

/** 改动来源 insight 条：任务归因 / 巡检写回 / 手动（未归因） */
export function SourceStrip({
  files,
  attribOf,
  onOpenChanges,
}: {
  files: GitFile[]
  attribOf: (path: string) => { kind: 'task'; task: TaskLite } | { kind: 'ev' } | { kind: 'manual' }
  onOpenChanges: () => void
}) {
  const src = useMemo(() => {
    const task = new Map<string, { task: TaskLite; n: number }>()
    let ev = 0
    let manual = 0
    for (const f of files) {
      const a = attribOf(f.path)
      if (a.kind === 'task') {
        const cur = task.get(a.task.id) ?? { task: a.task, n: 0 }
        cur.n += 1
        task.set(a.task.id, cur)
      } else if (a.kind === 'ev') ev += 1
      else manual += 1
    }
    return { tasks: [...task.values()].sort((a, b) => b.n - a.n), ev, manual }
  }, [files, attribOf])

  if (src.tasks.length === 0 && src.ev === 0) return null

  return (
    <div className="mx-4 mt-3 flex flex-wrap items-center gap-x-2 gap-y-1 rounded-xl border border-violet-100 bg-violet-50/50 px-3.5 py-2 text-cap text-violet-700">
      <Sparkles size={11} className="shrink-0" />
      <span className="font-semibold">改动来源：</span>
      {src.tasks.slice(0, 2).map(({ task, n }) => (
        <span key={task.id}>
          <b>任务「{task.title.slice(0, 12)}」</b>（{n} 个文件）
        </span>
      ))}
      {src.ev > 0 && <span>巡检健康写回（{src.ev} 个）</span>}
      {src.manual > 0 && <span className="text-violet-400">手动修改（{src.manual} 个 · 未归因）</span>}
      <button onClick={onOpenChanges} className="ml-auto font-semibold text-blue-600 hover:underline">
        查看任务 →
      </button>
    </div>
  )
}
