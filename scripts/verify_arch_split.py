#!/usr/bin/env python3
"""c-arch-1 关闭验收：把「边界层不得含业务规则」做成只读、可复跑、可趋势的判据（I1–I11）。

  --check [--map PATH]  默认读 live `.easyvibe/map/map.json`（本地/归纳期真值）
  --map <fixture>       读受版本控制快照（CI 用；不依赖被 gitignore 的 `.easyvibe/`）
  --selfcheck           负例自证（不依赖 live map、不改仓库）

断言：
  I1 覆盖率 coverage_ratio >= 1.0（fail-closed；地图自身 finalize 关口为 0.90）
  I2 server-api 归属文件数**不得超过登记棘轮上限**（只降不升）
     （脚本口径：app crate 产品文件 − `tests/` − `task_exec*`），
     且五个领域文件在 app crate 内均已不存在
  I3 server-api 归属 LOC **不得超过登记棘轮上限**（只降不升）

  ★ c-arch-7 登记（2026-10-06，方案硬伤处置）：本轮 app crate 结构性新增 4 个文件
     —— 组合根 `src/db_ports.rs`（task-engine 端口 ↔ 仓储适配器）+ 服务编排域
     `src/service/{agent,sessions,settings}.rs`（routes 直连归零的下沉落点），合计 599 行。
     这使「与 c-arch-1 拆分前快照（33 文件 / 7467 行）的相对比较」不再适用：新文件是
     application 层的编排/装配，不是领域规则回流边界。处置 = 把趋势判据从「相对旧快照」
     改为**登记棘轮上界（只降不升）**：文件数与 LOC 均不得超过本轮登记值，任何进一步增长
     即红；与拆分前快照的差额仍在报告里作为趋势信息输出（loc_drop_vs_pre_split）。
  I4 架构 concerns 无已登记闭环项（c-arch-1 / c-arch-13）；server-api 模块 decay_flags 无 god_module
  I5 外提模块在册（easyvibe-git / easyvibe-pipeline 出现在图内；live 模式另查 emit_order.json）
  I6 跨层 SCC == 0 且 direction_violation == 0（c-arch-5 闭环后棘轮归零；复用 map_policy.py，与 finalize 同实现）
  I7 趋势双口径输出（地图自评分 + 巡检分来源说明）
  I8 装配格棘轮（c-arch-10：assembly/** 文件数 / LOC 只降不升）
  I9 server-api 出边数 ≤ gates.server_api_out_edges_max，且出边目标集 == modules[server-api].dependencies
     （INV-1）== gates.server_api_out_edges（不升 + 逐条登记；c-arch-13 R5）
  I10 非 TestFixture 的 server-api 产品文件（`is_product()` 口径）LOC ≤ gates.single_file_loc_max（默认 400），
     且「实际 >400 的集合」与 gates.single_file_loc_caps 键集**双向全等**（棘轮自清理；c-arch-13 R6）
  I11 焦点面（src/db_ports.rs 或 src/db_ports/** + src/state.rs）直连集中度三量（c-arch-13 R6b）：
     I11a 守恒律：`easyvibe_db::` 出现总数 == gates.db_direct_total
     I11b 集中度：单文件最大 == gates.db_direct_focus_max（**双边全等**，不是 ≤）
     I11c 落点面：有直连的文件数 == gates.db_direct_focus_files_max
  I13 组合根（assembly）唯一性 + 出边登记（c-arch-16 R5）：
     I13a 图内 `{e | e.to == "assembly"} == ∅`（唯一组合根，无上游；违反即红）
     I13b `gates.assembly_out_edges` 存在且非空（缺失/空 ⇒ fail-closed 红），
          且 `{e.to | e.from == "assembly"} == set(登记)`（允许增长但禁静默）
     I13c 语义（注释）：判据对象是「**边界/编排层**（server-api）的出边数」，不是「谁度数居首」；
          组合根知道全部后端域是正当的（这正是它的唯一职责）。

口径（c-arch-13 R11）：
  · 「非测试面直连」= 产品文件 − `tests/` − `task_exec*` − `assembly/**`；**焦点面** = `db_ports/**` + `state.rs`。
  · 「单文件行数」= `is_product()` 网格口径，剔 `SrcClass::TestFixture` 类（解析 `module_size_guard.rs`
    的 `APP_SRC_OWNERSHIP`；解析失败 fail-closed），超标者须在 `single_file_loc_caps`。
  · 「出边数」= 地图 `edges` 中 `from == "server-api"` 的条数，且与 `dependencies` 集合全等。
  · 「直连集中度」= 守恒律（总数）+ 单文件最大 + 落点文件数三量；**枚举由判据输出**（`db_direct_by_file`）。
  · 直连计数口径 = 原始文本中 `easyvibe_db::` 字面量的**逐次出现**（与 `grep` 真值同源；注释中的回指
    亦计入，故 `db_ports/repo.rs` = 3），非「去注释 / 去行」口径——与 V8 命令级复核一致。

本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""
import argparse
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import map_policy  # noqa: E402

APP_CRATE = os.path.join("easyvibe-backend", "crates", "easyvibe-app")
APP_SRC = os.path.join(APP_CRATE, "src")
# c-arch-13：I10 口径来源（TestFixture 类别）+ I11 焦点面（目录化前后兼容）
MODULE_SIZE_GUARD = os.path.join(APP_CRATE, "tests", "module_size_guard.rs")
DB_PORTS_DIR = os.path.join(APP_SRC, "db_ports")
DB_PORTS_FILE = os.path.join(APP_SRC, "db_ports.rs")
STATE_RS = os.path.join(APP_SRC, "state.rs")
DB_DIRECT_LITERAL = "easyvibe_db::"
# c-arch-10：独立装配格（启动装配 + 静态托管；与 server-api 同层，单向外向）
ASSEMBLY_GRID = os.path.join(APP_CRATE, "src", "assembly")
DOMAIN_FILES = ["git.rs", "freshness.rs", "reinduce.rs", "pipeline.rs", "map_concerns.rs"]
EXTRACTED_MODULES = ["easyvibe-git", "easyvibe-pipeline"]
# c-arch-13（2026-10-07）：单点文件分散 + 三项回归闸门落地 ⇒ 纳入闭环判定（只增不减）。
CLOSED_CONCERNS = ("c-arch-1", "c-arch-13")
PRODUCT_NAMES = {"Cargo.toml", "build.rs", "tauri.conf.json", "index.html"}
FIXTURE_DIR = os.path.join("scripts", "tests", "fixtures")
BASELINE_FIXTURE = os.path.join(FIXTURE_DIR, "arch_split_pre.json")
POST_FIXTURE = os.path.join(FIXTURE_DIR, "map_post_split.json")
LOC_MIN_DROP = 1000

# c-arch-7（2026-10-06）登记棘轮上界：见文件头 ★ 说明。只降不升，超出即红。
# c-arch-10（2026-10-07）重登记：bootstrap.rs 纯搬运至 assembly/**（装配格另立棘轮），
#   server-api 因外提而**真降**：files 35→34、loc 6749→6472。
# c-arch-13（2026-10-07）第三次重登记（走 ΔS2 降级支）：db_ports.rs → db_ports/** 目录化
#   （10 文件 = +9）与 service/{patrol,submap,reinduce_start}.rs（+3）⇒ files 34 → 46。
#   同轮必落对冲判据 I9（出边数）/ I10（单文件 ≤400）/ I11a·b·c（直连集中度），否则不受理。
# c-arch-16（2026-10-08）第四次重登记：db_ports 内 task_engine.rs 按端口域拆 4 文件
#   （task_engine_approval.rs / session_attribution.rs / agent_slot.rs，纯搬家零语义新增）
#   ⇒ files 46 → 49；loc 为 src 落齐后由 inventory() 实测（含端口注入面上增加的字段/端口 trait）。
#   同批必落对冲判据 I11b·c 重登记（15 / 13）与 R6 单端口域断言，否则不受理。
C7_RATCHET = {
    "files": 49,
    # loc = 实测（2026-10-08；同事 agent 落齐 13 db_ports 文件 + assembly/ports.rs 后由 inventory() 数准）。
    # ★ 待主 agent 复核：同事 agent 仍在同工作区改 src/**，loc 可能再漂移；R8 终局刷新时以 inventory() 复测为准。
    "loc": 6702,
    "_registered_at": "2026-10-07",
    "_registered_additions": [
        "c-arch-10：bootstrap.rs 外提 assembly/**（server-api −1 文件），装配格另立 C7_ASSEMBLY_RATCHET",
        "c-arch-13：为分散单点新增 3 个文件（service/{patrol,submap,reinduce_start}.rs）；"
        "db_ports.rs → db_ports/** 目录化 +9（10 − 1）⇒ 合计 files 34 → 46；"
        "loc 增量仅为 mod 声明 / 文件头 / import 开销（零逻辑新增），由主 agent 实测校正。",
        "c-arch-16：db_ports/task_engine.rs 按端口域拆 4（+3 文件）⇒ files 46 → 49；"
        "loc 为落盘后实测值（含端口注入面：GitPort/RepoPipelinePort + AppState 字段）。",
        "对冲判据（本轮同时落地，否则不受理）：I9 出边数上界、I10 单文件 ≤400、I11a/b/c 直连集中度、"
        "I13 组合根唯一性/出边登记、R6 db_ports 单端口域断言。",
    ],
}
C7_RATCHET_MAX_FILES = C7_RATCHET["files"]
C7_RATCHET_MAX_LOC = C7_RATCHET["loc"]

# c-arch-10：装配格（assembly/**）棘轮——本轮**新登记**的一格（非抬升既有棘轮）。只降不升。
# c-arch-16：新增 ports.rs（GitAdapter + PipelineAdapter 两个端口适配器）⇒ files 5 → 6；
#   loc 为落盘后实测值（原 746 为 5 文件实测和）。
C7_ASSEMBLY_RATCHET = {
    "files": 6,
    "loc": 857,
    "_registered_at": "2026-10-07",
    "_files": ["mod.rs", "logging.rs", "bridges.rs", "schedulers.rs", "static_host.rs", "ports.rs"],
}


def is_product(rel):
    """server-api 归属口径：产品文件，剔除 `tests/`、task-engine 切片（`task_exec*`）
    与**装配格**（assembly/**，c-arch-10 独立成格）。"""
    if "/tests/" in "/" + rel:
        return False
    if "task_exec" in rel:
        return False
    if rel.startswith(ASSEMBLY_GRID + "/"):
        return False
    return rel.endswith(".rs") or os.path.basename(rel) in PRODUCT_NAMES


def inventory(root):
    """返回 (app crate 归属文件列表, LOC 合计)。"""
    files, loc = [], 0
    base = os.path.join(root, APP_CRATE)
    for dirpath, _dirnames, filenames in os.walk(base):
        if "/target/" in "/" + dirpath.replace(os.sep, "/"):
            continue
        for fn in sorted(filenames):
            rel = os.path.relpath(os.path.join(dirpath, fn), root).replace(os.sep, "/")
            if not is_product(rel):
                continue
            files.append(rel)
            try:
                with open(os.path.join(root, rel), encoding="utf-8", errors="ignore") as fh:
                    loc += sum(1 for _ in fh)
            except OSError:
                pass
    return sorted(files), loc


def inventory_assembly(root):
    """c-arch-10：装配格（`src/assembly/**`）归属文件列表与 LOC 合计。"""
    files, loc = [], 0
    base = os.path.join(root, ASSEMBLY_GRID)
    if not os.path.isdir(base):
        return files, loc
    for dirpath, _dirnames, filenames in os.walk(base):
        if "/target/" in "/" + dirpath.replace(os.sep, "/"):
            continue
        for fn in sorted(filenames):
            if not fn.endswith(".rs"):
                continue
            rel = os.path.relpath(os.path.join(dirpath, fn), root).replace(os.sep, "/")
            files.append(rel)
            try:
                with open(os.path.join(root, rel), encoding="utf-8", errors="ignore") as fh:
                    loc += sum(1 for _ in fh)
            except OSError:
                pass
    return sorted(files), loc


def test_fixture_basenames(root):
    """解析 `tests/module_size_guard.rs::APP_SRC_OWNERSHIP` 中 `SrcClass::TestFixture, "<name>"` 项，
    返回文件 basename 集合（I10 豁免面；沿用既有 ≤600 上限）。

    解析失败 / 为空 = fail-closed（抛 RuntimeError，调用方转 problem）——绝不放行全量。
    """
    p = os.path.join(root, MODULE_SIZE_GUARD)
    try:
        with open(p, encoding="utf-8") as fh:
            txt = fh.read()
    except OSError as e:
        raise RuntimeError("module_size_guard.rs 不可读（I10 口径 fail-closed）: %s" % e)
    names = set(re.findall(r'SrcClass::TestFixture\s*,\s*"([^"]+)"', txt))
    if not names:
        raise RuntimeError("module_size_guard.rs 未解析到 SrcClass::TestFixture 项（I10 口径 fail-closed）")
    return names


def product_file_locs(root):
    """{repo 相对路径: LOC} —— `is_product()` 口径的逐文件行数（I10 断言面）。"""
    out = {}
    base = os.path.join(root, APP_CRATE)
    for dirpath, _dirnames, filenames in os.walk(base):
        if "/target/" in "/" + dirpath.replace(os.sep, "/"):
            continue
        for fn in sorted(filenames):
            rel = os.path.relpath(os.path.join(dirpath, fn), root).replace(os.sep, "/")
            if not is_product(rel):
                continue
            try:
                with open(os.path.join(root, rel), encoding="utf-8", errors="ignore") as fh:
                    out[rel] = sum(1 for _ in fh)
            except OSError:
                out[rel] = 0
    return out


def focus_files(root):
    """焦点面 = `src/db_ports.rs`（目录化前）或 `src/db_ports/**`（目录化后）+ `src/state.rs`。

    以**磁盘实际**为准（目录存在取目录，否则取单文件）；兼容拆分前后同一判据。
    """
    out = []
    d = os.path.join(root, DB_PORTS_DIR)
    f = os.path.join(root, DB_PORTS_FILE)
    if os.path.isdir(d):
        for dirpath, _dirnames, filenames in os.walk(d):
            if "/target/" in "/" + dirpath.replace(os.sep, "/"):
                continue
            for fn in sorted(filenames):
                if not fn.endswith(".rs"):
                    continue
                out.append(os.path.relpath(os.path.join(dirpath, fn), root).replace(os.sep, "/"))
    elif os.path.isfile(f):
        out.append(DB_PORTS_FILE.replace(os.sep, "/"))
    s = os.path.join(root, STATE_RS)
    if os.path.isfile(s):
        out.append(STATE_RS.replace(os.sep, "/"))
    return sorted(out)


def db_direct_by_file(root):
    """{repo 相对焦点面路径: `easyvibe_db::` 出现次数} —— I11 的逐文件真值表（人工可读）。"""
    by = {}
    for rel in focus_files(root):
        try:
            with open(os.path.join(root, rel), encoding="utf-8", errors="ignore") as fh:
                by[rel] = fh.read().count(DB_DIRECT_LITERAL)
        except OSError:
            by[rel] = 0
    return by


def _as_int(v):
    """宽松取整：非整数 / 缺失 → None（由调用方记 problem，fail-closed）。"""
    try:
        return int(v)
    except (TypeError, ValueError):
        return None


def domain_files_present(root):
    """仍在 app crate 内的领域文件（期望空）+ 仍在 main.rs 声明的 mod（期望空）。"""
    present = [f for f in DOMAIN_FILES if os.path.exists(os.path.join(root, APP_SRC, f))]
    main_rs = os.path.join(root, APP_SRC, "main.rs")
    mods = []
    if os.path.isfile(main_rs):
        with open(main_rs, encoding="utf-8") as fh:
            txt = fh.read()
        mods = [f for f in DOMAIN_FILES if ("mod %s;" % f[:-3]) in txt]
    return present, mods


def crossed_scc_edges(m):
    ids = [x["id"] for x in m.get("modules", [])]
    edges = m.get("edges", [])
    layer_order = {x["id"]: x["order"] for x in m.get("layers", [])}
    mod_order = {x["id"]: layer_order.get(x.get("layer")) for x in m.get("modules", [])}
    return map_policy.cross_layer_scc(ids, edges, mod_order)


def check(m, files, loc, baseline, present, mods, emit_modules=None, ratchet=None,
          assembly=None, assembly_ratchet=None, gates=None, loc_by_file=None,
          db_focus=None, test_fixtures=None):
    """纯函数判决（selfcheck 可注入合成输入）。返回 (problems, report)。

    gates/loc_by_file/db_focus 为 c-arch-13 新增输入；为 None 时跳过 I9/I10/I11
    （既有 N1–N9 合成用例据此保持原语义）。
    """
    problems = []
    mods_by_id = {x["id"]: x for x in m.get("modules", [])}
    stats = (m.get("meta") or {}).get("stats") or {}
    ratio = stats.get("coverage_ratio")

    # I1 覆盖率
    if ratio is not None and ratio < 1.0:
        problems.append("I1 coverage_ratio=%s < 1.0（覆盖率跌破 fail-closed 线）" % ratio)

    # I2 文件集收缩 + 领域文件外提
    if ratchet is None:
        if len(files) >= baseline["server_api_files"]:
            problems.append("I2 server-api 归属文件数 %d 未低于基线 %d（领域规则未真正外提）"
                            % (len(files), baseline["server_api_files"]))
    elif len(files) > ratchet["files"]:
        problems.append("I2 server-api 归属文件数 %d 超登记棘轮上限 %d（c-arch-7 登记后只降不升；"
                        "新增文件须登记复核）" % (len(files), ratchet["files"]))
    if present:
        problems.append("I2 app crate 内仍存在领域文件 %s（c-arch-1 复发）" % present)
    if mods:
        problems.append("I2 main.rs 仍声明 mod %s（领域模块未摘除）" % mods)

    # I3 LOC 趋势
    drop = baseline["server_api_loc"] - loc
    if ratchet is None:
        if drop <= 0:
            problems.append("I3 server-api 归属 LOC %d 未低于基线 %d" % (loc, baseline["server_api_loc"]))
        elif drop < LOC_MIN_DROP:
            problems.append("I3 server-api 归属 LOC 仅下降 %d 行（< %d，拆分不充分）" % (drop, LOC_MIN_DROP))
    elif loc > ratchet["loc"]:
        problems.append("I3 server-api 归属 LOC %d 超登记棘轮上限 %d（c-arch-7 登记后只降不升）"
                        % (loc, ratchet["loc"]))

    # I4 concern 闭环
    for c in (m.get("health") or {}).get("concerns", []):
        if c.get("id") in CLOSED_CONCERNS:
            problems.append("I4 架构 concern %s 仍未闭环" % c.get("id"))
    sa = mods_by_id.get("server-api")
    if sa is None:
        problems.append("I4 图内找不到 server-api 模块")
    elif "god_module" in sa.get("health", {}).get("decay_flags", []):
        problems.append("I4 server-api 仍带 god_module 标记")

    # I5 外提模块在册
    for mid in EXTRACTED_MODULES:
        if mid not in mods_by_id:
            problems.append("I5 外提模块 %s 未登记进地图" % mid)
    if emit_modules is not None:
        for mid in EXTRACTED_MODULES:
            if mid not in emit_modules:
                problems.append("I5 外提模块 %s 未登记进 emit_order.json" % mid)

    # I6 图判据（跨层环 / DV 棘轮）
    for g in crossed_scc_edges(m):
        problems.append("I6 跨层 SCC: %s" % g)
    dv = sum(1 for e in m.get("edges", []) if e.get("direction_violation"))
    if dv > 0:
        problems.append("I6 direction_violation %d > 0" % dv)

    # I8 装配格棘轮（c-arch-10）：文件数 / LOC 不得超过登记值（只降不升；新格非抬升既有棘轮）
    if assembly is not None and assembly_ratchet is not None:
        af, aloc = assembly
        if len(af) > assembly_ratchet["files"]:
            problems.append("I8 装配格归属文件数 %d 超登记上限 %d（c-arch-10；只降不升）"
                            % (len(af), assembly_ratchet["files"]))
        if aloc > assembly_ratchet["loc"]:
            problems.append("I8 装配格归属 LOC %d 超登记上限 %d（c-arch-10；只降不升）"
                            % (aloc, assembly_ratchet["loc"]))

    # ---- c-arch-13：出边数（I9）/ 单文件行数（I10）/ 直连集中度（I11a·b·c）----
    server_api_out_edges = None
    max_file_loc = None
    max_file_loc_allowlist = None
    db_total = None
    db_focus_max = None
    db_focus_file_count = None
    db_by_file = None
    # c-arch-16 R5（I13）：组合根出边登记 / 入边（唯一性）
    assembly_out_edges = None
    assembly_in_edges = None

    if gates is not None:
        sa_mod = mods_by_id.get("server-api") or {}

        # I9：出边数上界 + 目标集 == dependencies（INV-1）== 登记表（不升 + 逐条登记）
        out_edges = [e for e in m.get("edges", []) if e.get("from") == "server-api"]
        targets = {e.get("to") for e in out_edges}
        deps = set(sa_mod.get("dependencies") or [])
        server_api_out_edges = sorted(targets)
        out_max = _as_int(gates.get("server_api_out_edges_max"))
        if out_max is None:
            problems.append("I9 gates.server_api_out_edges_max 缺失/非整数（fail-closed）")
        elif len(out_edges) > out_max:
            problems.append("I9 server-api 出边数 %d > 上限 %d（只降不升；新增出边须显式重登记）"
                            % (len(out_edges), out_max))
        if targets != deps:
            problems.append("I9 INV-1：modules[server-api].dependencies %s ≠ 出边目标集 %s"
                            % (sorted(deps), sorted(targets)))
        declared_edges = set(gates.get("server_api_out_edges") or [])
        if targets != declared_edges:
            problems.append("I9 出边目标集漂移：实际 %s ≠ 登记 %s（新增/替换须重登记）"
                            % (sorted(targets), sorted(declared_edges)))

        # I10：非 TestFixture 产品文件 ≤ single_file_loc_max；棘轮表键集 == 实际超标集（双向全等）
        if loc_by_file is not None:
            caps = dict(gates.get("single_file_loc_caps") or {})
            max_file_loc_allowlist = caps
            loc_max = _as_int(gates.get("single_file_loc_max"))
            if loc_max is None:
                problems.append("I10 gates.single_file_loc_max 缺失/非整数（fail-closed）")
            else:
                fixtures = set(test_fixtures or ())
                overs = []
                max_file_loc = 0
                for rel in files:
                    if os.path.basename(rel) in fixtures:
                        continue
                    n = int(loc_by_file.get(rel, 0))
                    max_file_loc = max(max_file_loc, n)
                    cap = int(caps.get(rel, loc_max))
                    if n > cap:
                        problems.append("I10 %s = %d 行 > 上限 %d（非 TestFixture 产品文件须 ≤%d；"
                                        "存量超标须登记 single_file_loc_caps）" % (rel, n, cap, loc_max))
                    if n > loc_max:
                        overs.append(rel)
                if set(overs) != set(caps):
                    problems.append("I10 棘轮表键集 ≠ 实际超标集：实际 %s ≠ 登记 %s（摘牌/登记漂移）"
                                    % (sorted(overs), sorted(caps)))

        # I11a/b/c：焦点面守恒律 + 单文件最大（双边全等）+ 落点文件数
        if db_focus is not None:
            db_by_file = dict(db_focus)
            db_total = sum(db_by_file.values())
            db_focus_max = max(db_by_file.values()) if db_by_file else 0
            db_focus_file_count = len([1 for v in db_by_file.values() if v > 0])
            want_total = _as_int(gates.get("db_direct_total"))
            want_max = _as_int(gates.get("db_direct_focus_max"))
            want_files = _as_int(gates.get("db_direct_focus_files_max"))
            if want_total is None:
                problems.append("I11 gates.db_direct_total 缺失/非整数（fail-closed）")
            elif db_total != want_total:
                problems.append("I11a 焦点面直连总处数 %d ≠ 登记 %d（守恒律破：搬运丢行/重复/新写直连）"
                                % (db_total, want_total))
            if want_max is None:
                problems.append("I11 gates.db_direct_focus_max 缺失/非整数（fail-closed）")
            elif db_focus_max != want_max:
                problems.append("I11b 单文件最大直连数 %d ≠ 登记 %d（双边全等，须同步登记）"
                                % (db_focus_max, want_max))
            if want_files is None:
                problems.append("I11 gates.db_direct_focus_files_max 缺失/非整数（fail-closed）")
            elif db_focus_file_count != want_files:
                problems.append("I11c 有直连的焦点面文件数 %d ≠ 登记 %d（落点面漂移）"
                                % (db_focus_file_count, want_files))

        # I13（c-arch-16 R5）：组合根唯一性 + 组合根出边逐条登记
        #   I13a 图内无指向 assembly 的入边（组合根无上游；违反即红）
        #   I13b gates.assembly_out_edges 存在且非空（缺失/空 ⇒ fail-closed），
        #        且 assembly 出边目标集 == 登记集（允许增长但禁静默，新增/替换须显式重登记）
        #   I13c 语义（注释，非可执行判据）：判据对象是「**边界/编排层**（server-api）的出边数」，
        #        不是「谁度数居首」。组合根知道全部后端域是正当的（这正是它的唯一职责）；
        #        console-ui 出边 11 是 c-arch-14 已登记的呈现层代价。
        #        与 verify_map_acyclic D（dependencies == 出边目标集）职责不同：D 管图内自洽，
        #        I13b 管与 policy 登记表同代——两者叠加，缺一不可。
        assembly_in = [e for e in m.get("edges", []) if e.get("to") == "assembly"]
        assembly_out_edges = sorted({e.get("to") for e in m.get("edges", []) if e.get("from") == "assembly"})
        assembly_in_edges = sorted(e.get("from") for e in assembly_in)
        if assembly_in:
            problems.append("I13a 组合根 assembly 出现入边 %s——唯一组合根被破坏（无上游）"
                            % assembly_in_edges)
        declared_asm = gates.get("assembly_out_edges")
        if not declared_asm:
            problems.append("I13b gates.assembly_out_edges 缺失/为空（fail-closed；组合根出边须逐条登记）")
        elif set(assembly_out_edges) != set(declared_asm):
            problems.append("I13b 组合根出边集漂移：实际 %s ≠ 登记 %s（允许增长但禁静默，须显式重登记）"
                            % (assembly_out_edges, sorted(declared_asm)))

    report = {
        "server_api_files": len(files), "server_api_files_baseline": baseline["server_api_files"],
        "server_api_loc": loc, "server_api_loc_baseline": baseline["server_api_loc"],
        "loc_drop": baseline["server_api_loc"] - loc,
        "loc_drop_vs_pre_split": baseline["server_api_loc"] - loc,
        "ratchet_files_max": (ratchet or {}).get("files"),
        "ratchet_loc_max": (ratchet or {}).get("loc"),
        "coverage_ratio": ratio, "modules": len(m.get("modules", [])), "edges": len(m.get("edges", [])),
        "cross_layer_scc": len(crossed_scc_edges(m)), "direction_violation": dv,
        # c-arch-10 装配格口径（G6 两指标可复读：files / loc 只降不升）
        "assembly_files": (len(assembly[0]) if assembly else None),
        "assembly_loc": (assembly[1] if assembly else None),
        "assembly_ratchet_files_max": (assembly_ratchet or {}).get("files"),
        "assembly_ratchet_loc_max": (assembly_ratchet or {}).get("loc"),
        # I7 趋势双口径：地图自评分（本文件）+ 巡检分（HealthPage 从库内巡检记录读取，按测量时间新旧裁决）
        "map_self_score": (m.get("health") or {}).get("score"),
        "patrol_score": "见 HealthPage（巡检分存于库内 patrol_runs，按测量时间新旧裁决，不写回地图）",
        "server_api_decay_flags": (sa or {}).get("health", {}).get("decay_flags", []),
        "arch_concerns": [c.get("id") for c in (m.get("health") or {}).get("concerns", [])],
        # c-arch-13 口径面（R11 自述）：出边登记 / 单文件行数 / 直连集中度（枚举由判据产出）
        "server_api_out_edges": server_api_out_edges,
        "max_file_loc": max_file_loc,
        "max_file_loc_allowlist": max_file_loc_allowlist,
        "db_direct_total": db_total,
        "db_direct_focus_max": db_focus_max,
        "db_direct_focus_files_max": db_focus_file_count,
        "db_direct_by_file": db_by_file,
        # c-arch-16 R5：组合根（assembly）出边登记集（I13b）与入边（I13a，期望空）
        "assembly_out_edges": assembly_out_edges,
        "assembly_in_edges": assembly_in_edges,
    }
    return problems, report


def synthetic_case(present=(), mods=(), coverage=1.0, concerns=(), decay=None, cyclic=False):
    """selfcheck 用合成输入：不读仓库、不依赖 live map。"""
    baseline = {"server_api_files": 5, "server_api_loc": 4000}
    files = ["easyvibe-backend/crates/easyvibe-app/Cargo.toml",
             "easyvibe-backend/crates/easyvibe-app/src/main.rs",
             "easyvibe-backend/crates/easyvibe-app/src/routes/repo.rs",
             "easyvibe-backend/crates/easyvibe-app/src/service/git.rs"]
    if present:
        files.append("easyvibe-backend/crates/easyvibe-app/src/%s" % present[0])
    modules = [
        {"id": "server-api", "layer": "application", "dependencies": ["contract-foundation"],
         "health": {"decay_flags": list(decay or ["coupling_high"]), "concerns": []}},
        {"id": "easyvibe-git", "layer": "application", "dependencies": ["contract-foundation"],
         "health": {"decay_flags": [], "concerns": []}},
        {"id": "easyvibe-pipeline", "layer": "application", "dependencies": ["contract-foundation"],
         "health": {"decay_flags": [], "concerns": []}},
        {"id": "contract-foundation", "layer": "foundation", "dependencies": [],
         "health": {"decay_flags": [], "concerns": []}},
    ]
    edges = [
        {"id": "e1", "from": "server-api", "to": "contract-foundation"},
        {"id": "e2", "from": "easyvibe-git", "to": "contract-foundation"},
        {"id": "e3", "from": "easyvibe-pipeline", "to": "contract-foundation"},
    ]
    if cyclic:
        edges.append({"id": "e4", "from": "contract-foundation", "to": "server-api"})
    m = {"layers": [{"id": "application", "order": 3}, {"id": "foundation", "order": 7}],
         "modules": modules, "edges": edges,
         "meta": {"stats": {"coverage_ratio": coverage}},
         "health": {"score": 85, "concerns": [{"id": c} for c in concerns]}}
    return check(m, files, 1500, baseline, list(present), list(mods), emit_modules=EXTRACTED_MODULES)


# ---- c-arch-13：I9/I10/I11 的合成输入（不读仓库、不依赖 live map）----
# 合成 gates：出边登记 1 条（与合成图一致）、单文件上限 400（无棘轮）、焦点面 91/22/10。
# c-arch-16 R5：再补组合根出边登记 1 条（与合成图一致；I13b 正例所需）。
BASE_GATES = {
    "server_api_out_edges_max": 1,
    "server_api_out_edges": ["contract-foundation"],
    "assembly_out_edges": ["contract-foundation"],
    "single_file_loc_max": 400,
    "single_file_loc_caps": {},
    "db_direct_total": 91,
    "db_direct_focus_max": 22,
    "db_direct_focus_files_max": 10,
}


def focus_ok():
    """10 文件 / 每文件 ≤22 / 总数 91 的合成焦点面（I11 正例；max 恰为 22）。"""
    counts = [22, 15, 14, 11, 9, 6, 5, 4, 3, 2]
    return {"db_ports/f%d.rs" % i: c for i, c in enumerate(counts)}


def gates_case(gates, edges_extra=None, files=None, loc_by_file=None,
               db_focus=None, test_fixtures=()):
    """selfcheck 合成输入（I9/I10/I11/I13）：合成图 + gates 注入，可逐面注入漂移。

    c-arch-16 R5：合成图纳入组合根 `assembly`（零入边）与其一条出边（与 BASE_GATES 登记一致），
    使 I13a/I13b 的正负例不依赖仓库状态。
    """
    modules = [
        {"id": "server-api", "layer": "application", "dependencies": ["contract-foundation"],
         "health": {"decay_flags": [], "concerns": []}},
        {"id": "easyvibe-git", "layer": "application", "dependencies": [],
         "health": {"decay_flags": [], "concerns": []}},
        {"id": "easyvibe-pipeline", "layer": "application", "dependencies": [],
         "health": {"decay_flags": [], "concerns": []}},
        {"id": "assembly", "layer": "application", "dependencies": ["contract-foundation"],
         "health": {"decay_flags": [], "concerns": []}},
        {"id": "contract-foundation", "layer": "foundation", "dependencies": [],
         "health": {"decay_flags": [], "concerns": []}},
    ]
    edges = [
        {"id": "e1", "from": "server-api", "to": "contract-foundation"},
        {"id": "eA", "from": "assembly", "to": "contract-foundation"},
    ]
    if edges_extra:
        edges.append(edges_extra)
    m = {"layers": [{"id": "application", "order": 3}, {"id": "foundation", "order": 7}],
         "modules": modules, "edges": edges,
         "meta": {"stats": {"coverage_ratio": 1.0}}, "health": {"score": 80, "concerns": []}}
    return check(m, files if files is not None else ["a.rs"], 100,
                 {"server_api_files": 1, "server_api_loc": 9999}, [], [],
                 emit_modules=EXTRACTED_MODULES, ratchet={"files": 100, "loc": 99999},
                 gates=gates, loc_by_file=loc_by_file, db_focus=db_focus,
                 test_fixtures=test_fixtures)


def selfcheck():
    results = []
    p1, _ = synthetic_case(present=["git.rs"])
    results.append(("N1 server-api 仍含 git.rs → 必红", any(x.startswith("I2") for x in p1), "; ".join(p1[:1])))
    p2, _ = synthetic_case(coverage=0.95)
    results.append(("N2 覆盖率 0.95 → 必红", any(x.startswith("I1") for x in p2), "; ".join(p2[:1])))
    p3, _ = synthetic_case(concerns=["c-arch-1"])
    results.append(("N3 concerns 含 c-arch-1 → 必红", any(x.startswith("I4") for x in p3), "; ".join(p3[:1])))
    p4, _ = synthetic_case(decay=["god_module"])
    results.append(("N4 server-api 带 god_module → 必红", any(x.startswith("I4") for x in p4), "; ".join(p4[:1])))
    p5, _ = synthetic_case(cyclic=True)
    results.append(("N5 合成跨层环 → 必红", any(x.startswith("I6") for x in p5), "; ".join(p5[:1])))
    p6, _ = synthetic_case(mods=["reinduce.rs"])
    results.append(("N6 main.rs 仍 mod reinduce → 必红", any(x.startswith("I2") for x in p6), "; ".join(p6[:1])))
    p7, _ = synthetic_case()
    results.append(("N7 正例 → 必绿", not p7, "; ".join(p7[:1])))
    # N8/N9 装配格棘轮（c-arch-10 I8）：超限必红 / 在限内必绿
    m8 = {"layers": [{"id": "application", "order": 3}], "modules": [
        {"id": "server-api", "layer": "application", "dependencies": [], "health": {"decay_flags": [], "concerns": []}},
        {"id": "easyvibe-git", "layer": "application", "dependencies": [], "health": {"decay_flags": [], "concerns": []}},
        {"id": "easyvibe-pipeline", "layer": "application", "dependencies": [], "health": {"decay_flags": [], "concerns": []}},
    ], "edges": [], "meta": {"stats": {"coverage_ratio": 1.0}}, "health": {"score": 80, "concerns": []}}
    bl = {"server_api_files": 5, "server_api_loc": 4000}
    asm_ratchet = {"files": 5, "loc": 746}
    p8, _ = check(m8, ["a.rs"], 100, bl, [], [], emit_modules=EXTRACTED_MODULES,
                  ratchet={"files": 40, "loc": 9000}, assembly=(["x/assembly/a.rs"], 99999),
                  assembly_ratchet=asm_ratchet)
    results.append(("N8 装配格超棘轮 → 必红", any(x.startswith("I8") for x in p8), "; ".join(p8[:1])))
    p9, _ = check(m8, ["a.rs"], 100, bl, [], [], emit_modules=EXTRACTED_MODULES,
                  ratchet={"files": 40, "loc": 9000}, assembly=(["x/assembly/a.rs"], 700),
                  assembly_ratchet=asm_ratchet)
    results.append(("N9 装配格在棘轮内 → 必绿", not p9, "; ".join(p9[:1])))

    # ---- c-arch-13：I9 / I10 / I11a·b·c 自检（合成输入，不改仓库）----
    # N10 I9 正例（出边 == 登记集）
    p10, _ = gates_case(BASE_GATES)
    results.append(("N10 出边 == 登记集 → I9 必绿", not p10, "; ".join(p10[:1])))
    # N11 I9 负例（追加一条 server-api → X 出边）
    p11, _ = gates_case(BASE_GATES,
                        edges_extra={"id": "eX", "from": "server-api", "to": "contract-foundation2"})
    results.append(("N11 追加 server-api → X 出边 → I9 必红",
                    any(x.startswith("I9") for x in p11), "; ".join(p11[:1])))
    # N12 I10 负例（合成 401 行产品文件；无棘轮登记 ⇒ 超标 + 键集漂移）
    r401 = "easyvibe-backend/crates/easyvibe-app/src/service/x.rs"
    p12, _ = gates_case(BASE_GATES, files=[r401], loc_by_file={r401: 401})
    results.append(("N12 合成 401 行产品文件 → I10 必红",
                    any(x.startswith("I10") for x in p12), "; ".join(p12[:1])))
    # N13 I10 正例（400 行正好在限内，棘轮空表 ↔ 无超标）
    p13, _ = gates_case(BASE_GATES, files=["a.rs"], loc_by_file={"a.rs": 400})
    results.append(("N13 合成 400 行产品文件 → I10 必绿", not p13, "; ".join(p13[:1])))
    # N14 I11 正例（10 文件 / 最大 22 / 总数 91）
    p14, _ = gates_case(BASE_GATES, db_focus=focus_ok())
    results.append(("N14 焦点面 10 文件/最大 22/总数 91 → I11 必绿", not p14, "; ".join(p14[:1])))
    # N15 I11b 负例（单文件 23 处；总数守恒 91 不变）
    bad_b = dict(focus_ok())
    bad_b["db_ports/f0.rs"] = 23
    bad_b["db_ports/f1.rs"] = 14
    p15, _ = gates_case(BASE_GATES, db_focus=bad_b)
    results.append(("N15 单文件 23 处直连 → I11b 必红",
                    any(x.startswith("I11b") for x in p15), "; ".join(p15[:1])))
    # N16 I11a 负例（总数 92）
    bad_a = dict(focus_ok())
    bad_a["db_ports/f9.rs"] += 1
    p16, _ = gates_case(BASE_GATES, db_focus=bad_a)
    results.append(("N16 焦点面总数 92 → I11a 必红",
                    any(x.startswith("I11a") for x in p16), "; ".join(p16[:1])))
    # N17 I11c 负例（落点 11 文件；总数守恒 91、最大 22 不变）
    bad_c = dict(focus_ok())
    bad_c["db_ports/f9.rs"] -= 1
    bad_c["db_ports/f10.rs"] = 1
    p17, _ = gates_case(BASE_GATES, db_focus=bad_c)
    results.append(("N17 焦点面落点 11 文件 → I11c 必红",
                    any(x.startswith("I11c") for x in p17), "; ".join(p17[:1])))
    # N18 I2 负例（文件数超 C7 棘轮）
    p18, _ = check(m8, ["a.rs", "b.rs", "c.rs"], 100, bl, [], [], emit_modules=EXTRACTED_MODULES,
                   ratchet={"files": 2, "loc": 9000})
    results.append(("N18 文件数超 C7 棘轮 → I2 必红",
                    any(x.startswith("I2") for x in p18), "; ".join(p18[:1])))

    # ---- c-arch-16 R5：I13 组合根唯一性 + 出边登记 自检 ----
    # N19 I13a 负例（注入 X → assembly 入边；组合根出现上游）
    p19, _ = gates_case(BASE_GATES,
                        edges_extra={"id": "eBad", "from": "easyvibe-git", "to": "assembly"})
    results.append(("N19 注入 X → assembly 入边 → I13a 必红",
                    any(x.startswith("I13a") for x in p19), "; ".join(p19[:1])))
    # N20 I13b 负例（登记集漂移：多登记一个图内不存在的目标）
    g20 = dict(BASE_GATES)
    g20["assembly_out_edges"] = ["contract-foundation", "map-domain"]
    p20, _ = gates_case(g20)
    results.append(("N20 组合根出边集与登记漂移 → I13b 必红",
                    any(x.startswith("I13b") for x in p20), "; ".join(p20[:1])))
    # N21 I13 正例（一致的 assembly 出边 + 零入边）
    p21, _ = gates_case(BASE_GATES)
    results.append(("N21 一致组合根出边 + 零入边 → 必绿",
                    not any(x.startswith("I13") for x in p21), "; ".join(p21[:1])))
    # N22 I13b fail-closed（gates.assembly_out_edges 缺失）
    g22 = {k: v for k, v in BASE_GATES.items() if k != "assembly_out_edges"}
    p22, _ = gates_case(g22)
    results.append(("N22 gates.assembly_out_edges 缺失 → I13b 必红（fail-closed）",
                    any(x.startswith("I13b") for x in p22), "; ".join(p22[:1])))

    ok = True
    for name, passed, detail in results:
        ok = ok and passed
        print("  %s %s  %s" % ("PASS" if passed else "FAIL", name, detail))
    return ok


def main():
    ap = argparse.ArgumentParser(description="c-arch-1 边界/领域分离验收（I1–I13）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--map", default=None)
    ap.add_argument("--root", default=os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    args = ap.parse_args()
    root = os.path.abspath(args.root)
    if args.selfcheck:
        print("▶ 边界/领域分离验收自检 N1–N22")
        return 0 if selfcheck() else 1

    with open(os.path.join(root, BASELINE_FIXTURE), encoding="utf-8") as fh:
        baseline = json.load(fh)
    live = args.map is None
    path = args.map or os.path.join(root, ".easyvibe", "map", "map.json")
    if not os.path.isfile(path):
        print(json.dumps({"ok": False, "problems": ["map not found: %s" % path]}, ensure_ascii=False))
        return 1
    with open(path, encoding="utf-8") as fh:
        m = json.load(fh)
    files, loc = inventory(root)
    assembly = inventory_assembly(root)
    present, mods = domain_files_present(root)
    emit_modules = None
    if live:
        order_path = os.path.join(root, ".easyvibe", "map", "emit_order.json")
        if os.path.isfile(order_path):
            with open(order_path, encoding="utf-8") as fh:
                emit_modules = json.load(fh).get("modules", [])
    # c-arch-13：gates（I9/I10/I11）+ 逐文件行数 + 焦点面直连真值表 + TestFixture 豁免集
    policy = map_policy.load_policy(root)
    gates = policy.get("gates") or {}
    loc_by_file = product_file_locs(root)
    db_focus = db_direct_by_file(root)
    fixture_err = None
    try:
        fixtures = test_fixture_basenames(root)
    except RuntimeError as e:  # fail-closed：口径解析失败即红，绝不放行全量
        fixtures = set()
        fixture_err = str(e)
    problems, report = check(m, files, loc, baseline, present, mods, emit_modules,
                            ratchet={"files": C7_RATCHET_MAX_FILES, "loc": C7_RATCHET_MAX_LOC},
                            assembly=assembly, assembly_ratchet=C7_ASSEMBLY_RATCHET,
                            gates=gates, loc_by_file=loc_by_file, db_focus=db_focus,
                            test_fixtures=fixtures)
    if fixture_err:
        problems.append("I10 %s" % fixture_err)
    report["test_fixture_basenames"] = sorted(fixtures) if not fixture_err else None
    print(json.dumps({"ok": not problems, **report, "problems": problems}, ensure_ascii=False, indent=2))
    return 0 if not problems else 1


if __name__ == "__main__":
    sys.exit(main())
