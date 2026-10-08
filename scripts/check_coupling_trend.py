#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""耦合趋势 / 棘轮判据（c-arch-18 / R5）——「架构级问题不再复现」的机器落点。

命题：巡检每轮报「架构级腐化：coupling_high」，但此前**没有任何机器口径**能回答「比上一代好了还是差了」：
`decay_flags` 含 coupling_high 的格数、枢纽出入度只散落在叙事里，既不可复算也不可比对。

本判据把「不再复现」落成**只降不升的上限 + 显式已接受裁定**两条互证的线：
  METRIC-1  coupling_high 载体格数 <= `gates.coupling_ratchet.coupling_high_cells_max`
  METRIC-2  登记枢纽的出入度逐分量 <= `gates.coupling_ratchet.degrees_max`
  METRIC-3  载体格集 == `gates.accepted_coupling.cells` 的 id 集（增减即红，接受不是静默豁免）
  METRIC-4  顶层 `health.decay_flags` 与 METRIC-1 派生一致（载体非空 ⇒ 顶层含该 flag）
  ACCEPT    命中 1–4 且裁定记录齐备（role/reason/review_by 非空）⇒ 输出 accepted:true + 理由

诚实声明：本判据**不设「必须下降」的目标**——棘轮只做「上限不涨」。组合根/装配枢纽知道全部后端域
是**正当职责**，把耦合归零不是本判据的目的；判据的目的是「新增耦合必须显式重登记并给出理由」。
拆格降耦合须另立项（会改结构面，触发地图重采与冻结表同步）。

用法：
  python3 scripts/check_coupling_trend.py --check                 # 对受版本控制 fixture（CI 入口）
  python3 scripts/check_coupling_trend.py --check --map <map>      # 对任意地图（本地可传 live）
  python3 scripts/check_coupling_trend.py --selfcheck              # c1–c5 负例必红自证

本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""
import argparse
import copy
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import map_policy  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURE = os.path.join(REPO, "scripts", "tests", "fixtures", "map_post_split.json")
LIVE = os.path.join(REPO, ".easyvibe", "map", "map.json")

FLAG = "coupling_high"


def _degrees(m):
    out, inc = {}, {}
    for e in m.get("edges", []):
        out[e.get("from")] = out.get(e.get("from"), 0) + 1
        inc[e.get("to")] = inc.get(e.get("to"), 0) + 1
    return out, inc


def evaluate(policy, m):
    """返回 (problems, detail)：METRIC-1..4 + ACCEPT。"""
    problems = []
    try:
        rat = map_policy.coupling_ratchet(policy)
        acc = map_policy.accepted_coupling(policy)
    except map_policy.PolicyMissing as e:
        return ["P policy 缺字段（fail-closed）: %s" % e.dotted_key], {}

    cells = sorted(x.get("id") for x in m.get("modules", [])
                   if FLAG in ((x.get("health") or {}).get("decay_flags") or []))
    out, inc = _degrees(m)
    cmax = rat.get("coupling_high_cells_max")
    dmax = rat.get("degrees_max") or {}
    accepted_ids = sorted(c.get("id") for c in acc.get("cells", []))

    # METRIC-1
    if not isinstance(cmax, int):
        problems.append("P gates.coupling_ratchet.coupling_high_cells_max 缺失/非整数（fail-closed）")
    elif len(cells) > cmax:
        problems.append("METRIC-1 coupling_high 载体 %d > 上限 %d（只降不升；新增载体须显式重登记）: %s"
                        % (len(cells), cmax, cells))
    # METRIC-2
    for mid, caps in sorted(dmax.items()):
        if not isinstance(caps, dict):
            problems.append("P gates.coupling_ratchet.degrees_max.%s 非对象（fail-closed）" % mid)
            continue
        got = {"out": out.get(mid, 0), "in": inc.get(mid, 0)}
        for comp in ("out", "in"):
            lim = caps.get(comp)
            if not isinstance(lim, int):
                problems.append("P gates.coupling_ratchet.degrees_max.%s.%s 缺失/非整数（fail-closed）" % (mid, comp))
            elif got[comp] > lim:
                problems.append("METRIC-2 %s 出/入度 %s=%d > 上限 %d（只降不升）: %s"
                                % (mid, comp, got[comp], lim, got))
    # METRIC-3
    if set(cells) != set(accepted_ids):
        problems.append("METRIC-3 载体格集 != accepted_coupling.cells: 图内=%s 裁定=%s"
                        % (cells, accepted_ids))
    # METRIC-4
    top_flags = ((m.get("health") or {}).get("decay_flags") or [])
    top_has = FLAG in top_flags
    if bool(cells) != bool(top_has):
        problems.append("METRIC-4 顶层 decay_flags 与载体格派生不一致: cells=%s top_has_flag=%s"
                        % (cells, top_has))
    # ACCEPT：裁定记录齐备（防空头接受）
    reasons = {}
    for c in acc.get("cells", []):
        cid = c.get("id")
        for k in ("role", "reason", "review_by"):
            if not isinstance(c.get(k), str) or not c.get(k, "").strip():
                problems.append("ACCEPT 裁定 %s 缺 %s（禁空头接受）" % (cid, k))
        reasons[cid] = c.get("reason", "")

    detail = {
        "coupling_high_cells": cells,
        "coupling_high_cells_max": cmax,
        "degrees": {mid: {"out": out.get(mid, 0), "in": inc.get(mid, 0)} for mid in sorted(dmax)},
        "accepted": not problems,
        "reasons": reasons,
        "scores": {"map": (m.get("health") or {}).get("score"),
                   "modules": {x.get("id"): (x.get("health") or {}).get("score")
                               for x in m.get("modules", []) if x.get("id") in set(accepted_ids)}},
    }
    return problems, detail


