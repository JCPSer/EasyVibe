#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""架构事实「同代」红绿判据（c-arch-12 / R9，本命题核心）。

命题：模块 glob 表、期望边数、退役边 id、DV/SCC 棘轮、粒度白名单的唯一受版本控制事实源
= `scripts/map_edge_policy.json`（v2）。live map、`emit_order*.json`、`parts/*.files`、
受版本控制 fixture 均为其**派生物**。本文件把「地图与守卫同代」从**人工同步**变成**红绿可判**。

判据（`--check`，集合级而非仅计数——只比计数会漏判形态漂移）：
  F0 policy 自洽：layer/id/emit_order 唯一、files 非空、layer ∈ layers、必需字段齐备。
  F1 policy ↔ fixture：模块 id 集合全等；**逐格 files 集合全等**；逐格 layer 全等；
     layers id+order 全等；边数 == expect_count；**边投影 sha256 == policy.edges.sha256**；
     fixture 的 concern（顶层 + 逐模块）∩ retired_concerns == ∅；边 id ∩ retired_ids == ∅。
  F2（`--live` 或本地存在 live map 时）对 live map 复判 F1（归纳期/本地同代自证）。

  `--selfcheck`：漂移注入负例**必红**、正例**必绿**（fail-closed 自证，不依赖 live map）：
     inj1 policy 删一格 / inj2 改某格一个 glob / inj3 expect_count +1 /
     inj4 sha256 改一位 / inj5 retired_ids 加一个「图内已有 id」。

CI（无 live map）：`python3 scripts/verify_arch_facts.py --check --fixture <fixture>`（只走 F0+F1）。
本地/归纳期：追加 `--live` 走 F2（policy ↔ live ↔ fixture 三方）。

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


def _concern_ids(health):
    return {c.get("id") for c in (health or {}).get("concerns", [])}


def policy_vs_map(policy, m, label):
    """F1：policy ↔ 一份地图（fixture 或 live）的集合级同代比对。返回问题列表。"""
    problems = []
    try:
        pid = map_policy.module_ids(policy)
        pmm = map_policy.modules_map(policy)
        plm = map_policy.layers_map(policy)
        pexp = map_policy.expect_edges(policy)
        psha = map_policy.expect_edges_sha(policy)
        pretired_ids = map_policy.retired_edge_ids(policy)
        pretired_concerns = map_policy.retired_concerns(policy)
    except map_policy.PolicyMissing as e:
        return ["F1[%s] policy 缺字段: %s" % (label, e.dotted_key)]

    mods = m.get("modules", [])
    edges = m.get("edges", [])
    mid = [x.get("id") for x in mods]
    mmm = {x.get("id"): x for x in mods}

    # 模块 id 集合
    if set(mid) != set(pid):
        problems.append("F1[%s] 模块 id 集合不等: policy-only=%s map-only=%s"
                        % (label, sorted(set(pid) - set(mid)), sorted(set(mid) - set(pid))))
    # 逐格 files 集合 + layer
    for i in pid:
        if i not in mmm:
            continue
        got = sorted(set(str(g) for g in (mmm[i].get("files") or [])))
        want = pmm[i]["files"]
        if got != want:
            problems.append("F1[%s] %s.files 集合不等: policy=%s map=%s" % (label, i, want, got))
        if mmm[i].get("layer") != pmm[i]["layer"]:
            problems.append("F1[%s] %s.layer 不等: policy=%s map=%s"
                            % (label, i, pmm[i]["layer"], mmm[i].get("layer")))
    # layers id+order
    mlm = {l.get("id"): l.get("order") for l in (m.get("layers") or [])}
    plm_io = {k: v["order"] for k, v in plm.items()}
    if mlm != plm_io:
        problems.append("F1[%s] layers id/order 不等: policy=%s map=%s" % (label, plm_io, mlm))
    # 边数与指纹
    if len(edges) != pexp:
        problems.append("F1[%s] 边数 %d != expect_count %d" % (label, len(edges), pexp))
    fpsha = map_policy.edge_fingerprint(edges)
    if fpsha != psha:
        problems.append("F1[%s] 边投影 sha256 不等: policy=%s map=%s"
                        % (label, psha[:16], fpsha[:16]))
    # 边 id ∩ retired_ids
    present = {e.get("id") for e in edges}
    hit = sorted(present & pretired_ids)
    if hit:
        problems.append("F1[%s] 退役边 id 仍在图内: %s" % (label, hit))
    # 架构级在册 concern 集（P0-2）：交付地图顶层 health.concerns 的 id 集 == policy.gates.active_concerns。
    # 这使「架构级关注点在交付面消失」（live=[] 而 authoring=3）成为红——不再依赖人工纪律。
    try:
        pactive = set(map_policy.active_concerns(policy))
    except map_policy.PolicyMissing as e:
        return ["F1[%s] policy 缺字段: %s" % (label, e.dotted_key)]
    top_ids = _concern_ids(m.get("health"))
    if top_ids != pactive:
        problems.append("F1[%s] 顶层在册 concern 集不等: policy=%s map=%s"
                        % (label, sorted(pactive), sorted(top_ids)))
    # concern ∩ retired_concerns（顶层 + 逐模块）
    top_hit = _concern_ids(m.get("health")) & pretired_concerns
    if top_hit:
        problems.append("F1[%s] 顶层未闭环 concern: %s" % (label, sorted(top_hit)))
    for x in mods:
        c = _concern_ids(x.get("health")) & pretired_concerns
        if c:
            problems.append("F1[%s] %s 未闭环 concern: %s" % (label, x.get("id"), sorted(c)))
    return problems


