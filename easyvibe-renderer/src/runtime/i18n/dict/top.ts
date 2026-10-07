// i18n-shard: top
// 词表分片（纯数据、零运行时逻辑）——由 ../index.ts 聚合；取词契约见 ../index.ts。
// zh 为基准（as const）；en 由 Record<keyof typeof zh, string> 编译期对齐，key 集合与 zh 全等。

export const zh = {
  // top.*：顶栏中区仓库切换器（TopCenter）
  'top.offlineBtnTip': '后端不在线：当前为演示数据，点击看详情',
  'top.switchTip': '切换/管理仓库',
  'top.demoOffline': '演示数据 · 后端离线',
  'top.connecting': '连接后端中…',
  'top.noRepo': '未选择仓库',
  'top.offlineTitle': '后端不在线',
  'top.offlineBody': '当前画布是内置演示数据（hover-client）。归纳 / 巡检 / 任务 / 仓库管理都需要本地后端在线。',
  'top.offlineHint': '应用启动后后端在冷加载？每 5 秒自动重连，恢复后此面板自动可用。',
  'top.mounted': '已挂载仓库',
  'top.removeTip': '移除仓库…',
  'top.removeQ': '正在运行的任务/归纳会被终止。本地数据怎么处理？',
  'top.removeKeep': '移除，保留数据',
  'top.removeKeepTip': '任务/会话/巡检历史留在本地库，重新添加仓库后可见',
  'top.removeWipe': '移除并清除数据',
  'top.removeWipeTip': '抹掉该仓库的任务/会话/审批/巡检历史/事件/仓库级设置（不可恢复）',
  'top.emptyRepos': '尚未挂载任何仓库',
  'top.openRepo': '打开本地仓库…',
} as const

export const en: Record<keyof typeof zh, string> = {
  // top.*：顶栏中区仓库切换器（TopCenter）
  'top.offlineBtnTip': 'Backend offline: currently demo data — click for details',
  'top.switchTip': 'Switch / manage repositories',
  'top.demoOffline': 'Demo Data · Backend Offline',
  'top.connecting': 'Connecting to backend…',
  'top.noRepo': 'No repository selected',
  'top.offlineTitle': 'Backend Offline',
  'top.offlineBody': 'The current canvas is built-in demo data (hover-client). Induction / patrol / tasks / repository management all require the local backend to be online.',
  'top.offlineHint': 'Is the backend cold-starting after app launch? It reconnects automatically every 5 seconds and this panel becomes usable again.',
  'top.mounted': 'Mounted Repositories',
  'top.removeTip': 'Remove repository…',
  'top.removeQ': 'Running tasks/inductions will be terminated. How should local data be handled?',
  'top.removeKeep': 'Remove, keep data',
  'top.removeKeepTip': 'Task/session/patrol history stays in the local database and is visible again after re-adding the repository',
  'top.removeWipe': 'Remove and clear data',
  'top.removeWipeTip': 'Erase this repository tasks/sessions/approvals/patrol history/events/repo-level settings (cannot be undone)',
  'top.emptyRepos': 'No repositories mounted yet',
  'top.openRepo': 'Open Local Repository…',
}
