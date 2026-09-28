# hover-client v2.1 试点验收 fixtures

提示词 `easyvibe-map-prompt-v2.1.md` 的实证基线。2026-09-29 在 hover-client
（Flutter/Dart，308 Dart 文件，144,389 LOC）上按 v2 全流程真实执行产出，
执行中发现的问题与修订依据见 `pilot-report-v2.md`。

## 内容

```
expected/
  map.json            最终地图（finalize 五道关全过的产物）
  growth.log          生长日志（20 事件：6 layer + 12 module + arch_health + done）
  progress.done.json  完成态进度（含 decision_reason=backed_up 记录）
  repo_profile.json   scanning 阶段机械数据
  edges_computed.json edging 阶段确定性 import 聚合（70 边及计数）
  emit_order.json     追加顺序显式 manifest
reference-easyvibe_map.py  CLI 参考实现（提示词 CLI 契约的可工作样本）
pilot-report-v2.md    试点报告（6 项提示词修订的实证来源）
check_fixtures.py     回归检查脚本
```

## 回归用法

```bash
python3 check_fixtures.py /Users/liyuhang/Documents/git_projects/language-band/hover-client \
    /Users/liyuhang/Documents/EasyVibe/easyvibe-map-schema-v1.json
```

六项检查：Schema 校验、growth.log 重放一致、layers 存储序升序、deps==edges、
覆盖率 >90%、**边确定性回归**（按 map.json 的模块 globs 重算 lib/ import 聚合，
与 `edges_computed.json` 逐边比对）。任一项失败退出码 1。

## 什么时候会失败（预期内）

- **边确定性回归失败**：hover-client 的 lib/ 代码发生了 import 层面的真实变更。
  这是 fixtures 的设计目的——提示词或 Schema 再修订后，重跑分析应先人工确认
  新边集合理，再更新 `expected/edges_computed.json` 与本目录的 map 产物；
- **Schema 校验失败**：`easyvibe-map-schema-v1.json` 升级后未同步 fixtures；
- **覆盖率/重放失败**：CLI 参考实现或提示词流水线被改动后行为回退。

## 注意

- `map.json` 中的 meta.generated_at 为试点时刻，比较时请忽略时间戳字段；
- 本 fixtures 只覆盖 WORKERS=1 路径；并行 fan-out 的 fixtures 待宿主支持
  子 agent 后补充。
