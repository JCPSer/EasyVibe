export interface Concern {
  severity: 'critical' | 'high'
  finding: string
  suggestion: string
}

export interface Health {
  score: number
  coupling: 'low' | 'medium' | 'high' | 'critical'
  complexity: 'low' | 'medium' | 'high'
  churn: 'low' | 'medium' | 'high'
  decay_flags: string[]
  review_note: string
  concerns?: Concern[]
}

export interface Layer {
  id: string
  name: string
  order: number
  description: string
}

export interface KeyEntry {
  file: string
  symbol: string
  kind: string
}

export interface Module {
  id: string
  name: string
  layer: string
  responsibility: string
  files: string[]
  key_entries: KeyEntry[]
  dependencies: string[]
  health: Health
}

export type EdgeType = 'call' | 'import' | 'api' | 'event' | 'db' | 'config'
export type EdgeStrength = 'strong' | 'normal' | 'weak'

export interface MapEdge {
  from: string
  to: string
  type: EdgeType
  label?: string
  strength: EdgeStrength
  direction_violation?: boolean
}

export interface MapMeta {
  repo: string
  generated_at: string
  generator: string
  description?: string
  languages?: string[]
  loc?: number
  map_freshness?: 'fresh' | 'drifting' | 'stale'
}

export interface CodeMap {
  version: string
  meta: MapMeta
  layers: Layer[]
  modules: Module[]
  edges: MapEdge[]
  health: Health
}

// §8 子图（模块展开 drill-down）
export interface SubModule {
  id: string
  name: string
  responsibility: string
  files: string[]
  key_entries: KeyEntry[]
  dependencies: string[]
  health: Health
}

export interface SubEdge {
  from: string
  to: string
  type: EdgeType
  label?: string
  strength: EdgeStrength
  circular_dep?: boolean
}

export interface SubMap {
  version: string
  parent: { module_id: string; files_snapshot: string[] }
  generated_at: string
  generator: string
  sub_modules: SubModule[]
  edges: SubEdge[]
}
