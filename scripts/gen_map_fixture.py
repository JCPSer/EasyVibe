#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""受版本控制地图产物的生成器（c-arch-12 / R7；c-arch-18 / R1+R4）。

职责单一：从 **live map** 派生两份受版本控制产物（**单一生成点、单趟、共享同一 provenance**）：

  ① `scripts/tests/fixtures/map_post_split.json`     —— **结构投影**（模块/层/边/health 档位，既有）
  ② `scripts/tests/fixtures/map_prose_projection.json` —— **叙事投影**（R1 新增，只存摘要/档位/计数）

叙事投影**不存 prose 正文**（红线：与 `verify_assets --forbid-literals` 的「受管名唯一合法落点」
不变量冲突；且正文会被散文式修改，进受控面只会制造假漂移），只存：
  notes_projection 口径副本 / notes_sha256 / notes_sha256_by_module（逐格，含 review_note + notes）/
  churn_bands（离散档位）/ provenance（生成期注入的 head_short + generated_at）/ stats。
故「叙事面与结构面同代」由**构造**保证，而非人工纪律（矩阵 §R1 风险的根治）。

去脆化（R4 / Q4-B）：结构投影**不再**投影 `meta.generated_at`（时间戳进叙事投影的 provenance），
否则每次 live 重采都让「重生成 == 已提交」这条字节判据无条件变红（ΔS6）。

输出**确定性**（固定键序 + `indent=2` + 末尾换行），故「重生成 == 已提交」是字节判据。

