import type { CodeMap, Concern, Module } from '@/types/map'

export interface TaskDraft {
  title: string
  description: string
  modules: string[]
  acceptance: string
  source: 'module' | 'concern' | 'layer' | 'manual'
  context: Record<string, unknown>
  /** M4-2：任务←→会话关联（对话升级路径自动带上） */
  conversation_id?: string
}

// 指哪打哪的上下文组织器：把模块职责/边界/问题/相关违规边组装成任务草稿（M3-2）
// 草稿即"事前注入"的原料（F4）：提交后随任务进入 harness 执行上下文
export function buildModuleTask(map: CodeMap, moduleId: string): TaskDraft {
  const mod = map.modules.find((m) => m.id === moduleId)
  if (!mod) return { title: '', description: '', modules: [], acceptance: '', source: 'manual', context: {} }
  const violations = map.edges.filter((e) => e.direction_violation && (e.from === moduleId || e.to === moduleId))
  const concern = mod.health.concerns?.[0]
  const desc = concern
    ? `修复「${mod.name}」的问题：${concern.finding}\n建议方向：${concern.suggestion}`
    : `优化「${mod.name}」（${mod.responsibility}）：改善其健康度（当前 ${mod.health.score} 分）与依赖边界。`
  return {
    title: `修复 ${mod.name}`,
    description: desc,
    modules: [moduleId],
    acceptance: concern ? '按建议完成调整后，该问题不再复现；相关 direction_violation 消除或有明确豁免理由。' : '健康度评估提升，无新增逆向依赖。',
    source: 'module',
    context: {
      inject: {
        module: {
          id: mod.id,
          name: mod.name,
          layer: mod.layer,
          responsibility: mod.responsibility,
          files: mod.files,
          health: mod.health,
        },
        violations,
      },
    },
  }
}

/** M4-1.5：子模块修复——任务挂在父模块上，上下文注入子模块的职责/文件/健康评审 */
export function buildSubmoduleTask(parent: Module, sub: { id: string; name: string; responsibility: string; files: string[]; health: unknown }): TaskDraft {
  const h = sub.health as { score: number; review_note?: string; decay_flags?: string[] }
  const desc = `修复父模块「${parent.name}」内的子模块「${sub.name}」（内部健康 ${h.score} 分）。\n职责：${sub.responsibility}\n文件：${sub.files.join('、')}\n评审意见：${h.review_note ?? '无'}${h.decay_flags?.length ? `\n腐化标记：${h.decay_flags.join('、')}` : ''}`
  return {
    title: `修复 ${parent.name} · ${sub.name}`,
    description: desc,
    modules: [parent.id],
    acceptance: '子模块内部健康度提升（重新展开子图复检），不破坏父模块对外契约。',
    source: 'module',
    context: {
      inject: {
        module: { id: parent.id, name: parent.name, layer: parent.layer, responsibility: parent.responsibility, files: parent.files },
        submodule: { id: sub.id, name: sub.name, files: sub.files, health: sub.health },
      },
    },
  }
}

export function buildConcernTask(map: CodeMap, moduleId: string, concern: Concern, index: number): TaskDraft {
  const base = buildModuleTask(map, moduleId)
  const mod = map.modules.find((m) => m.id === moduleId)
  return {
    ...base,
    title: `修复 ${mod?.name ?? moduleId} · 问题 ${index + 1}`,
    description: `${concern.finding}\n建议：${concern.suggestion}`,
    acceptance: '问题消除，审查通过；如无法消除，给出结构化的豁免说明。',
    source: 'concern',
    context: { ...base.context, concern },
  }
}

export function buildLayerTask(map: CodeMap, layerId: string): TaskDraft {
  const layer = map.layers.find((l) => l.id === layerId)
  const mods = map.modules.filter((m) => m.layer === layerId)
  const violations = map.edges.filter((e) => e.direction_violation && mods.some((m) => m.id === e.from || m.id === e.to))
  const worst = [...mods].sort((a, b) => a.health.score - b.health.score)[0]
  return {
    title: `治理层「${layer?.name ?? layerId}」`,
    description: `对「${layer?.name}」层（${mods.length} 个模块，${violations.length} 条层间逆向依赖）做架构治理。${worst ? `最薄弱模块：${worst.name}（${worst.health.score} 分）——${worst.health.review_note}` : ''}`,
    modules: mods.map((m) => m.id),
    acceptance: '层内模块健康度均 ≥75 或给出豁免；层间逆向依赖消除或有明确豁免。',
    source: 'layer',
    context: {
      inject: {
        layer,
        modules: mods.map((m) => ({ id: m.id, name: m.name, responsibility: m.responsibility, health: m.health })),
        violations,
      },
    },
  }
}

export interface Suggestion {
  title: string
  description: string
  modules: string[]
  priority: string
  rationale: string
}

// 智能优化建议 → 任务草稿：AI 主动发现的优化机会一键转修复任务
export function buildSuggestionTask(map: CodeMap, sg: Suggestion): TaskDraft {
  const validModules = sg.modules.filter((id) => map.modules.some((m) => m.id === id))
  const names = validModules.map((id) => map.modules.find((m) => m.id === id)!.name)
  return {
    title: sg.title,
    description: `${sg.description}\n\n（优化建议 · ${sg.rationale}）`,
    modules: validModules,
    acceptance: '优化落地后地图重归纳/巡检确认改善，且无新增逆向依赖。',
    source: 'manual',
    context: {
      inject: {
        suggestion: sg,
        modules: validModules.map((id) => {
          const m = map.modules.find((x) => x.id === id)!
          return { id: m.id, name: m.name, responsibility: m.responsibility, health: m.health }
        }),
      },
      note: `影响模块：${names.join('、') || '待确认'}`,
    },
  }
}
