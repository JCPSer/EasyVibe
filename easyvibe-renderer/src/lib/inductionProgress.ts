// 归纳期间地图区域"原位过场"的纯逻辑：阶段标签表 / 陈旧 done 解读 / agent 输出行过滤。
// 数据通道：GET /api/repos/{id}/progress 透传 .easyvibe/map/progress.json（文件缺失返回 data:null）。
// 词表按 v2.2 prompt 真实 phase 词汇：init|scanning|clustering|module-analysis|edging|health|assembling|done|failed。

export interface InductionProgress {
  phase: string
  percent: number
  modules_done: number
  modules_total: number
  current_module?: string | null
}

/** v2.2 协议 phase → 中文阶段名。done/failed 是终态，不展示（由退出逻辑处理）。 */
export const INDUCTION_PHASE_LABELS: Record<string, string> = {
  init: '初始化',
  scanning: '模块扫描',
  clustering: '结构聚类',
  'module-analysis': '模块归纳',
  edging: '依赖边推导',
  health: '健康度评估',
  assembling: '收尾组装',
}

export function inductionPhaseLabel(phase: string): string {
  return INDUCTION_PHASE_LABELS[phase] ?? phase
}

export interface InductionPhaseView {
  title: string
  percent: number | null // null → 不显示百分比（无数据/写盘收尾中）
  subline: string
}

/**
 * progress.json → 阶段卡视图。
 * 陈旧 done 陷阱：progress.json 是上一次归纳残留的 done 不得当真——会话仍活动但 phase=done，
 * 只说明 agent 已写完 progress、正在落盘 map.json，解读为"写盘收尾中"，不显示百分比。
 * 文件缺失（null）退回"归纳中 · agent 执行中"，不显示百分比。
 */
export function interpretInductionProgress(prog: InductionProgress | null): InductionPhaseView {
  if (!prog || prog.phase === 'failed') {
    return { title: '归纳中', percent: null, subline: 'agent 执行中，通常数分钟' }
  }
  if (prog.phase === 'done') {
    return { title: '写盘收尾中', percent: null, subline: 'agent 执行中，通常数分钟' }
  }
  const subline = prog.current_module
    ? `正在归纳模块：${prog.current_module}`
    : prog.modules_total > 0
      ? `已归纳 ${prog.modules_done}/${prog.modules_total} 个模块`
      : 'agent 执行中，通常数分钟'
  const percent =
    typeof prog.percent === 'number' && Number.isFinite(prog.percent)
      ? Math.max(0, Math.min(100, Math.round(prog.percent)))
      : null
  return { title: inductionPhaseLabel(prog.phase), percent, subline }
}

/**
 * agent 实时输出行过滤（口径复用 RunsPage toBlocks/lastTextOf）：
 * stderr/[err] 是协议噪音不显示；空行跳过；[思考] 前缀剥掉，改"正在思考：…"。
 */
export function formatTickerLine(line: string, stream: string): string | null {
  if (!line || !line.trim()) return null
  if (stream === 'stderr' || line.startsWith('[err]')) return null
  if (line.startsWith('[思考]')) {
    const body = line.slice('[思考]'.length).trim()
    return body ? `正在思考：${body}` : null
  }
  return line
}
