// 新手引导全部文案（2026-10-04）
// i18n 债（调研 C4）：本期文案集中于此文件（不 hardcode 进组件），下期换 i18n 框架时
// 本文件整体替换为语言包加载即可。德语/俄语等长文本预留 40% 长度冗余（卡片文案保持短句）。

export const ONBOARDING_COPY = {
  welcome: {
    kicker: '欢迎使用',
    title: '让 Agent 写代码，让架构留在你手里',
    subtitle:
      'EasyVibe 从你的真实代码库归纳出架构地图，AI 的每次改动经审批门禁落地、全程留痕。',
    ctaPrimary: '添加我的仓库',
    ctaPrimaryHint: '选择本地代码目录，开始归纳架构地图（通常数分钟）',
    ctaSecondary: '先逛逛演示数据',
    ctaSecondaryHint: '稍后可从顶栏项目切换器随时添加仓库',
    reopenNote: '本引导可随时从顶栏「?」重新打开',
  },
  /** 概念卡片：欢迎页展示 + 归纳等待期轮播（同一份事实源） */
  concepts: [
    {
      id: 'map',
      title: '语义代码地图',
      body: 'LLM 把代码库翻译成架构视图：模块、职责、依赖一目了然，存在你的仓库里（.easyvibe/map），git 可见、随时重新归纳。',
    },
    {
      id: 'health',
      title: '架构健康审计',
      body: '模块级 + 架构级两级健康评估：耦合、复杂度、腐化标记，巡检自动对比上轮基线，漂移无处可藏。',
    },
    {
      id: 'task',
      title: '对话孵化任务',
      body: '在「任务对话」里描述需求，AI 走 harness 五阶段流程：需求矩阵 → 方案设计 → 开发 → 代码审查，全程可见。',
    },
    {
      id: 'gate',
      title: '审批门禁与留痕',
      body: '每个变更经你批准才落地（计划/Diff/审查报告三道关），全部产物归档进仓库，事后可回溯、可审计。',
    },
  ] as const,
  waiting: {
    ideaTitle: '等待时不白等',
    ideaPlaceholder: '把你想做的第一个任务/改进想法写在这里…',
    ideaSaved: '已保存——归纳完成后自动带入「任务对话」',
    ideaButton: '保存想法',
  },
  checklist: {
    title: '上手指引',
    doneToast: '🎉 上手指引全部完成，开始享受 EasyVibe',
    items: {
      addRepo: { label: '添加你的第一个仓库', hint: '顶栏项目切换器 → 添加本地目录' },
      viewMap: { label: '查看生成的架构地图', hint: '点开任意模块看详情与健康度' },
      viewHealth: { label: '看看健康看板或漂移洞察', hint: '左侧导航「探索」组' },
      firstTask: { label: '孵化第一个任务', hint: '「任务对话」里描述需求，或地图上点「发起修复」' },
      firstApproval: { label: '完成一次审批', hint: '任务走到审批关时，在流水线里通过或驳回' },
    } as const,
  },
  /** 对话空态示例 prompt（AionUi 式"活的引导"：点一下即进入真实工作流） */
  samplePrompts: {
    generic: [
      '帮我梳理这个仓库的架构分层是否合理，有没有逆向依赖？',
      '哪些模块的健康度最差？按优先级给出治理建议。',
    ],
    withModule: '请评估一下「{module}」模块的健康度并给出改进建议。',
  },
}
