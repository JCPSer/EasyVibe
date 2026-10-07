// i18n-shard: kinds,gate,kb,time,app,updater,runtime
// 词表分片（纯数据、零运行时逻辑）——由 ../index.ts 聚合；取词契约见 ../index.ts。
// zh 为基准（as const）；en 由 Record<keyof typeof zh, string> 编译期对齐，key 集合与 zh 全等。

export const zh = {
  // kinds.*：会话/任务类型名（运行页、用量页、治理账单共用）
  'kinds.induce': '归纳',
  'kinds.patrol': '巡检',
  'kinds.submap': '子图分析',
  'kinds.task': '任务执行',
  'kinds.subagent-review': '任务执行 · 初审',
  'kinds.subagent-audit': '任务执行 · 审查',
  'kinds.unknown': '其他',
  // time.*：相对时间（shared/logic/diffStat relTime 经 tx 注入翻译）
  'time.justNow': '刚刚',
  'time.minutesAgo': '{count} 分钟前',
  'time.hoursAgo': '{count} 小时前',
  'time.daysAgo': '{count} 天前',
  'time.monthsAgo': '{count} 个月前',
  // runtime.queue.*：sessionQueue 入队失败提示（runtime 内部自译）
  'runtime.queue.enqueueFailed': '加入队列失败：{reason}',
  'runtime.queue.enqueueOffline': '加入队列失败（请确认后端在线后重试）。',
  // kb.*：知识库三页占位（routes.tsx）
  'kb.docs.title': '文档中心',
  'kb.docs.desc': '知识库三页为 P3 骨架：从已定样式模式派生。',
  'kb.decisions.title': '决策记录',
  'kb.decisions.desc': '这个仓库做过的重要技术决策及其来龙去脉。',
  'kb.apis.title': '接口目录',
  'kb.apis.desc': '全部关键入口（路由/函数/任务）的索引：谁对外提供什么能力。',
  // app.*：应用根（App.tsx 全屏错误/加载与 WS 版本提示）
  'app.staticLoadFail': '静态数据加载失败（/data/map.json）：{err}',
  'app.loadingMap': '正在加载代码地图…',
  'app.backendUpdated': '后端已更新（{prev} → {v}），刷新页面以加载新界面',
  // gate.*：地图守门员（MapGate / InductionWaiting 骨架文案）
  'gate.probing': '正在探测仓库状态…',
  'gate.induceFail': '归纳发起失败',
  'gate.induceFailOffline': '归纳发起失败（需要后端在线）',
  'gate.loadFail': '代码地图加载失败',
  'gate.noMap': '该仓库尚未生成代码地图',
  'gate.noMapHint': '发起归纳后，EasyVibe 的 agent 会扫描仓库并生成架构地图（通常数分钟）',
  'gate.start': '开始归纳',
  // updater.*：桌面壳自动更新提示
  'updater.ready': '新版本 {v} 已就绪',
  'updater.relaunch': '重启更新',
} as const

export const en: Record<keyof typeof zh, string> = {
  'kinds.induce': 'Induction',
  'kinds.patrol': 'Patrol',
  'kinds.submap': 'Submap analysis',
  'kinds.task': 'Task run',
  'kinds.subagent-review': 'Task run · Review',
  'kinds.subagent-audit': 'Task run · Audit',
  'kinds.unknown': 'Other',
  'time.justNow': 'just now',
  'time.minutesAgo': '{count} min ago',
  'time.hoursAgo': '{count} hr ago',
  'time.daysAgo': '{count} d ago',
  'time.monthsAgo': '{count} mo ago',
  'runtime.queue.enqueueFailed': 'Failed to enqueue: {reason}',
  'runtime.queue.enqueueOffline': 'Failed to enqueue — make sure the backend is online and retry.',
  // kb.*：知识库三页占位（routes.tsx）
  'kb.docs.title': 'Docs',
  'kb.docs.desc': 'The three knowledge-base pages are a P3 skeleton: derived from established style patterns.',
  'kb.decisions.title': 'Decisions',
  'kb.decisions.desc': 'Important technical decisions made in this repo and the reasoning behind them.',
  'kb.apis.title': 'API Catalog',
  'kb.apis.desc': 'An index of all key entry points (routes/functions/tasks): who provides what capability.',
  // app.*：应用根（App.tsx 全屏错误/加载与 WS 版本提示）
  'app.staticLoadFail': 'Failed to load static data (/data/map.json): {err}',
  'app.loadingMap': 'Loading code map…',
  'app.backendUpdated': 'Backend updated ({prev} → {v}) — refresh the page to load the new UI',
  // gate.*：地图守门员（MapGate / InductionWaiting 骨架文案）
  'gate.probing': 'Probing repository state…',
  'gate.induceFail': 'Failed to start induction',
  'gate.induceFailOffline': 'Failed to start induction (backend must be online)',
  'gate.loadFail': 'Failed to load the code map',
  'gate.noMap': 'No code map has been generated for this repository',
  'gate.noMapHint': 'After induction starts, the EasyVibe agent scans the repository and generates the architecture map (usually a few minutes)',
  'gate.start': 'Start Induction',
  // updater.*：桌面壳自动更新提示
  'updater.ready': 'New version {v} is ready',
  'updater.relaunch': 'Restart to Update',
}
