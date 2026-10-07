// i18n-shard: deps
// 词表分片（纯数据、零运行时逻辑）——由 ../index.ts 聚合；取词契约见 ../index.ts。
// zh 为基准（as const）；en 由 Record<keyof typeof zh, string> 编译期对齐，key 集合与 zh 全等。

export const zh = {
  // deps.*：依赖体检（卡片文案经 shared/logic/depsAnalysis 的 tx 注入翻译，缺 key 回退中文）
  'deps.type.call': '函数调用',
  'deps.type.import': 'import',
  'deps.type.api': 'API 调用',
  'deps.type.event': '消息事件',
  'deps.type.db': '共享数据库',
  'deps.type.config': '配置依赖',
  'deps.refCount': '{count} 处引用',
  'deps.card.violation.title': '「{from}」反向调用了「{to}」',
  'deps.card.violation.evidence': '{type} · {count} 处引用 · 违反「{layerFrom} → {layerTo}」分层',
  'deps.card.violation.consequence': '改「{to}」时「{from}」被一起拖着改，下层无法独立替换、独立测试。',
  'deps.card.cycle.title': '循环群：{count} 个模块互相可达{singles}',
  'deps.card.cycle.singles': '，全仓仅「{names}」独善其身',
  'deps.card.cycle.evidence': '闭环由 {count} 条逆向依赖互相打通；直连跨度越大，环越难拆。',
  'deps.card.cycle.consequence': '发布与测试互相绑架，任何一环改动都可能波及全链。',
  'deps.card.highRisk.title': '「{name}」（{score} 分）被 {count} 条强耦合依赖——腐化在传染',
  'deps.card.highRisk.evidence': 'strong 依赖来自：{froms}',
  'deps.card.highRisk.consequence': '它的腐化会顺着强耦合传给 {count} 个调用方。',
} as const

export const en: Record<keyof typeof zh, string> = {
  'deps.type.call': 'function call',
  'deps.type.import': 'import',
  'deps.type.api': 'API call',
  'deps.type.event': 'message event',
  'deps.type.db': 'shared database',
  'deps.type.config': 'config dependency',
  'deps.refCount': '{count} references',
  'deps.card.violation.title': '"{from}" calls upward into "{to}"',
  'deps.card.violation.evidence': '{type} · {count} references · violates "{layerFrom} → {layerTo}" layering',
  'deps.card.violation.consequence': 'Changing "{to}" drags "{from}" along — the lower layer can’t be replaced or tested independently.',
  'deps.card.cycle.title': 'Cycle: {count} modules mutually reachable{singles}',
  'deps.card.cycle.singles': ' — only "{names}" stays out of it',
  'deps.card.cycle.evidence': 'The loop is closed by {count} reverse dependencies; the wider the direct spans, the harder to break.',
  'deps.card.cycle.consequence': 'Releases and tests hold each other hostage — a change anywhere can ripple through the whole chain.',
  'deps.card.highRisk.title': '"{name}" ({score} pts) is under {count} strong couplings — decay is spreading',
  'deps.card.highRisk.evidence': 'Strong dependencies from: {froms}',
  'deps.card.highRisk.consequence': 'Its decay propagates to {count} callers through the strong couplings.',
}