def run_check(fixture_path, live_path, want_live):
    policy = map_policy.load_policy(REPO)
    problems = ["F0 %s" % p for p in map_policy.validate_policy(policy)]
    report = {"ok": True, "policy": {
        "version": policy.get("version"),
        "modules": len(policy.get("modules", [])),
        "edges_expect": policy.get("edges", {}).get("expect_count"),
        "retired_ids": policy.get("edges", {}).get("retired_ids"),
    }}
    try:
        fx = json.load(open(fixture_path, encoding="utf-8"))
    except (OSError, ValueError) as e:
        problems.append("F1 fixture 不可读: %s" % e)
        fx = None
    if fx is not None:
        problems += policy_vs_map(policy, fx, "fixture")
    if want_live:
        if os.path.isfile(live_path):
            lv = json.load(open(live_path, encoding="utf-8"))
            problems += policy_vs_map(policy, lv, "live")
        else:
            problems.append("F2 live map 不存在: %s" % live_path)
    report["ok"] = not problems
    report["problems"] = problems
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["ok"] else 1


def _inject(policy, kind):
    p = copy.deepcopy(policy)
    if kind == "inj1":            # 删一格
        p["modules"].pop()
    elif kind == "inj2":          # 改某格一个 glob
        p["modules"][0]["files"][0] = p["modules"][0]["files"][0] + "-drift"
    elif kind == "inj3":          # 边数 +1
        p["edges"]["expect_count"] = int(p["edges"]["expect_count"]) + 1
    elif kind == "inj4":          # sha256 改一位
        sha = p["edges"]["sha256"]
        p["edges"]["sha256"] = ("0" if sha[0] != "0" else "1") + sha[1:]
    elif kind == "inj5":          # retired_ids 加一个「图内已有 id」
        fx = json.load(open(FIXTURE, encoding="utf-8"))
        p["edges"]["retired_ids"] = list(p["edges"]["retired_ids"]) + [fx["edges"][0]["id"]]
    return p


def selfcheck():
    policy = map_policy.load_policy(REPO)
    fx = json.load(open(FIXTURE, encoding="utf-8"))
    results = []
    base = policy_vs_map(policy, fx, "fixture")
    results.append(("F1 正例（policy ↔ 已提交 fixture）→ 必绿", not base, "; ".join(base[:2])))
    for kind, name in [
        ("inj1", "policy 删一格 → 必红"),
        ("inj2", "policy 改某格一个 glob → 必红"),
        ("inj3", "expect_count +1 → 必红"),
        ("inj4", "sha256 改一位 → 必红"),
        ("inj5", "retired_ids 加图内已有 id → 必红"),
    ]:
        probs = policy_vs_map(_inject(policy, kind), fx, "fixture")
        results.append((name, bool(probs), "; ".join(probs[:1])))
    # inj6（P0-2 返修）：交付地图顶层 health.concerns 与 policy.gates.active_concerns 不一致 → 必红
    fx_empty = copy.deepcopy(fx)
    fx_empty.setdefault("health", {})["concerns"] = []
    probs6 = policy_vs_map(policy, fx_empty, "fixture")
    results.append(("inj6 顶层在册 concern 集清空（架构级关注点消失）→ 必红",
                    bool(probs6), "; ".join(probs6[:1])))
    ok = True
    for name, passed, detail in results:
        ok = ok and passed
        print("  %s %s  %s" % ("PASS" if passed else "FAIL", name, detail))
    print("inj1–inj5 %s" % ("全 PASS" if ok else "有 FAIL"))
    return ok


def main():
    ap = argparse.ArgumentParser(description="架构事实同代判据（c-arch-12 / R9）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--fixture", default=FIXTURE)
    ap.add_argument("--live", action="store_true",
                    help="追加 F2：对 live map 复判（本地/归纳期；CI 无 live 不传）")
    args = ap.parse_args()
    if args.selfcheck:
        return 0 if selfcheck() else 1
    return run_check(os.path.abspath(args.fixture), LIVE, args.live)


if __name__ == "__main__":
    sys.exit(main())
