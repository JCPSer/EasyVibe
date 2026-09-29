// 数据类型的单一入口。
// 主图类型由 easyvibe-map-schema-v1.json 自动生成（./generated），禁止手工镜像 schema 字段；
// 改数据格式 = 改 Schema → npm run gen:types。
export type {
  CodeMap,
  MapMeta,
  Layer,
  Module,
  KeyEntry,
  Health,
  Concern,
  MapEdge,
} from './generated'
import type { KeyEntry, Health, Layer, Module, MapEdge } from './generated'

// ---- 以下类型尚无机器 Schema（格式规范 §8/§9 仅文档定义），先手工维护 ----
// TODO(v1.2)：为子图与用户视图补 JSON Schema 后并入自动生成

export type EdgeType = 'call' | 'import' | 'api' | 'event' | 'db' | 'config'
export type EdgeStrength = 'strong' | 'normal' | 'weak'

// growth.log 事件（v2.2 协议）
export type GrowthEvent =
  | { type: 'layer'; layer: Layer }
  | { type: 'module'; module: Module; out_edges: MapEdge[] }
  | { type: 'arch_health'; health: Health }
  | { type: 'done' }

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
  id: string // v1.1 起同主图规则
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

// §9 用户视图（对话沉淀）——引用式
export type ViewNodeRef =
  | { ref: `module:${string}` }
  | { ref: `submodule:${string}/${string}` }
  | { ref: `file:${string}` }

export interface ViewFile {
  version: string
  name: string
  created_at: string
  source: { conversation_id: string }
  nodes: ViewNodeRef[]
  edges: { from_ref: string; to_ref: string; label?: string }[]
  annotations: { ref: string; note: string }[]
}
