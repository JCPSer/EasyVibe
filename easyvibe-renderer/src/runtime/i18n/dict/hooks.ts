// i18n-shard: hooks
// 词表分片（纯数据、零运行时逻辑）——由 ../index.ts 聚合；取词契约见 ../index.ts。
// zh 为基准（as const）；en 由 Record<keyof typeof zh, string> 编译期对齐，key 集合与 zh 全等。

export const zh = {
  // hooks.patrol.*：顶栏巡检触发提示
  'hooks.patrol.queuedReplaced': '已加入队列：巡检将在当前会话结束后自动开始（已替换排队：{label}）',
  'hooks.patrol.queued': '已加入队列：巡检将在当前会话结束后自动开始',
  'hooks.patrol.started': '已直接开始巡检',
  'hooks.patrol.startFailed': '巡检启动失败（请确认后端在线后重试）。',
  // hooks.repo.*：仓库管理（useBackendConnection toast/prompt）
  'hooks.repo.pickTitle': '选择本地仓库目录',
  'hooks.repo.promptPath': '输入本地仓库目录的绝对路径',
  'hooks.repo.promptFallback': '目录选择器不可用，输入本地仓库目录的绝对路径',
  'hooks.repo.addFail': '添加失败（目录不可读或已挂载）',
  'hooks.repo.added': '已添加仓库，正在归纳…',
  'hooks.repo.addOffline': '添加失败（需要后端在线）',
  'hooks.repo.removeFail': '移除失败',
  'hooks.repo.removedWiped': '已移除仓库并清除其数据',
  'hooks.repo.removedKept': '已移除仓库（数据保留，重新添加后可见）',
  'hooks.repo.removeOffline': '移除失败（需要后端在线）',
  // hooks.agent.*：执行 agent 采用/安装（useAgentState toast）
  'hooks.agent.writeFail': '配置写入失败',
  'hooks.agent.adoptedOk': '已采用 {command}，协议兼容（{ms}ms）',
  'hooks.agent.adoptedFail': '已采用 {command}，但协议测试未通过：{protocol}',
  'hooks.agent.unknown': '未知',
  'hooks.agent.adoptFail': '采用失败',
  'hooks.agent.installCopied': '安装命令已复制——在终端粘贴执行，完成后回到这里点"重新探测"',
  // hooks.notify.*：系统通知标题（useSystemNotifications）
  'hooks.notify.sessionFailedTitle': 'EasyVibe · 会话失败',
  'hooks.notify.sessionFailedBody': '会话 {id} 执行失败——回来看看原因',
  'hooks.notify.queueStartedTitle': 'EasyVibe · 排队任务已开始',
} as const

export const en: Record<keyof typeof zh, string> = {
  'hooks.patrol.queuedReplaced': 'Queued: patrol starts automatically when the current session ends (replaced queue: {label})',
  'hooks.patrol.queued': 'Queued: patrol starts automatically when the current session ends',
  'hooks.patrol.started': 'Patrol started directly',
  'hooks.patrol.startFailed': 'Failed to start patrol — make sure the backend is online and retry.',
  // hooks.repo.*：仓库管理（useBackendConnection toast/prompt）
  'hooks.repo.pickTitle': 'Choose a local repository directory',
  'hooks.repo.promptPath': 'Enter the absolute path of the local repository directory',
  'hooks.repo.promptFallback': 'Directory picker unavailable — enter the absolute path of the local repository directory',
  'hooks.repo.addFail': 'Add failed (directory unreadable or already mounted)',
  'hooks.repo.added': 'Repository added, inducting…',
  'hooks.repo.addOffline': 'Add failed (backend must be online)',
  'hooks.repo.removeFail': 'Remove failed',
  'hooks.repo.removedWiped': 'Repository removed and its data cleared',
  'hooks.repo.removedKept': 'Repository removed (data kept, visible again after re-adding)',
  'hooks.repo.removeOffline': 'Remove failed (backend must be online)',
  // hooks.agent.*：执行 agent 采用/安装（useAgentState toast）
  'hooks.agent.writeFail': 'Failed to write configuration',
  'hooks.agent.adoptedOk': 'Adopted {command}, protocol compatible ({ms}ms)',
  'hooks.agent.adoptedFail': 'Adopted {command}, but protocol test failed: {protocol}',
  'hooks.agent.unknown': 'unknown',
  'hooks.agent.adoptFail': 'Adoption failed',
  'hooks.agent.installCopied': 'Install command copied — paste it in a terminal, then come back and click "Re-detect"',
  // hooks.notify.*：系统通知标题（useSystemNotifications）
  'hooks.notify.sessionFailedTitle': 'EasyVibe · Session Failed',
  'hooks.notify.sessionFailedBody': 'Session {id} failed — come back and check why',
  'hooks.notify.queueStartedTitle': 'EasyVibe · Queued Task Started',
}
