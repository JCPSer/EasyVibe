import type { GrowthEvent } from '@/types/map'

/** 画布过滤器：只看违规 / 问题视图 / 强聚焦（solo） */
export interface Filters {
  violationsOnly: boolean
  issuesOnly: boolean
  solo: boolean // 只看选中模块的依赖（强聚焦）
}

/** 同屏最多展开的模块数（超出淘汰最旧） */
export const MAX_EXPANDED = 3

/** 生长回放状态（消费 v2.2 growth.log 事件流） */
export interface GrowthState {
  events: GrowthEvent[]
  index: number // 已消费事件数
  playing: boolean
  done: boolean
}