def run_check(map_path):
    policy = map_policy.load_policy(REPO)
    problems = ["F0 %s" % p for p in map_policy.validate_policy(policy)]
    if not os.path.isfile(map_path):
        problems.append("map 不可读: %s" % map_path)
        m = {}
    else:
        m = json.load(open(map_path, encoding="utf-8"))
    sub, detail = evaluate(policy, m)
    problems += sub
    report = {"ok": not problems, "map": os.path.relpath(map_path, REPO),
              "coupling_high_cells": detail.get("coupling_high_cells"),
              "coupling_high_cells_max": detail.get("coupling_high_cells_max"),
              "degrees": detail.get("degrees"), "accepted": detail.get("accepted"),
              "scores": detail.get("scores"), "problems": problems}
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["ok"] else 1


def _synth_policy():
    return {
        "modules": [{"id": "hub", "name": "hub", "layer": "l", "emit_order": 0, "files": ["a/**"]},
                    {"id": "leaf", "name": "leaf", "layer": "l", "emit_order": 1, "files": ["b/**"]}],
        "edges": {"expect_count": 1, "sha256": "0" * 64, "retired_ids": []},
        "gates": {
            "retired_concerns": [],
            "coupling_ratchet": {"coupling_high_cells_max": 1,
                                 "degrees_max": {"hub": {"out": 1, "in": 0}}},
            "accepted_coupling": {"cells": [{"id": "hub", "role": "组合根", "reason": "正当职责",
                                             "caps": {"out": 1, "in": 0}, "review_by": "下轮"}]},
        },
    }


def _synth_map():
    return {
        "health": {"score": 80, "decay_flags": [FLAG]},
        "modules": [{"id": "hub", "health": {"score": 76, "decay_flags": [FLAG]}},
                    {"id": "leaf", "health": {"score": 90, "decay_flags": []}}],
        "edges": [{"id": "e1", "from": "hub", "to": "leaf", "type": "import"}],
    }


def selfcheck():
    results = []

    def add(name, passed, detail=""):
        results.append((name, bool(passed), detail))

    pol, m = _synth_policy(), _synth_map()
    probs, detail = evaluate(pol, m)
    add("c5 真实形态（载体 1 == 上限、度数不涨、裁定齐备）→ 必绿 + accepted:true",
        not probs and detail.get("accepted") is True, "; ".join(probs[:1]))

    # c1：注入第 2 格 coupling_high → METRIC-1 红
    m1 = copy.deepcopy(m)
    m1["modules"][1]["health"]["decay_flags"] = [FLAG]
    probs1, _ = evaluate(pol, m1)
    add("c1 注入第 2 格 coupling_high → METRIC-1 必红",
        any("METRIC-1" in p for p in probs1), "; ".join(probs1[:1]))

    # c2：hub 出边 +1 → METRIC-2 红
    m2 = copy.deepcopy(m)
    m2["modules"].append({"id": "leaf2", "health": {"score": 90, "decay_flags": []}})
    m2["edges"].append({"id": "e2", "from": "hub", "to": "leaf2", "type": "import"})
    probs2, _ = evaluate(pol, m2)
    add("c2 hub 出度 +1 → METRIC-2 必红",
        any("METRIC-2" in p for p in probs2), "; ".join(probs2[:1]))

    # c3：摘除载体 flag 但裁定未更新 → METRIC-3 红
    m3 = copy.deepcopy(m)
    m3["modules"][0]["health"]["decay_flags"] = []
    m3["health"]["decay_flags"] = []
    probs3, _ = evaluate(pol, m3)
    add("c3 载体减一而裁定未更新 → METRIC-3 必红",
        any("METRIC-3" in p for p in probs3), "; ".join(probs3[:1]))

    # c4：裁定缺 reason → 必红（fail-closed，防空头接受）
    p4 = _synth_policy()
    p4["gates"]["accepted_coupling"]["cells"][0]["reason"] = "  "
    probs4, _ = evaluate(p4, m)
    add("c4 裁定缺 reason → 必红（禁空头接受）",
        any("ACCEPT" in p for p in probs4), "; ".join(probs4[:1]))

    # c4b：policy 缺 coupling_ratchet → fail-closed
    p5 = _synth_policy()
    del p5["gates"]["coupling_ratchet"]
    probs5, _ = evaluate(p5, m)
    add("c4b 缺 gates.coupling_ratchet → 必红（fail-closed）",
        any("fail-closed" in p for p in probs5), "; ".join(probs5[:1]))

    # c4c：顶层 flag 与载体派生不一致 → METRIC-4 红
    m6 = copy.deepcopy(m)
    m6["health"]["decay_flags"] = []
    probs6, _ = evaluate(pol, m6)
    add("c4c 顶层 flag 与载体派生不一致 → METRIC-4 必红",
        any("METRIC-4" in p for p in probs6), "; ".join(probs6[:1]))

    failed = 0
    for name, ok, detail2 in results:
        failed += 0 if ok else 1
        print("%s %s  %s" % ("PASS" if ok else "FAIL", name, detail2))
    print("c1–c5 %s" % ("全 PASS" if failed == 0 else "%d 项 FAIL" % failed))
    return failed == 0


def main():
    ap = argparse.ArgumentParser(description="耦合趋势/棘轮判据（c-arch-18 / R5）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--map", default=FIXTURE,
                    help="默认受版本控制 fixture（CI 入口）；本地可传 live map")
    args = ap.parse_args()
    if args.selfcheck:
        return 0 if selfcheck() else 1
    return run_check(os.path.abspath(args.map))


if __name__ == "__main__":
    sys.exit(main())