用法：
  python3 scripts/gen_map_fixture.py            # 生成并写盘（生成前校验同代，失败即拒写）
  python3 scripts/gen_map_fixture.py --check    # 只比两份产物「重生成 == 已提交」，不写盘

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
FIXTURES = os.path.join(REPO, "scripts", "tests", "fixtures")
FIXTURE = os.path.join(FIXTURES, "map_post_split.json")
PROJECTION = os.path.join(FIXTURES, "map_prose_projection.json")
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
    """结构投影（不投影 prose，也不投影 generated_at —— 去脆化见 R4/Q4-B）。"""
    meta = live.get("meta") or {}
    stats = meta.get("stats") or {}
    return {
        "version": "1.0",
        "meta": {
            "repo": meta.get("repo"),
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


def project_prose(policy, live):
    """叙事投影：只存摘要 / 档位 / 计数（无正文）。"""
    meta = live.get("meta") or {}
    stats = meta.get("stats") or {}
    try:
        proj = map_policy.notes_projection(policy)
        arch = map_policy.notes_sha256(policy)
        by_mod = map_policy.notes_sha256_by_module(policy)
        keys = map_policy.prose_provenance_keys(policy)
    except map_policy.PolicyMissing:
        proj, arch, by_mod, keys = {}, None, {}, []
    prov = meta.get("provenance") or {}
    churn = {m.get("id"): (m.get("health") or {}).get("churn") for m in live.get("modules", [])}
    # P0-2 返修：架构级在册 concern（id+severity）纳入受控投影面——「架构级关注点在交付面消失」必红。
    arch_concerns = [{"id": c.get("id"), "severity": c.get("severity")}
                     for c in (live.get("health") or {}).get("concerns", [])]
    arch_concerns.sort(key=lambda c: str(c.get("id")))
    return {
        "version": "1.0",
        "notes_projection": proj,
        "notes_sha256": arch,
        "notes_sha256_by_module": {k: by_mod.get(k) for k in sorted(by_mod)},
        "arch_concerns": arch_concerns,
        "churn_bands": {k: churn.get(k) for k in sorted(churn)},
        "provenance": {k: prov.get(k) for k in keys},
        "stats": {"files_total": stats.get("files_total"),
                  "files_covered": stats.get("files_covered"),
                  "modules": len(live.get("modules", [])),
                  "edges": len(live.get("edges", []))},
    }


def render(proj):
    return json.dumps(proj, ensure_ascii=False, indent=2) + "\n"


def _write(path, text):
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(text)
    os.replace(tmp, path)


def _compare(path, text):
    try:
        cur = open(path, encoding="utf-8").read()
    except OSError as e:
        return ["%s 不可读: %s" % (os.path.relpath(path, REPO), e)]
    if cur != text:
        return ["重生成的 %s 与已提交不一致（不同代）" % os.path.basename(path)]
    return []


def refresh_policy_digests(live_path):
    """生成期注入（R3/R6）：把 live map 的 prose 摘要写回 policy（唯一受版本控制的叙事锚点）。

    为什么必须由生成器注入：prose 住在 gitignored 的 `.easyvibe/map/**`，**受版本控制的唯一痕迹**
    就是 policy 里的这两个 hex 字段；手写它们等于给闸门一个可被静默改写的真值源。
    本模式只写 hex，不写正文（避开 `--forbid-literals` 的受管名不变量）。
    """
    live = json.load(open(live_path, encoding="utf-8"))
    policy = map_policy.load_policy(REPO)
    ids = [m.get("id") for m in live.get("modules", [])]
    new_arch = map_policy.prose_arch_digest(live)
    new_mods = {mid: map_policy.prose_module_digest(mid, live) for mid in ids}
    policy["gates"]["notes_sha256"] = new_arch
    policy["gates"]["notes_sha256_by_module"] = {k: new_mods[k]
                                                 for k in sorted(map_policy.module_ids(policy))
                                                 if k in new_mods}
    with open(map_policy.policy_path(REPO), "w", encoding="utf-8") as f:
        json.dump(policy, f, ensure_ascii=False, indent=2)
        f.write("\n")
    print(json.dumps({"ok": True, "mode": "refresh-policy-digests",
                      "arch_digest": new_arch, "modules": len(new_mods)}, ensure_ascii=False))
    return 0


def main():
    ap = argparse.ArgumentParser(description="受版本控制地图产物生成器（c-arch-12/18）")
    ap.add_argument("--check", action="store_true", help="只比「重生成 == 已提交」")
    ap.add_argument("--live", default=LIVE)
    ap.add_argument("--out", default=FIXTURE)
    ap.add_argument("--projection-out", default=PROJECTION)
    ap.add_argument("--refresh-policy-digests", action="store_true",
                    help="生成期注入：把 live map 的 prose 摘要写回 policy（只写 hex）")
    args = ap.parse_args()

    if args.refresh_policy_digests:
        return refresh_policy_digests(os.path.abspath(args.live))

    policy = map_policy.load_policy(REPO)
    if not os.path.isfile(args.live):
        print(json.dumps({"ok": False, "problems": ["live map 不存在: %s" % args.live]},
                         ensure_ascii=False))
        return 1
    live = json.load(open(args.live, encoding="utf-8"))
    proj = project(live)
    prose = project_prose(policy, live)

    problems = ["F0 %s" % p for p in map_policy.validate_policy(policy)]
    problems += verify_arch_facts.policy_vs_map(policy, proj, "generated")
    if problems:
        print(json.dumps({"ok": False, "stage": "same-generation", "problems": problems},
                         ensure_ascii=False, indent=2))
        return 1

    text = render(proj)
    text_prose = render(prose)
    if args.check:
        probs = _compare(args.out, text) + _compare(args.projection_out, text_prose)
        print(json.dumps({"ok": not probs, "mode": "check",
                          "fixture": os.path.relpath(args.out, REPO),
                          "projection": os.path.relpath(args.projection_out, REPO),
                          "bytes": len(text.encode("utf-8")) + len(text_prose.encode("utf-8")),
                          "problems": probs}, ensure_ascii=False, indent=2))
        return 0 if not probs else 1

    _write(args.out, text)
    _write(args.projection_out, text_prose)
    print(json.dumps({"ok": True, "mode": "write",
                      "fixture": os.path.relpath(args.out, REPO),
                      "projection": os.path.relpath(args.projection_out, REPO),
                      "modules": len(proj["modules"]), "edges": len(proj["edges"]),
                      "bytes": len(text.encode("utf-8")) + len(text_prose.encode("utf-8"))},
                     ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
