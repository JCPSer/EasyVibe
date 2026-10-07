#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""受版本控制地图 fixture 的生成器（c-arch-12 / R7）。

职责单一：从 **live map 的结构投影**派生 `scripts/tests/fixtures/map_post_split.json`，
写盘前先做「同代」校验（复用 scripts/verify_arch_facts.py 的集合级判据）——生成即校验。
输出**确定性**（固定键序 + `indent=2` + 末尾换行），故「重生成 == 已提交」是字节判据（V8）。

投影口径（尊重 fixture 是**结构投影**的不变量，prose 不入受控面）：
  顶层：version / meta(repo,generated_at,stats) / layers / modules / edges / health
  module：id, name, layer, files, dependencies, health(score,coupling,complexity,churn,
          decay_flags, concerns[id,severity])
  edge  ：id, from, to, type, strength
  health：score, coupling, complexity, churn, decay_flags, concerns[id,severity]
（review_note / responsibility / key_entries / edge label / concern 正文等 prose 一律不投影。）

用法：
  python3 scripts/gen_map_fixture.py            # 生成并写盘（生成前校验同代，失败即拒写）
  python3 scripts/gen_map_fixture.py --check    # 只比「重生成 == 已提交」，不写盘（V8）

本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""
import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import map_policy  # noqa: E402
import verify_arch_facts  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURE = os.path.join(REPO, "scripts", "tests", "fixtures", "map_post_split.json")
LIVE = os.path.join(REPO, ".easyvibe", "map", "map.json")

_STAT_KEYS = ("files_total", "files_covered", "coverage_ratio",
              "edges_derived", "edges_inferred", "retried_modules")


def _health(h):
    h = h or {}
    return {
        "score": h.get("score"),
        "coupling": h.get("coupling"),
        "complexity": h.get("complexity"),
        "churn": h.get("churn"),
        "decay_flags": list(h.get("decay_flags", [])),
        "concerns": [{"id": c.get("id"), "severity": c.get("severity")}
                     for c in h.get("concerns", [])],
    }


def project(live):
    meta = live.get("meta") or {}
    stats = meta.get("stats") or {}
    return {
        "version": "1.0",
        "meta": {
            "repo": meta.get("repo"),
            "generated_at": meta.get("generated_at"),
            "stats": {k: stats.get(k) for k in _STAT_KEYS},
        },
        "layers": [{"id": l.get("id"), "name": l.get("name"), "order": l.get("order"),
                    "description": l.get("description", "")} for l in live.get("layers", [])],
        "modules": [{
            "id": m.get("id"), "name": m.get("name"), "layer": m.get("layer"),
            "files": list(m.get("files", [])),
            "dependencies": list(m.get("dependencies", [])),
            "health": _health(m.get("health")),
        } for m in live.get("modules", [])],
        "edges": [{"id": e.get("id"), "from": e.get("from"), "to": e.get("to"),
                   "type": e.get("type"), "strength": e.get("strength")}
                  for e in live.get("edges", [])],
        "health": _health(live.get("health")),
    }


def render(proj):
    return json.dumps(proj, ensure_ascii=False, indent=2) + "\n"


def main():
    ap = argparse.ArgumentParser(description="受版本控制地图 fixture 生成器（c-arch-12 / R7）")
    ap.add_argument("--check", action="store_true", help="只比「重生成 == 已提交」")
    ap.add_argument("--live", default=LIVE)
    ap.add_argument("--out", default=FIXTURE)
    args = ap.parse_args()

    policy = map_policy.load_policy(REPO)
    if not os.path.isfile(args.live):
        print(json.dumps({"ok": False, "problems": ["live map 不存在: %s" % args.live]},
                         ensure_ascii=False))
        return 1
    live = json.load(open(args.live, encoding="utf-8"))
    proj = project(live)

    problems = ["F0 %s" % p for p in map_policy.validate_policy(policy)]
    problems += verify_arch_facts.policy_vs_map(policy, proj, "generated")
    if problems:
        print(json.dumps({"ok": False, "stage": "same-generation", "problems": problems},
                         ensure_ascii=False, indent=2))
        return 1

    text = render(proj)
    if args.check:
        try:
            cur = open(args.out, encoding="utf-8").read()
        except OSError as e:
            print(json.dumps({"ok": False, "problems": ["fixture 不可读: %s" % e]}, ensure_ascii=False))
            return 1
        same = cur == text
        print(json.dumps({"ok": same, "mode": "check", "fixture": os.path.relpath(args.out, REPO),
                          "bytes": len(text.encode("utf-8")),
                          "problems": [] if same else ["重生成的 fixture 与已提交不一致（不同代）"]},
                         ensure_ascii=False, indent=2))
        return 0 if same else 1

    tmp = args.out + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(text)
    os.replace(tmp, args.out)
    print(json.dumps({"ok": True, "mode": "write", "fixture": os.path.relpath(args.out, REPO),
                      "modules": len(proj["modules"]), "edges": len(proj["edges"]),
                      "bytes": len(text.encode("utf-8"))}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
