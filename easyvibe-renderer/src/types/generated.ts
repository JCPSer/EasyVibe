/* eslint-disable */
/**
 * 本文件由 scripts/gen-types.mjs 从 easyvibe-map-schema-v1.1.json 自动生成，请勿手改。
 * 修改数据格式请改 Schema，然后 npm run gen:types。
 */

/**
 * EasyVibe 语义代码地图数据格式 v1.1（边稳定 id + meta.stats 溯源 + 模块级时间戳；向后兼容）。数据文件存语义，不存布局；布局由渲染器推导。
 */
export interface CodeMap {
  version: "1.0";
  meta: MapMeta;
  /**
   * 矩形分层架构图的层定义，由 LLM 按项目实际结构动态生成（层数、层名、层职责均不预设）。order=0 为最顶层，数字越大越靠底。
   *
   * @minItems 1
   */
  layers: [Layer, ...Layer[]];
  /**
   * @minItems 1
   */
  modules: [Module, ...Module[]];
  /**
   * 模块间依赖边，渲染依赖关系的唯一事实来源
   */
  edges: MapEdge[];
  health: Health;
}
export interface MapMeta {
  /**
   * 仓库标识（通常为仓库名或根目录名）
   */
  repo: string;
  /**
   * ISO 8601 时间戳
   */
  generated_at: string;
  /**
   * ISO 8601；上次巡检时间，F7 新鲜度提醒依据
   */
  last_patrol_at?: string;
  /**
   * 溯源统计（finalize 阶段确定性计算，非 LLM 自评）
   */
  stats?: {
    /**
     * 产品代码文件总数
     */
    files_total?: number;
    /**
     * 被 modules[].files 覆盖的文件数
     */
    files_covered?: number;
    /**
     * 覆盖率
     */
    coverage_ratio?: number;
    /**
     * 确定性推导（import 扫描等）的边数
     */
    edges_derived?: number;
    /**
     * LLM 断言的边数
     */
    edges_inferred?: number;
    /**
     * 校验-重试循环中重试过的模块数
     */
    retried_modules?: number;
  };
  /**
   * 生成者标识，格式：agent名/模型名，如 easyvibe-architect/claude-sonnet-4-6
   */
  generator: string;
  /**
   * 全库一句话概述
   */
  description?: string;
  /**
   * 主要编程语言列表
   */
  languages?: string[];
  /**
   * 估算代码行数
   */
  loc?: number;
  /**
   * fresh=与代码一致；drifting=存在漂移；stale=已明显过期
   */
  map_freshness?: "fresh" | "drifting" | "stale";
}
export interface Layer {
  id: string;
  name: string;
  order: number;
  /**
   * 该层的职责定义
   */
  description: string;
}
export interface Module {
  /**
   * 模块唯一标识，小写字母/数字/下划线/中划线
   */
  id: string;
  /**
   * ISO 8601；该模块上次归纳/巡检时间，F7 模块级新鲜度依据
   */
  last_analyzed_at?: string;
  /**
   * 模块显示名，人类可读
   */
  name: string;
  /**
   * 所属层 id，必须存在于 layers[]
   */
  layer: string;
  /**
   * 一句话职责描述
   */
  responsibility: string;
  /**
   * 归属该模块的文件/目录，glob 路径，相对仓库根
   *
   * @minItems 1
   */
  files: [string, ...string[]];
  /**
   * 关键入口标注
   */
  key_entries: KeyEntry[];
  /**
   * 依赖的模块 id 列表；edges[] 中必须有对应 from=本模块 的边
   */
  dependencies: string[];
  health: Health;
  /**
   * 其他备注（可选）
   */
  notes?: string;
}
export interface KeyEntry {
  file: string;
  symbol: string;
  kind: "function" | "class" | "interface" | "route" | "cli" | "job" | "config";
}
/**
 * 架构级 LLM 评估（跨模块耦合形态、分层合理性、逆向依赖密度、职责重叠/缺失）；模块全绿 ≠ 架构健康，须独立评估
 */
export interface Health {
  /**
   * LLM 综合健康分
   */
  score: number;
  coupling: "low" | "medium" | "high" | "critical";
  complexity: "low" | "medium" | "high";
  /**
   * 近期变更频率，LLM 依据 git 历史判断
   */
  churn?: "low" | "medium" | "high";
  /**
   * 腐化标记，模块级如 coupling_high, god_module, circular_dep；架构级如 layer_violation, responsibility_overlap, layering_mismatch
   */
  decay_flags: string[];
  /**
   * LLM 给出的改进建议
   */
  review_note?: string;
  /**
   * LLM 提名最值得关注的 top-3 问题（架构级与模块级共用结构）
   *
   * @maxItems 3
   */
  concerns?: [] | [Concern] | [Concern, Concern] | [Concern, Concern, Concern];
}
export interface Concern {
  /**
   * critical=建议尽快处理；high=值得关注
   */
  severity: "critical" | "high";
  /**
   * 现状：一句话说清问题是什么
   */
  finding: string;
  /**
   * 建议：一句话给出改法
   */
  suggestion: string;
  /**
   * 稳定标识：c-arch-N（架构级）/ c-<module_id>-N（模块级）；巡检继承上轮同问题 id
   */
  id?: string;
}
export interface MapEdge {
  /**
   * 边稳定 id（v1.1 起必填）：供边级证据、置信度、视图引用。按确定性生成序编排（如 e1、e2），重归纳时必须保持稳定
   */
  id: string;
  /**
   * 起点模块 id
   */
  from: string;
  /**
   * 终点模块 id
   */
  to: string;
  /**
   * call=函数调用；import=代码引用；api=HTTP/RPC；event=消息事件；db=共享数据库；config=配置依赖
   */
  type: "call" | "import" | "api" | "event" | "db" | "config";
  /**
   * 边上显示的文字（可选），如方法名
   */
  label?: string;
  /**
   * 耦合强度估计：strong=频繁/核心路径，weak=偶发
   */
  strength: "strong" | "normal" | "weak";
  /**
   * true=从下层指向上层的逆向依赖（架构违规信号）
   */
  direction_violation?: boolean;
}
