// Git 域共享类型（GitPage / SourceStrip / DiffDrawer 三方共用；与后端 easyvibe-git::GitFile 形状一致）。

export interface GitFile {
  status: string // M / A / D / R / ?
  path: string
  orig: string | null
  adds: number | null
  dels: number | null
}

export interface TaskLite {
  id: string
  title: string
  result?: { diffStat?: string } | null
}
