// 画布选区与右栏页签的共享契约类型（原定义在 components/DetailPanel.tsx）。
// 下沉到 renderer-shared：canvas 侧（Canvas/buildFlow/useCanvasPanelDrag）与 chat 侧
// （PanelChat/QuickAsk/WorkbenchPage/DepsPage）同时消费，留在 DetailPanel 会让 chat 域反向依赖 map-canvas。
export type Selection =
  | { kind: 'module' | 'layer'; id: string }
  | { kind: 'submodule'; parentId: string; subId: string }
  | null

export type PanelTab = 'detail' | 'issues' | 'chat'
