# 架构守卫复核手册（v1，c-arch-18 / R8）

本手册把「复核面」写成**可粘贴的命令**与**口径说明**，防止下一轮巡检以口径差异（如「扫描器 60 行 ≠ 图内 59 对」）误判漂移。
所有命令在仓库根执行，纯 `python3`，不需 cargo / npm。

> 位置说明：需求方案原写「增补到 `docs/`」，但 `docs/` 在 `.gitignore` 内（`/docs/`）⇒ 手册在版本控制与 CI 面前
> 不可见。故落到 `scripts/`（受版本控制，且由 `verify_assets.py --forbid-literals` 自守：本文件不得出现受管契约名 / env 名）。

## 0. 书写约定（易错点）

- **旗标 vs 子命令**：`verify_assets.py` 走**旗标**（`--check` / `--verify-manifest` / `--forbid-literals` / `--selfcheck`），
  **没有** `check` 子命令；多数 `check_*.py` / `verify_*.py` 走**子命令**（`--check` / `--selfcheck`）。二者不要混写。
- **live 面 vs 受控面**：`.easyvibe/**` 整目录 gitignored ⇒ **CI 上不存在 live map**。CI 只跑「对受版本控制 fixture /
  策略 / 快照」的判据；依赖 live 的判据（`check_notes_freshness --check`、`check_churn_bands`、`gen_map_fixture --check`）
  只在**本地 / 归纳期**跑。
- **边扫描口径（★最易误判）**：`.easyvibe/map/_tools/scan_edges.py` 的原始输出是 **60 行**，其中 1 行是
  `host-adapter|__host__`（宿主能力标记，**不是模块对**）⇒ 真实 **import 模块对 = 59**。
  `59 + 8 条非 import 边 = 67 = 图内边数`。下一轮若看到「60 ≠ 59」，是口径未扣伪边，不是漂移。
- **测试面不产出边**（c-arch-15）：`product_files()` 仍枚举测试面文件、归属/coverage 不受影响，但 `is_test_face` 谓词
  使它们**不参与出边证据提取**。
- **prose 模板 token 必先注入（★返修 P0-1）**：authoring 面（`.easyvibe/map/parts/**`）的 prose 携带模板 token
  `{{provenance.head_short}}` / `{{provenance.generated_at}}`，由 `finalize` 用 `meta.provenance` 填充（`assemble` 与
  `replay_from_log` **同源同一 prov**）。交付地图（live / fixture 投影）**不得残留 token**：判据 L 对残留即红。
  往 parts 写入 prose 后必须重走 `init → append-log → finalize → gen_map_fixture（--refresh-policy-digests 再 write）`。
- **`meta.provenance.generated_at` = HEAD 的提交时间（非墙钟）**：叙事基准的时间戳半边取 `git show -s --format=%cI HEAD`，
  故同一 HEAD 反复 finalize 逐字节相同（可复检性），并作为 churn 窗口终点（`--until`）的确定锚点。
  墙钟生成时间另记于 `meta.generated_at`（不参与任何受控投影）。

## 1. 结构面（边 / 层 / 模块 / 同代）

```bash
python3 scripts/verify_map_acyclic.py --check          # DAG / DV / 跨层 SCC / 边数
python3 scripts/verify_map_acyclic.py --selfcheck
python3 scripts/verify_arch_facts.py --check --fixture scripts/tests/fixtures/map_post_split.json  # CI
python3 scripts/verify_arch_facts.py --check --live                                                 # 本地/归纳期
python3 scripts/verify_arch_facts.py --selfcheck
python3 scripts/verify_arch_split.py --map scripts/tests/fixtures/map_post_split.json
python3 scripts/verify_arch_split.py --selfcheck
python3 scripts/gen_map_fixture.py --check             # 本地：结构投影 + 叙事投影「重生成 == 已提交」
```

## 2. 叙事面（prose 摘要 / 计数规则 / 基准 / churn 档位）

```bash
python3 scripts/check_notes_freshness.py --check                                     # 本地：live 面
python3 scripts/check_notes_freshness.py --projection scripts/tests/fixtures/map_prose_projection.json  # CI
python3 scripts/check_notes_freshness.py --selfcheck
python3 scripts/check_churn_bands.py                                                 # 本地
python3 scripts/check_churn_bands.py --projection scripts/tests/fixtures/map_prose_projection.json       # CI（需 fetch-depth: 0）
python3 scripts/check_churn_bands.py --selfcheck
```

判据速查：I14 = 摘要闸门（顶层 `review_note` + 逐格 `review_note` 与 `notes`）；
J = 退役 concern 不得在 prose 中复现为「未闭环」；K = prose 不得写滑动窗口量（churn 触点 / 全库排名 / 产品文件数 / 文件行数），
离散档位与 policy 键名引用不受限；L = 叙事基准同代，三条 fail-closed：
**L-1** 任何受判 prose 残留模板 token（`{{provenance.*}}`）⇒ 红；
**L-2** 顶层 arch note **必须**至少含一个 `HEAD <sha>`（占位符 / 漏写 / 无基准都红，不再静默通过）；
**L-3** prose 内 `HEAD <sha>` 必须等于 `meta.provenance.head_short`。

**架构级在册 concern**（P0-2 返修）：`gates.active_concerns` 是交付地图顶层 `health.concerns` 的 id 集唯一真值；
`verify_arch_facts` F1 对 fixture/live 逐份断言「顶层 concern 集 == active_concerns」，投影快照亦带 `arch_concerns`
（`check_notes_freshness --projection` 断言与 policy 同代）——「架构级关注点在交付面消失」必红。

