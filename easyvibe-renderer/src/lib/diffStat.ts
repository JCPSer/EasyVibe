// M4-3 变更记录/影响面的 diff --stat 文本解析（git diff --stat 输出 → 结构化数据）。
// 生产端：task_exec.rs git_change_summary（git diff --stat [base]）。
// 解析规则与 ReviewPage/WorkbenchPage 中的内联版本同口径，收拢为单一实现后可复用。

export interface DiffFileStat {
  path: string
  adds: number
  dels: number
}

export interface DiffStat {
  files: DiffFileStat[]
  /** 文件总数（优先取 git 汇总行；缺失时按解析到的文件行计数） */
  fileCount: number
  /** 新增行总数（优先取 git 汇总行 insertions） */
  insertions: number
  /** 删除行总数（优先取 git 汇总行 deletions） */
  deletions: number
}

/**
 * 解析 git diff --stat 文本。
 * 文件行：` path/to/file.ts | 12 ++++---`（+/- 号的个数即增删行数；二进制文件行无 +/-，跳过）
 * 汇总行：` 3 files changed, 42 insertions(+), 17 deletions(-)`
 */
export function parseDiffStat(text: string): DiffStat {
  const files: DiffFileStat[] = []
  let fileCount = 0
  let insertions = 0
  let deletions = 0
  let sawSummary = false

  for (const line of text.split('\n')) {
    const summary = line.match(/^\s*(\d+)\s+files?\s+changed(?:,\s*(\d+)\s+insertions?\(\+\))?(?:,\s*(\d+)\s+deletions?\(-\))?/)
    if (summary) {
      sawSummary = true
      fileCount = Number(summary[1])
      insertions = Number(summary[2] ?? 0)
      deletions = Number(summary[3] ?? 0)
      continue
    }
    const file = line.match(/^\s*(.+?)\s*\|\s*\d+\s*([+-]*)\s*$/)
    if (!file) continue
    const marks = file[2] ?? ''
    files.push({
      path: file[1].trim(),
      adds: (marks.match(/\+/g) ?? []).length,
      dels: (marks.match(/-/g) ?? []).length,
    })
  }

  return {
    files,
    fileCount: sawSummary ? fileCount : files.length,
    insertions: sawSummary ? insertions : files.reduce((s, f) => s + f.adds, 0),
    deletions: sawSummary ? deletions : files.reduce((s, f) => s + f.dels, 0),
  }
}

/** 单文件归属模块（glob 前缀匹配，带路径段边界；未命中返回 null）——
 *  R2 审查实锤：startsWith("src/core") 会放过 src/coreography/，前缀必须有边界。 */
export function moduleOfFile(
  path: string,
  modules: { id: string; name: string; files: string[] }[],
): { id: string; name: string } | null {
  const mod = modules.find((mm) =>
    mm.files.some((g) => {
      const base = g.replace(/\*\*.*$/, '').replace(/\/$/, '')
      return base !== '' && (path.startsWith(base + '/') || path.includes('/' + base + '/'))
    }),
  )
  return mod ? { id: mod.id, name: mod.name } : null
}

/** 把文件级 stat 按模块 files glob 聚合（与 WorkbenchPage 影响面同口径） */
export function aggregateByModule(
  files: DiffFileStat[],
  modules: { id: string; name: string; files: string[] }[],
): { id: string; name: string; adds: number; dels: number; fileCount: number }[] {
  const byModule = new Map<string, { id: string; name: string; adds: number; dels: number; fileCount: number }>()
  for (const f of files) {
    const mod = moduleOfFile(f.path, modules)
    const key = mod?.id ?? '_other'
    const cur = byModule.get(key) ?? { id: key, name: mod?.name ?? '未映射文件', adds: 0, dels: 0, fileCount: 0 }
    cur.adds += f.adds
    cur.dels += f.dels
    cur.fileCount += 1
    byModule.set(key, cur)
  }
  return [...byModule.values()].sort((a, b) => b.adds + b.dels - (a.adds + a.dels))
}

/** ISO 或 epoch 秒/毫秒 → 毫秒；无法解析返回 null（巡检 started_at / 任务时间戳均有 epoch 落库口径） */
export function toMs(iso: string): number | null {
  const t = Date.parse(iso)
  if (!Number.isNaN(t)) return t
  if (/^\d{9,13}$/.test(iso)) {
    const n = Number(iso)
    return n < 1e12 ? n * 1000 : n
  }
  return null
}

/** 相对时间："3 分钟前 / 2 小时前 / 5 天前"；无法解析返回原始串 */
export function relTime(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return '—'
  const t = toMs(iso)
  if (t === null) return iso
  const diff = Math.max(0, now - t)
  const min = Math.floor(diff / 60000)
  if (min < 1) return '刚刚'
  if (min < 60) return `${min} 分钟前`
  const hours = Math.floor(min / 60)
  if (hours < 24) return `${hours} 小时前`
  const days = Math.floor(hours / 24)
  if (days < 30) return `${days} 天前`
  const months = Math.floor(days / 30)
  return `${months} 个月前`
}

/** 绝对时间："2026-09-30 09:15"；无法解析返回原始串 */
export function absTime(iso: string | null | undefined): string {
  if (!iso) return '—'
  const t = toMs(iso)
  if (t === null) return iso
  const d = new Date(t)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`
}
