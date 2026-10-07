// i18n-shard: git
// 词表分片（纯数据、零运行时逻辑）——由 ../index.ts 聚合；取词契约见 ../index.ts。
// zh 为基准（as const）；en 由 Record<keyof typeof zh, string> 编译期对齐，key 集合与 zh 全等。

export const zh = {
  // git.diff*：Git 页文件差异抽屉（变更区点击文件滑出）
  'git.diff.title': '文件差异',
  'git.diff.closeTip': '关闭（Esc）',
  'git.diff.loading': '正在加载差异…',
  'git.diff.loadFailed': '差异加载失败',
  'git.diff.retry': '重试',
  'git.diff.empty': '没有可显示的差异（改动可能已暂存、已提交或已撤销）',
  'git.diff.binary': '二进制文件，无法显示差异',
  'git.diff.untracked': '新文件（未跟踪）· 全部内容为新增',
  'git.diff.truncated': '差异过大，仅显示前 {max} 行（共 {total} 行）',
  'git.diff.baseWorktree': '工作区 vs 暂存区',
  'git.diff.baseStaged': '暂存区 vs HEAD',
  'git.diff.viewTip': '查看文件差异',
  'git.diff.stagedHint': '暂存区差异暂未支持，当前显示工作区未暂存差异',
  'git.diff.lineCount': '{count} 行',
} as const

export const en: Record<keyof typeof zh, string> = {
  'git.diff.title': 'File Diff',
  'git.diff.closeTip': 'Close (Esc)',
  'git.diff.loading': 'Loading diff…',
  'git.diff.loadFailed': 'Failed to load diff',
  'git.diff.retry': 'Retry',
  'git.diff.empty': 'No diff to show (changes may be staged, committed, or discarded)',
  'git.diff.binary': 'Binary file — diff cannot be displayed',
  'git.diff.untracked': 'New file (untracked) · all lines are additions',
  'git.diff.truncated': 'Diff too large — showing first {max} of {total} lines',
  'git.diff.baseWorktree': 'worktree vs index',
  'git.diff.baseStaged': 'staged vs HEAD',
  'git.diff.viewTip': 'View file diff',
  'git.diff.stagedHint': 'Staged diff not supported yet — showing unstaged worktree diff',
  'git.diff.lineCount': '{count} lines',
}
