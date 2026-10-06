#!/usr/bin/env python3
"""地图无环验收（R6）：跨层环 / 构建期边 / 不变量，一键可判、可复跑、可趋势。

  --check [--map PATH]   默认读 live `.easyvibe/map/map.json`（本地/归纳期真值）；
                         CI 传 `--map scripts/tests/fixtures/map_post_split.json`（受版本控制，
                         与 EXPECT_EDGES 同代：settings 域析出后的 20 模块 / 52 边快照）。
                         fixture 是**结构投影**：只保留本文件与 verify_arch_split 断言的字段
                         （模块 id/name/layer/files/dependencies/health 与 concern 的 id+severity、
                         边的 id/from/to/type/strength/DV），prose（review_note / concern 正文 /
                         edge label / key_entries / responsibility）不入 fixture——prose 会点名
                         受管资产（如仓库根 schema 文件名），写进受版本控制面即违反
                         `verify_assets.py --forbid-literals` 的「受管名唯一合法落点」不变量。
  --selfcheck            负例自证，**不依赖 live map**（CI 安全）

断言：
  A. 构建期产物托管/内嵌关系不得入 edges（读 scripts/map_edge_policy.json）
  B. 已退役边 id 不得出现（读同目录 emit_order.json，若存在）
  C. desktop-shell.dependencies == ["server-api","map-toolchain"]
  D. INV-1：所有模块 dependencies == 其出边目标集
  E. SCC(>1) == 0（Tarjan，同前端口径）
  F. direction_violation 计数 <= 1（棘轮不增）
  G. 边数 == 期望（live 图默认 52；`--map <fixture>` 时**默认不绑定**——fixture 是历史
     快照，需断言时显式传 `--expect-edges <n>`）
  H. health.concerns 不含 c-arch-2（环已闭环）

本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""
import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import map_policy  # noqa: E402

EXPECT_EDGES = 52
DV_MAX = 1
EXPECT_SHELL_DEPS = ["server-api", "map-toolchain"]
RETIRED_CONCERN = "c-arch-2"
FIXTURE_DIR = os.path.join("scripts", "tests", "fixtures")


def check_map(m, root, map_path, expect_edges=EXPECT_EDGES):
    problems = []
    policy = map_policy.load_policy(root)
    modules = m.get("modules", [])
    edges = m.get("edges", [])
    mods = {x["id"]: x for x in modules}
    ids = [x["id"] for x in modules]
    derived = {}
    for e in edges:
        derived.setdefault(e.get("from"), []).append(e.get("to"))

    # A 构建期关系不得成边
    for eid, frm, to in map_policy.policy_violations(edges, policy):
        problems.append("A 构建期产物托管关系入了 edges: %s %s→%s" % (eid, frm, to))
    # B 退役边 id 不得出现
    if map_path:
        mo = os.path.join(os.path.dirname(os.path.abspath(map_path)), "emit_order.json")
        if os.path.isfile(mo):
            retired = json.load(open(mo, encoding="utf-8")).get("retired_edge_ids", [])
            present = {e.get("id") for e in edges}
            for rid in retired:
                if rid in present:
                    problems.append("B 退役边 id 仍在图内: %s" % rid)
    # C 壳依赖口径
    if "desktop-shell" in mods:
        shell_deps = mods["desktop-shell"].get("dependencies", [])
        if shell_deps != EXPECT_SHELL_DEPS:
            problems.append("C desktop-shell.dependencies=%s != %s" % (shell_deps, EXPECT_SHELL_DEPS))
    # D INV-1
    for mid in ids:
        want = derived.get(mid, [])
        got = mods[mid].get("dependencies", [])
        if got != want:
            problems.append("D deps!=edges: %s %s vs %s" % (mid, got, want))
    # E SCC(>1) 归零
    for g in map_policy.scc_groups(ids, edges):
        problems.append("E SCC(>1): %s" % g)
    # F DV 棘轮
    dv = sum(1 for e in edges if e.get("direction_violation"))
    if dv > DV_MAX:
        problems.append("F direction_violation %d > %d" % (dv, DV_MAX))
    # G 边数
    if expect_edges is not None and len(edges) != expect_edges:
        problems.append("G edges=%d != %d" % (len(edges), expect_edges))
    # H c-arch-2 闭环
    for c in m.get("health", {}).get("concerns", []):
        if c.get("id") == RETIRED_CONCERN:
            problems.append("H 未闭环 concern: %s" % RETIRED_CONCERN)
    return problems


def summarize(m, problems):
    return {
        "ok": not problems,
        "modules": len(m.get("modules", [])),
        "edges": len(m.get("edges", [])),
        "scc_gt1": len(map_policy.scc_groups([x["id"] for x in m.get("modules", [])], m.get("edges", []))),
        "direction_violation": sum(1 for e in m.get("edges", []) if e.get("direction_violation")),
        "problems": problems,
    }


def run_check(root, path, expect_edges=EXPECT_EDGES, quiet=False):
    m = json.load(open(path, encoding="utf-8"))
    problems = check_map(m, root, path, expect_edges)
    summary = summarize(m, problems)
    if not quiet:
        print(json.dumps(summary, ensure_ascii=False))
    return summary


def selfcheck(root):
    results = []
    pre = json.load(open(os.path.join(root, FIXTURE_DIR, "map_pre_migration.json"), encoding="utf-8"))
    post_path = os.path.join(root, FIXTURE_DIR, "map_post_migration.json")
    post = json.load(open(post_path, encoding="utf-8"))

    # fixture 自证不绑定 live 边数（EXPECT_EDGES 随地图生长而变，fixture 是历史快照）
    s0 = summarize(pre, check_map(pre, root, None, None))
    results.append(("H0 删前态 fixture → 必红", not s0["ok"], "; ".join(s0["problems"][:2])))

    s1 = summarize(post, check_map(post, root, post_path, None))
    results.append(("H1 删后态 fixture → 必绿", s1["ok"], "; ".join(s1["problems"][:2])))

    inj = json.loads(json.dumps(post))
    inj["edges"].append({"id": "e31", "from": "desktop-shell", "to": "console-ui",
                         "type": "config", "direction_violation": False})
    s2 = summarize(inj, check_map(inj, root, None, None))
    hit = any(p.startswith("A ") for p in s2["problems"]) and any(p.startswith("E ") for p in s2["problems"])
    results.append(("H2 注入 e31 → 必红（策略 + 跨层 SCC）",
                    (not s2["ok"]) and hit, "; ".join(s2["problems"][:2])))

    cyc = {
        "layers": [{"id": "app-entry", "order": 0}, {"id": "presentation", "order": 1}],
        "modules": [{"id": "mod-a", "layer": "app-entry", "dependencies": ["mod-b"]},
                    {"id": "mod-b", "layer": "presentation", "dependencies": ["mod-a"]}],
        "edges": [{"id": "ex1", "from": "mod-a", "to": "mod-b", "type": "call"},
                  {"id": "ex2", "from": "mod-b", "to": "mod-a", "type": "call"}],
        "health": {"concerns": []},
    }
    s3 = summarize(cyc, check_map(cyc, root, None, None))
    results.append(("H3 合成跨层 SCC → 必红（门禁独立生效）",
                    (not s3["ok"]) and any(p.startswith("E ") for p in s3["problems"]),
                    "; ".join(s3["problems"][:2])))

    ok = True
    for name, passed, detail in results:
        ok = ok and passed
        print("  %s %s  %s" % ("PASS" if passed else "FAIL", name, detail))
    return ok


def main():
    ap = argparse.ArgumentParser(description="地图无环验收（R6）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--map", default=None)
    ap.add_argument("--expect-edges", type=int, default=None,
                    help="显式期望边数（对 fixture 亦生效）；缺省时仅 live map 绑定 EXPECT_EDGES，"
                         "fixture 是历史快照故不绑定（与 selfcheck 同口径）")
    ap.add_argument("--root", default=os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    args = ap.parse_args()
    root = os.path.abspath(args.root)
    if args.selfcheck:
        print("▶ 无环验收自检 H0–H3")
        return 0 if selfcheck(root) else 1
    live = args.map is None
    path = args.map or os.path.join(root, ".easyvibe", "map", "map.json")
    if not os.path.isfile(path):
        print(json.dumps({"ok": False, "problems": ["map not found: %s" % path]}, ensure_ascii=False))
        return 1
    # 边数（G）是 live 图的棘轮：fixture 只做 A–F/H 结构判据，除非调用方显式传 --expect-edges。
    # 否则 live 图生长一次就要改一个与该 fixture 无关的常量，CI 必假红（见审查 P0）。
    expect_edges = args.expect_edges if args.expect_edges is not None else (EXPECT_EDGES if live else None)
    summary = run_check(root, path, expect_edges)
    return 0 if summary["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
