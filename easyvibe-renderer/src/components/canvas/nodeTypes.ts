import { ModuleNode } from '@/components/ModuleNode'
import { BandNode } from '@/components/BandNode'
import { ExpandedModuleNode } from '@/components/ExpandedModuleNode'
import { SubmoduleNode } from '@/components/SubmoduleNode'

/** ReactFlow 自定义节点类型表（画布私有） */
export const nodeTypes = { module: ModuleNode, moduleExpanded: ExpandedModuleNode, submodule: SubmoduleNode, band: BandNode }
