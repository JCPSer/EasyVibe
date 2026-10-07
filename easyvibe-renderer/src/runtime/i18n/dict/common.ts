// i18n-shard: common
// 词表分片（纯数据、零运行时逻辑）——由 ../index.ts 聚合；取词契约见 ../index.ts。
// zh 为基准（as const）；en 由 Record<keyof typeof zh, string> 编译期对齐，key 集合与 zh 全等。

export const zh = {
  // common.*：跨域通用
  'common.language': '语言',
  // common.*：跨页通用（第二批追加）
  'common.pickProject': '先在左侧选择一个项目。',
  'common.loading': '加载中…',
  'common.replacedFallback': '旧任务',
  'common.unknownReason': '未知原因',
  'common.status.succeeded': '成功',
  'common.status.failed': '失败',
  'common.status.running': '进行中',
  // common.*：第三批追加（跨域通用动作/空态/时长）
  'common.cancel': '取消',
  'common.confirm': '确认',
  'common.retry': '重试',
  'common.copy': '复制',
  'common.copied': '已复制',
  'common.copyFail': '复制失败（剪贴板不可用）',
  'common.needBackend': '需要本地后端在线',
  'common.planned': '规划中',
  'common.agentMissing': '未检测到执行 agent——先安装或在设置中配置',
  'common.sec': '{s} 秒',
  'common.minSec': '{m} 分钟 {s} 秒',
  'common.min': '{m} 分钟',
  'common.hourMin': '{h} 小时 {m} 分',
  'common.justNow': '刚刚',
} as const

export const en: Record<keyof typeof zh, string> = {
  'common.language': 'Language',
  'common.pickProject': 'Pick a project on the left first.',
  'common.loading': 'Loading…',
  'common.replacedFallback': 'previous job',
  'common.unknownReason': 'unknown reason',
  'common.status.succeeded': 'Succeeded',
  'common.status.failed': 'Failed',
  'common.status.running': 'Running',
  // common.*：第三批追加（跨域通用动作/空态/时长）
  'common.cancel': 'Cancel',
  'common.confirm': 'Confirm',
  'common.retry': 'Retry',
  'common.copy': 'Copy',
  'common.copied': 'Copied',
  'common.copyFail': 'Copy failed (clipboard unavailable)',
  'common.needBackend': 'Local backend must be online',
  'common.planned': 'Planned',
  'common.agentMissing': 'No execution agent detected — install one or configure it in Settings',
  'common.sec': '{s}s',
  'common.minSec': '{m}m {s}s',
  'common.min': '{m}m',
  'common.hourMin': '{h}h {m}m',
  'common.justNow': 'just now',
}