**churn 窗口固化**（P3-8 返修）：`check_churn_bands` 的复算窗口终点取 `provenance.generated_at`（= HEAD 提交时间），
使 CI 复算**确定**、不随运行时刻滑移（边界格不再无代码变更而假红）。

## 3. 耦合趋势（「架构级问题不再复现」的落点）

```bash
python3 scripts/check_coupling_trend.py --check      # CI：对受版本控制 fixture
python3 scripts/check_coupling_trend.py --selfcheck
```

口径：`gates.coupling_ratchet`（`coupling_high` 载体计数上限 + 枢纽出入度上限，**只降不升**）
＋ `gates.accepted_coupling`（每个载体的「已接受」裁定：role / reason / review_by）。
棘轮**不设「必须下降」目标**——组合根/装配枢纽知道全部后端域是正当职责；新增耦合必须**显式重登记并给理由**；
拆格降耦合须另立项（改结构面 ⇒ 触发地图重采与冻结表同步）。

## 4. 其余守卫（逐条命令）

```bash
python3 scripts/verify_assets.py --check
python3 scripts/verify_assets.py --forbid-literals
python3 scripts/verify_assets.py --selfcheck
bash scripts/verify-assets.sh --check
python3 scripts/check_shell_boundary.py --check --selfcheck           # 壳边界
python3 scripts/check_components_ownership.py --check --selfcheck     # 组件归属
python3 scripts/verify_components_ownership.py --map scripts/tests/fixtures/map_post_ownership.json --selfcheck
python3 scripts/check_map_domain_guard.py --check --selfcheck         # 地图域守卫（解析 Rust 常量）
python3 scripts/check_renderer_rest.py --check --selfcheck            # REST/WS 直连
python3 scripts/check_renderer_granularity.py --check --map scripts/tests/fixtures/map_post_split.json --selfcheck
python3 scripts/check_console_granularity.py --check --map scripts/tests/fixtures/map_post_split.json --selfcheck
python3 scripts/check_i18n_sharding.py --check --selfcheck
python3 scripts/check_host_boundary.py --check --map scripts/tests/fixtures/map_post_split.json --selfcheck
python3 scripts/check_app_db_boundary.py --check --selfcheck
python3 scripts/check_app_service_boundary.py --check --selfcheck
python3 scripts/check_map_edge_test_face.py --check --selfcheck       # 测试面口径 + G5 扫描口径
```

## 5. CI 覆盖面（`.github/workflows/asset-guard.yml`）

`checkout` 为 `fetch-depth: 0`（churn 复算需 92 天历史）。步骤覆盖：产物一致性 / 字面量 / 自检 / shim / 壳边界 /
无环 / 架构事实同代 / 边界领域分离 / 组件归属（两条）/ 地图域守卫 / REST 直连 / renderer 粒度 / console 粒度 /
i18n 分片 / 宿主边界 / app DB 边界 / app 编排边界 / 测试面与扫描口径 / **叙事投影同代** / **churn 档位同代** /
**耦合趋势棘轮**（各自带 `--selfcheck`）。

**不入 CI 者**（依赖 gitignored 的 live map）：`gen_map_fixture.py --check`、`check_notes_freshness.py --check`、
`check_churn_bands.py`（无 `--projection` 时）、`verify_arch_facts.py --check --live`。它们只在本地/归纳期跑。

## 6. 叙事面覆盖边界（诚实声明，P2-4 取舍）

摘要闸门 I14 与计数判据 K 的覆盖面 = `gates.notes_projection.fields`（**顶层 `health.review_note` + 逐格
`review_note` + 逐格 `notes`**）。**未覆盖**的用户可见 prose：
逐格 `responsibility`、`key_entries[].symbol`、concern 的 `finding`/`suggestion` 正文、`files` 之外的描述性文本。
取舍理由：这些字段同样是用户可见 prose，但其内容**逐轮随结构变化**（如 `key_entries` 的行数会随任何提交失真），
把它们纳入字节摘要会把「结构生长」误判为「叙事漂移」，代价高于收益。故本轮**只做人工复核 + 就地订正**（如把 4 处
陈旧行数 500/460/293/323 订正为实测 502/464/295/325），**不**纳入闸门。若下一轮要把覆盖面再扩，须先解决
「行数/规模类描述」的稳定表达（改为口径引用，如 K 所做），再进摘要。

## 7. 地图管线再生成顺序（硬约束，R2→R6→R3→R4）

```bash
python3 scripts/gen_map_cli.py                         # run/ 源 → .easyvibe 自托管副本（逐字节）
REPO_ROOT="$PWD" python3 run/easyvibe_map_cli.py init  # 归档并清空 growth.log（唯一合法截断点）
REPO_ROOT="$PWD" python3 run/easyvibe_map_cli.py append-log
REPO_ROOT="$PWD" python3 run/easyvibe_map_cli.py finalize            # 生成期注入 provenance + replay 关口
python3 scripts/gen_map_fixture.py --refresh-policy-digests          # 冻结 prose 摘要（hex）进 policy
python3 scripts/gen_map_fixture.py                                   # 重生成结构投影 + 叙事投影
```
顺序不可换：先定稿 prose（finalize），再冻结摘要（refresh），最后生成受控投影。任一 `parts/**` prose 改动后都须重走此序列。
