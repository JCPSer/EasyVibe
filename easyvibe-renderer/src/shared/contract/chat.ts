// 对话上下文契约（原定义在 components/DetailPanel.tsx）。
// v0.2：对话页签占位/详情视图「就此对话」的上抛回调——App 转换为跨页携带上下文跳「任务对话」。
export type ChatAboutTarget = { refId: string; refName: string; kind: 'module' | 'layer' }
