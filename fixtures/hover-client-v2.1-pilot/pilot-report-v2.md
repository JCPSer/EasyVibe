# EasyVibe 提示词 v2 试点报告 — hover-client

- 日期：2026-09-29
- 对象仓库：hover-client（Flutter/Dart，308 个 Dart 文件，144,389 LOC）
- 试点方式：WORKERS=1 全流程实跑（含已存在文件分支的真实触发）
- 耗时：init→done 约 3 分钟（T0 01:52:21 → T7 01:55:29）

## 0. 意外收获：真实覆盖了"已存在文件"分支

试点准备时发现 `map.json` 已被另一生成器（deepseek-v4.1-flash，01:10）覆盖，
init 阶段真实检测到已存在文件并执行了备份决策（见下"发现 1"）。
DeepSeek 版本留存为 `map.v1.reference.json`，成为交叉验证参照。

## 1. 流水线执行结果

| 阶段 | 结果 | 证据 |
|---|---|---|
| init | ✅ 检测到已存在 map.json，判定并备份 | progress.json `existing_file_decision` |
| scanning | ✅ 重算 308 文件 / 144,389 LOC / churn top10 | `map/repo_profile.json` |
| clustering | ✅ 12 模块方案复核（lib/ 自 v1 分析以来无变化，git 验证） | — |
| module-analysis | ✅ 12/12 模块逐一经 CLI 校验写入 parts/，progress 逐步推进 | `map/parts/*.json` |
| edging | ✅ 确定性 import 聚合重算，70 边，关键计数与 v1 对拍一致 | `map/edges_computed.json` |
| append-log | ✅ 20 事件（6 layer + 12 module + 1 arch_health + 1 done），逐行合法 JSON | `map/growth.log` |
| finalize 五道关 | ✅ 全过 | 见下 |

finalize 五道关：gate1 Schema（jsonschema）PASS；gate2 自检 PASS（模块均分 72.3，
架构分 58，独立判定成立）；gate3 重放==parts PASS（12 模块）；gate4 覆盖率 100.0%；
gate5 原子写入 + phase=done PASS。

## 2. 提示词 v2 需修订项（试点实证发现）

1. **已存在文件的判定启发式误报**。DeepSeek 产物顶层 health 带 Schema 之外的
   `concerns` 字段（不合规生成，非人工修改），被"Schema 外字段=人工修改"规则
   误判。建议改为三分类：合规机器产物→覆盖；不合规机器产物/疑似人工修改→
   一律备份（安全兜底），progress.json 记录 `decision_reason` 区分两类。
2. **arch_health 事件的产出来源未指定**。已补进 CLI：health 阶段由 orchestrator
   写 `parts/_arch_health.json`，append-log 携带。提示词 v2 需写明。
3. **生长序与存储序是两个序**。growth.log 按层序自底向上追加，但 map.json 的
   layers[] 必须按 order 升序存储——第一版 finalize 没做排序，被 gate2 拦下。
   提示词需明确"追加顺序 ≠ 存储顺序，finalize 负责归一"。
4. **追加顺序要落成显式 manifest**。口头约定"层序自底向上"在并行下不可判定
   完成先后，已采用 `emit_order.json` 显式清单（layers 有序数组 + modules 有序 id）。
   提示词 v2 应把 manifest 列为 append-log 的输入契约。
5. **小仓库的阶段权重推进几乎不可见**。scanning/clustering 瞬过未单独更新
   progress；大仓库需在每阶段边界显式置位 percent。
6. **正面证据：五道关拦下了实现者自己的 bug**（发现 3 的 layers 排序错误），
   验证闭环设计有效。

## 3. 跨生成器交叉验证（Kimi v2 vs DeepSeek 参照）

| 维度 | Kimi v2 | DeepSeek | 结论 |
|---|---|---|---|
| 层数/层 id/层序 | 6 层（app-entry…foundation） | **完全相同** | 分层方案是强信号，两模型独立收敛 |
| 模块数 | 12 | 12 | 一致 |
| 架构 decay_flags | layer_violation, layering_mismatch, circular_dep, responsibility_overlap | **完全相同（4/4）** | 架构级诊断高度一致 |
| 逆向边 | 14/70 | 14/77 | 一致（边集密度不同） |
| 架构 health 分 | 58 | 57 | 噪声范围内 |
| 同名模块 health 分差 | — | — | 全部 Δ≤10，多数 Δ≤3 |

分歧点在**模块粒度判断**：DeepSeek 把 UI 拆成 desktop-ui/mobile-ui/shared-ui 三块，
Kimi 合为 ui-shell；基础设施侧 DeepSeek 合为 device-services，Kimi 拆出
audio-media/local-storage/system-services。结论：层划分与架构诊断稳定可复现，
功能域内聚粒度属合理判断差异——提示词 v2 可对"何时按平台/媒介拆分 UI 模块"
给出指引，降低此处的随机性。

## 4. 边界与未尽事项

- **真并发未验**：本环境禁止主动开子 agent，WORKERS>1 路径仅验证了分片协议
  （parts 文件名分片、orchestrator 单写者、finalize 缺口检测），多写者竞态
  与 worker prompt 的评分口径漂移待宿主支持后验证。
- **模块定义复用**：因 lib/ 自 v1 分析（21:53）以来无变化（git 验证），模块
  深读结论复用 v1；edging 与 scanning 为真实重算。对结论的影响见第 3 节。
- **CLI 为参考实现**：`.easyvibe/easyvibe_map.py` 约 210 行，验证契约可实现，
  非生产级（无 Windows rename 兼容、无日志、无过期 parts 清理）。

## 5. 产物清单

- `.easyvibe/map/map.json` — 最终地图（phase=done）
- `.easyvibe/map/growth.log` — 20 事件生长日志
- `.easyvibe/map/progress.json` — 进度（含已存在文件决策记录）
- `.easyvibe/map/parts/` — 12 模块分片 + 各模块出边 + 架构 health
- `.easyvibe/map/repo_profile.json` / `edges_computed.json` / `emit_order.json` / `meta.json`
- `.easyvibe/map/map.json.bak-2026-09T015308_0800` — DeepSeek 产物备份
- `.easyvibe/map.v1.reference.json` — 交叉验证参照
- `.easyvibe/easyvibe_map.py` — CLI 参考实现
- `.easyvibe/build_parts.py` — 试点编排脚本
