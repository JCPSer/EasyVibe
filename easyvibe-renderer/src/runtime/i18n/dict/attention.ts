// i18n-shard: attention
// 词表分片（纯数据、零运行时逻辑）——由 ../index.ts 聚合；取词契约见 ../index.ts。
// zh 为基准（as const）；en 由 Record<keyof typeof zh, string> 编译期对齐，key 集合与 zh 全等。

export const zh = {
  // attention.*：应用壳注意力条（AttentionBar 三态）
  'attention.noRepoTitle': '尚未打开任何仓库——当前画布是演示数据',
  'attention.noRepoSub': '归纳 / 巡检 / 任务 / 对话都需要一个本地代码仓库',
  'attention.noRepoCta': '选择仓库 →',
  'attention.detectedTitle': '检测到 {cmd} 已安装',
  'attention.detectedSub': '采用后即可开始归纳 / 任务',
  'attention.adopt': '采用 {cmd} →',
  'attention.goSettings': '去设置',
  'attention.missingTitle': '未检测到执行 agent',
  'attention.missingSub': '归纳 / 巡检 / 任务需要本地 CLI agent（支持 claude / codex / opencode）',
  'attention.copyInstall': '复制 claude 安装命令',
  'attention.learnMore': '了解更多',
  'attention.approvalsTitle': '{n} 项任务等你审批',
  'attention.goHandle': '去处理 →',
} as const

export const en: Record<keyof typeof zh, string> = {
  // attention.*：应用壳注意力条（AttentionBar 三态）
  'attention.noRepoTitle': 'No repository opened yet — the current canvas is demo data',
  'attention.noRepoSub': 'Induction / patrol / tasks / chat all need a local code repository',
  'attention.noRepoCta': 'Choose Repository →',
  'attention.detectedTitle': 'Detected {cmd} installed',
  'attention.detectedSub': 'Adopt it to start induction / tasks',
  'attention.adopt': 'Adopt {cmd} →',
  'attention.goSettings': 'Settings',
  'attention.missingTitle': 'No execution agent detected',
  'attention.missingSub': 'Induction / patrol / tasks need a local CLI agent (claude / codex / opencode supported)',
  'attention.copyInstall': 'Copy claude install command',
  'attention.learnMore': 'Learn more',
  'attention.approvalsTitle': '{n} tasks awaiting your approval',
  'attention.goHandle': 'Handle →',
}
