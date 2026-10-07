// 新手引导全部文案（2026-10-04；第三批 i18n 改造为 tx 注入模式）。
// shared 层是 archGuard 叶子，不得 import runtime——沿用「可选 tx 函数注入 + 中文兜底」
// 模式（同 shared/logic/inductionProgress.ts 先例）：调用方（组件/hook）把 useLang 的 t
// 注进来，缺 key 或注入缺失时逐条回退中文词表，行为与改造前一致。
// 德语/俄语等长文本预留 40% 长度冗余（卡片文案保持短句）。

export type OnboardingTx = (key: string, vars?: Record<string, string | number>) => string

export interface OnboardingCopy {
  welcome: {
    kicker: string
    title: string
    subtitle: string
    ctaPrimary: string
    ctaPrimaryHint: string
    ctaSecondary: string
    ctaSecondaryHint: string
    reopenNote: string
  }
  /** 概念卡片：欢迎页展示 + 归纳等待期轮播（同一份事实源） */
  concepts: readonly { id: string; title: string; body: string }[]
  waiting: { ideaTitle: string; ideaPlaceholder: string; ideaSaved: string; ideaButton: string }
  checklist: {
    title: string
    doneToast: string
    items: {
      addRepo: { label: string; hint: string }
      viewMap: { label: string; hint: string }
      viewHealth: { label: string; hint: string }
      firstTask: { label: string; hint: string }
      firstApproval: { label: string; hint: string }
    }
  }
  /** 对话空态示例 prompt（AionUi 式"活的引导"：点一下即进入真实工作流） */
  samplePrompts: { generic: [string, string]; withModule: string }
}

/** 引导文案词表：tx 注入 t()，缺 key（返回 key 本身）或未注入时回退中文。 */
export function getOnboardingCopy(tx?: OnboardingTx): OnboardingCopy {
  const tt = (key: string, fallback: string, vars?: Record<string, string | number>) => {
    if (!tx) return fallback
    const s = tx(key, vars)
    return s === key ? fallback : s
  }
  return {
    welcome: {
      kicker: tt('onboarding.welcomeKicker', '欢迎使用'),
      title: tt('onboarding.welcomeTitle', '让 Agent 写代码，让架构留在你手里'),
      subtitle: tt(
        'onboarding.welcomeSubtitle',
        'EasyVibe 从你的真实代码库归纳出架构地图，AI 的每次改动经审批门禁落地、全程留痕。',
      ),
      ctaPrimary: tt('onboarding.ctaAdd', '添加我的仓库'),
      ctaPrimaryHint: tt('onboarding.ctaAddHint', '选择本地代码目录，开始归纳架构地图（通常数分钟）'),
      ctaSecondary: tt('onboarding.ctaDemo', '先逛逛演示数据'),
      ctaSecondaryHint: tt('onboarding.ctaDemoHint', '稍后可从顶栏项目切换器随时添加仓库'),
      reopenNote: tt('onboarding.reopenNote', '本引导可随时从顶栏「?」重新打开'),
    },
    concepts: [
      {
        id: 'map',
        title: tt('onboarding.conceptMapTitle', '语义代码地图'),
        body: tt(
          'onboarding.conceptMapBody',
          'LLM 把代码库翻译成架构视图：模块、职责、依赖一目了然，存在你的仓库里（.easyvibe/map），git 可见、随时重新归纳。',
        ),
      },
      {
        id: 'health',
        title: tt('onboarding.conceptHealthTitle', '架构健康审计'),
        body: tt(
          'onboarding.conceptHealthBody',
          '模块级 + 架构级两级健康评估：耦合、复杂度、腐化标记，巡检自动对比上轮基线，漂移无处可藏。',
        ),
      },
      {
        id: 'task',
        title: tt('onboarding.conceptTaskTitle', '对话孵化任务'),
        body: tt(
          'onboarding.conceptTaskBody',
          '在「任务对话」里描述需求，AI 走 harness 五阶段流程：需求矩阵 → 方案设计 → 开发 → 代码审查，全程可见。',
        ),
      },
      {
        id: 'gate',
        title: tt('onboarding.conceptGateTitle', '审批门禁与留痕'),
        body: tt(
          'onboarding.conceptGateBody',
          '每个变更经你批准才落地（计划/Diff/审查报告三道关），全部产物归档进仓库，事后可回溯、可审计。',
        ),
      },
    ],
    waiting: {
      ideaTitle: tt('onboarding.ideaTitle', '等待时不白等'),
      ideaPlaceholder: tt('onboarding.ideaPh', '把你想做的第一个任务/改进想法写在这里…'),
      ideaSaved: tt('onboarding.ideaSaved', '已保存——归纳完成后自动带入「任务对话」'),
      ideaButton: tt('onboarding.ideaBtn', '保存想法'),
    },
    checklist: {
      title: tt('onboarding.checklistTitle', '上手指引'),
      doneToast: tt('onboarding.checklistDone', '🎉 上手指引全部完成，开始享受 EasyVibe'),
      items: {
        addRepo: {
          label: tt('onboarding.item.addRepo.label', '添加你的第一个仓库'),
          hint: tt('onboarding.item.addRepo.hint', '顶栏项目切换器 → 添加本地目录'),
        },
        viewMap: {
          label: tt('onboarding.item.viewMap.label', '查看生成的架构地图'),
          hint: tt('onboarding.item.viewMap.hint', '点开任意模块看详情与健康度'),
        },
        viewHealth: {
          label: tt('onboarding.item.viewHealth.label', '看看健康看板或漂移洞察'),
          hint: tt('onboarding.item.viewHealth.hint', '左侧导航「探索」组'),
        },
        firstTask: {
          label: tt('onboarding.item.firstTask.label', '孵化第一个任务'),
          hint: tt('onboarding.item.firstTask.hint', '「任务对话」里描述需求，或地图上点「发起修复」'),
        },
        firstApproval: {
          label: tt('onboarding.item.firstApproval.label', '完成一次审批'),
          hint: tt('onboarding.item.firstApproval.hint', '任务走到审批关时，在流水线里通过或驳回'),
        },
      },
    },
    samplePrompts: {
      generic: [
        tt('onboarding.sampleGeneric1', '帮我梳理这个仓库的架构分层是否合理，有没有逆向依赖？'),
        tt('onboarding.sampleGeneric2', '哪些模块的健康度最差？按优先级给出治理建议。'),
      ],
      withModule: tt('onboarding.sampleWithModule', '请评估一下「{module}」模块的健康度并给出改进建议。'),
    },
  }
}
