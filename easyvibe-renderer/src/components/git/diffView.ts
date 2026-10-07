// Git 差异抽屉的纯渲染逻辑：统一 diff 文本 → 行分类（零依赖，可单测）。
// 分类口径对齐后端 easyvibe_git::diff 输出（含未跟踪文件合成 diff）。

export type DiffLineKind = 'file' | 'hunk' | 'add' | 'del' | 'context' | 'meta'

export interface DiffLine {
  kind: DiffLineKind
  text: string
}

/** 逐行分类：文件头弱化、hunk 头弱化、新增绿、删除红、其余为上下文。 */
export function classifyDiffLine(line: string): DiffLineKind {
  if (
    line.startsWith('diff --git') ||
    line.startsWith('index ') ||
    line.startsWith('--- ') ||
    line.startsWith('+++ ') ||
    line.startsWith('similarity ') ||
    line.startsWith('rename ') ||
    line.startsWith('old mode') ||
    line.startsWith('new mode')
  ) {
    return 'file'
  }
  if (line.startsWith('@@')) return 'hunk'
  if (line.startsWith('+')) return 'add'
  if (line.startsWith('-')) return 'del'
  if (line.startsWith('\\')) return 'meta' // "\ No newline at end of file"
  return 'context'
}

/** 统一 diff 文本 → 分类行序列；容忍尾部换行产生的空末行。 */
export function parseDiffText(text: string): DiffLine[] {
  const lines = text.split('\n')
  // 去掉末尾空元素（text 以 \n 结尾时 split 产生）；行内空行保留（上下文行可能是单空格）
  if (lines.length > 0 && lines[lines.length - 1] === '') lines.pop()
  return lines.map((l) => ({ kind: classifyDiffLine(l), text: l }))
}

/** 统计 +/- 行数（抽屉页脚 "+a −d"）。 */
export function countDiffStats(lines: DiffLine[]): { adds: number; dels: number } {
  let adds = 0
  let dels = 0
  for (const l of lines) {
    if (l.kind === 'add') adds += 1
    else if (l.kind === 'del') dels += 1
  }
  return { adds, dels }
}
