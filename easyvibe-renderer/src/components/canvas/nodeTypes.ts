import { ModuleNode } from './ModuleNode'
import { BandNode } from './BandNode'
import { ExpandedModuleNode } from './ExpandedModuleNode'
import { SubmoduleNode } from './SubmoduleNode'

/** ReactFlow 自定义节点类型表（画布私有） */
export const nodeTypes = { module: ModuleNode, moduleExpanded: ExpandedModuleNode, submodule: SubmoduleNode, band: BandNode }
