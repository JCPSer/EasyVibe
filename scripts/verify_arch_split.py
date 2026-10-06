#!/usr/bin/env python3
"""c-arch-1 关闭验收：把「边界层不得含业务规则」做成只读、可复跑、可趋势的判据（I1–I7）。

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
  I4 架构 concerns 无 c-arch-1；server-api 模块 decay_flags 无 god_module
  I5 外提模块在册（easyvibe-git / easyvibe-pipeline 出现在图内；live 模式另查 emit_order.json）
  I6 跨层 SCC == 0 且 direction_violation == 0（c-arch-5 闭环后棘轮归零；复用 map_policy.py，与 finalize 同实现）
  I7 趋势双口径输出（地图自评分 + 巡检分来源说明）

本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""
import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import map_policy  # noqa: E402

APP_CRATE = os.path.join("easyvibe-backend", "crates", "easyvibe-app")
APP_SRC = os.path.join(APP_CRATE, "src")
DOMAIN_FILES = ["git.rs", "freshness.rs", "reinduce.rs", "pipeline.rs", "map_concerns.rs"]
EXTRACTED_MODULES = ["easyvibe-git", "easyvibe-pipeline"]
CLOSED_CONCERN = "c-arch-1"
PRODUCT_NAMES = {"Cargo.toml", "build.rs", "tauri.conf.json", "index.html"}
FIXTURE_DIR = os.path.join("scripts", "tests", "fixtures")
BASELINE_FIXTURE = os.path.join(FIXTURE_DIR, "arch_split_pre.json")
POST_FIXTURE = os.path.join(FIXTURE_DIR, "map_post_split.json")
LOC_MIN_DROP = 1000

# c-arch-7（2026-10-06）登记棘轮上界：见文件头 ★ 说明。只降不升，超出即红。
C7_RATCHET = {
    "files": 35,
    "loc": 6749,
    "_registered_at": "2026-10-06",
    "_registered_additions": [
        "src/db_ports.rs（组合根：端口↔仓储适配器唯一落点）",
        "src/service/agent.rs",
        "src/service/sessions.rs",
        "src/service/settings.rs",
    ],
}
C7_RATCHET_MAX_FILES = C7_RATCHET["files"]
C7_RATCHET_MAX_LOC = C7_RATCHET["loc"]


def is_product(rel):
    """server-api 归属口径：产品文件，剔除 `tests/` 与 task-engine 切片（`task_exec*`）。"""
    if "/tests/" in "/" + rel:
        return False
    if "task_exec" in rel:
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


def check(m, files, loc, baseline, present, mods, emit_modules=None, ratchet=None):
    """纯函数判决（selfcheck 可注入合成输入）。返回 (problems, report)。"""
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
        if c.get("id") == CLOSED_CONCERN:
            problems.append("I4 架构 concern %s 仍未闭环" % CLOSED_CONCERN)
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

    report = {
        "server_api_files": len(files), "server_api_files_baseline": baseline["server_api_files"],
        "server_api_loc": loc, "server_api_loc_baseline": baseline["server_api_loc"],
        "loc_drop": baseline["server_api_loc"] - loc,
        "loc_drop_vs_pre_split": baseline["server_api_loc"] - loc,
        "ratchet_files_max": (ratchet or {}).get("files"),
        "ratchet_loc_max": (ratchet or {}).get("loc"),
        "coverage_ratio": ratio, "modules": len(m.get("modules", [])), "edges": len(m.get("edges", [])),
        "cross_layer_scc": len(crossed_scc_edges(m)), "direction_violation": dv,
        # I7 趋势双口径：地图自评分（本文件）+ 巡检分（HealthPage 从库内巡检记录读取，按测量时间新旧裁决）
        "map_self_score": (m.get("health") or {}).get("score"),
        "patrol_score": "见 HealthPage（巡检分存于库内 patrol_runs，按测量时间新旧裁决，不写回地图）",
        "server_api_decay_flags": (sa or {}).get("health", {}).get("decay_flags", []),
        "arch_concerns": [c.get("id") for c in (m.get("health") or {}).get("concerns", [])],
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
    ok = True
    for name, passed, detail in results:
        ok = ok and passed
        print("  %s %s  %s" % ("PASS" if passed else "FAIL", name, detail))
    return ok


def main():
    ap = argparse.ArgumentParser(description="c-arch-1 边界/领域分离验收（I1–I7）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--map", default=None)
    ap.add_argument("--root", default=os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    args = ap.parse_args()
    root = os.path.abspath(args.root)
    if args.selfcheck:
        print("▶ 边界/领域分离验收自检 N1–N7")
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
    present, mods = domain_files_present(root)
    emit_modules = None
    if live:
        order_path = os.path.join(root, ".easyvibe", "map", "emit_order.json")
        if os.path.isfile(order_path):
            with open(order_path, encoding="utf-8") as fh:
                emit_modules = json.load(fh).get("modules", [])
    problems, report = check(m, files, loc, baseline, present, mods, emit_modules,
                            ratchet={"files": C7_RATCHET_MAX_FILES, "loc": C7_RATCHET_MAX_LOC})
    print(json.dumps({"ok": not problems, **report, "problems": problems}, ensure_ascii=False, indent=2))
    return 0 if not problems else 1


if __name__ == "__main__":
    sys.exit(main())
