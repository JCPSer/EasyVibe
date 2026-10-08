#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""叙事（prose）同代判据（c-arch-17 / R3+R4）。

命题：prose（`health.review_note`）此前**没有任何机器判据**，且只住在 gitignored 的 `.easyvibe/map/**`
⇒ 上一代 note 写出「c-server-api-3 未闭环」而无人报警。本文件把「叙事漂移」变成命令级红绿：

  I14 prose 摘要闸门（照 `edges.sha256` 既有范式）：
      live map 的 `health.review_note`（顶层 + 逐格）经规范化后的摘要，
      必须逐字等于 `policy.gates.notes_sha256` / `gates.notes_sha256_by_module`。
      投影字段与规范化口径登记在 `policy.gates.notes_projection`（唯一真值）。
  J  退役 concern 不得在 prose 中复现为「未闭环」：
      `policy.gates.retired_concerns` 中的 id 若与其**同分句**出现未闭环/待处理语义，即红。
      引号内的引述（「…」『…』"…"）先剥离——本轮 note 正是以「上轮断言『c-X 未闭环』与代码矛盾」的
      形式**订正**漂移，属正当表述，不得误伤。
  INV-5 叙事单源：`run/**` 不得出现对 review_note 的**字面量赋值**——叙事唯一写入方 = live map。

fail-closed：policy 缺 `gates.notes_projection` / `notes_sha256` / `notes_sha256_by_module` 即红，
绝不静默回退到默认值（否则闸门自身成为新的歧义源）。

边界（诚实声明）：本闸门只能判「文本是否**变化**」，不能判「文本是否**正确**」；
后者的判据是 J（语义）+ 既有判据 H + 人工评审。

本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""
import argparse
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import map_policy  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LIVE = os.path.join(REPO, ".easyvibe", "map", "map.json")

# 未闭环/待处理语义标记（判据 J）：与退役 id 同分句出现即视为「复现为未闭环」
OPENNESS_MARKERS = ["未闭环", "仍未", "仍存在", "待处理", "尚未", "未解决", "未清零",
                    "仍需", "未收敛", "未纳入", "未修", "待闭环", "仍缺"]
_QUOTED = re.compile(r"[「『\"'][^」』\"']*[」』\"']")
_UNIT = re.compile(r"[。；\n]+")
# INV-5：`review_note`（可带 ['...'] 下标）后紧跟字面量赋值
_ASSIGN = re.compile(r"review_note'?\]?\s*=\s*[\"']")


def _units(note):
    for unit in _UNIT.split(note or ""):
        if unit.strip():
            yield unit.strip()


def _strip_quotes(unit):
    return _QUOTED.sub(" ", unit)


def digest_problems(policy, live):
    """I14：prose 摘要闸门。返回问题列表（空 = 同代）。"""
    try:
        want_arch = map_policy.notes_sha256(policy)
        want_mods = map_policy.notes_sha256_by_module(policy)
    except map_policy.PolicyMissing as e:
        return ["F 摘要字段缺失（fail-closed）: %s" % e.dotted_key]
    problems = []
    ids = [m.get("id") for m in live.get("modules", [])]
    if set(want_mods) != set(ids):
        problems.append("F 格集不等: policy-only=%s map-only=%s"
                        % (sorted(set(want_mods) - set(ids)), sorted(set(ids) - set(want_mods))))
    got_arch = map_policy.prose_arch_digest(live)
    if got_arch != want_arch:
        problems.append("F arch note 漂移: policy=%s live=%s" % (want_arch[:16], got_arch[:16]))
    for mid in ids:
        if mid not in want_mods:
            continue
        got = map_policy.prose_module_digest(mid, live)
        if got != want_mods[mid]:
            problems.append("F note 漂移@%s: policy=%s live=%s" % (mid, want_mods[mid][:16], got[:16]))
    return problems


def retired_prose_problems(policy, live):
    """J：退役 concern 不得在 prose 中复现为「未闭环」。"""
    problems = []
    retired = sorted(map_policy.retired_concerns(policy))
    if not retired:                       # 唯一允许的「空即跳过」
        return problems
    notes = [("arch", (live.get("health") or {}).get("review_note", ""))]
    notes += [(m.get("id"), (m.get("health") or {}).get("review_note", ""))
              for m in live.get("modules", [])]
    for owner, note in notes:
        for unit in _units(note):
            bare = _strip_quotes(unit)
            hits = [r for r in retired if r in bare]
            if hits and any(mk in bare for mk in OPENNESS_MARKERS):
                problems.append("J 退役 concern 在 prose 中复现为「未闭环」@%s: %s" % (owner, hits))
    return problems


def single_source_problems(root):
    """INV-5：**构建/迁移路径**（run/** 与受管 CLI 副本）不得硬编码 review_note 字面量。

    范围界定（诚实声明）：`run/**` 是「构建期整体覆盖已纠正 note」的回归向量（Δ4 的 `ARCH_REVIEW_NOTE`
    正住在这里），故纳入断言。`.easyvibe/map/_tools/spec_*.py` 是 **authoring 面**（模块规格生成器，
    逐格 note 的原始宿主、gitignored 且不被地图管线执行）——把它们判红会让闸门在真实仓库上永久为红，
    属「口径过宽」，故**不在**本断言范围内；它们的叙事同样受 I14 摘要闸门（prose 一旦落进 live map 即被冻结比对）约束。
    """
    problems = []
    targets = [os.path.join(root, "run")]
    copy = os.path.join(root, ".easyvibe", "map", "easyvibe_map_cli.py")
    if os.path.isfile(copy):
        targets.append(copy)
    for t in targets:
        if os.path.isfile(t):
            paths = [t]
        elif os.path.isdir(t):
            paths = [os.path.join(dp, fn) for dp, _dn, fns in os.walk(t) for fn in fns if fn.endswith(".py")]
        else:
            continue
        for p in paths:
            try:
                text = open(p, encoding="utf-8", errors="replace").read()
            except OSError:
                continue
            for i, line in enumerate(text.splitlines(), 1):
                if _ASSIGN.search(line):
                    problems.append("INV-5 review_note 字面量赋值: %s:%d" % (os.path.relpath(p, root), i))
    return problems


def run_check(map_path):
    policy = map_policy.load_policy(REPO)
    problems = ["F0 %s" % p for p in map_policy.validate_policy(policy)]
    if not os.path.isfile(map_path):
        problems.append("live map 不存在: %s" % map_path)
        live = None
    else:
        live = json.load(open(map_path, encoding="utf-8"))
    if live is not None:
        problems += digest_problems(policy, live)
        problems += retired_prose_problems(policy, live)
    problems += single_source_problems(REPO)
    report = {"ok": not problems, "map": os.path.relpath(map_path, REPO),
              "modules": len((live or {}).get("modules", [])),
              "arch_digest": map_policy.prose_arch_digest(live) if live else None,
              "problems": problems}
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["ok"] else 1


def _synth():
    live = {"health": {"review_note": "顶层叙事 A"},
            "modules": [{"id": "m1", "health": {"review_note": "格一叙事"}},
                        {"id": "m2", "health": {"review_note": "格二叙事"}}]}
    policy = {"gates": {
        "retired_concerns": ["c-x-1"],
        "notes_sha256": map_policy.prose_arch_digest(live),
        "notes_sha256_by_module": {m["id"]: map_policy.prose_module_digest(m["id"], live)
                                   for m in live["modules"]},
    }}
    return policy, live


def selfcheck():
    import tempfile
    results = []

    def add(name, passed, detail=""):
        results.append((name, bool(passed), detail))

    policy, live = _synth()
    add("n1 正例（摘要同代 + 无退役 id）→ 必绿",
        not digest_problems(policy, live) and not retired_prose_problems(policy, live))

    drift = json.loads(json.dumps(live))
    drift["health"]["review_note"] = "顶层叙事 B"
    probs = digest_problems(policy, drift)
    add("n2 arch note 改一位 → 必红", bool(probs), "; ".join(probs[:1]))

    drift2 = json.loads(json.dumps(live))
    drift2["modules"][1]["health"]["review_note"] = "格二叙事改"
    probs = digest_problems(policy, drift2)
    add("n3 某格 note 改一位 → 必红且定位到格",
        bool(probs) and any("@m2" in p for p in probs), "; ".join(probs[:1]))

    miss = {"gates": dict(policy["gates"])}
    miss["gates"].pop("notes_sha256")
    add("n4 policy 缺 notes_sha256 → 必红（fail-closed）", bool(digest_problems(miss, live)))

    extra = json.loads(json.dumps(policy))
    extra["gates"]["notes_sha256_by_module"]["m3"] = "0" * 64
    probs = digest_problems(extra, live)
    add("n5 by_module 多一格 → 必红（格集不等）", any("格集不等" in p for p in probs),
        "; ".join(probs[:1]))

    messy = json.loads(json.dumps(live))
    messy["health"]["review_note"] = "顶层叙事 A\r\n   \r\n"
    add("n6 规范化等价（CRLF/尾随空格/空行）→ 必绿",
        map_policy.prose_arch_digest(messy) == policy["gates"]["notes_sha256"])

    add("n7 真实 run/** 无 review_note 字面量 → 必绿", not single_source_problems(REPO),
        "; ".join(single_source_problems(REPO)[:1]))
    with tempfile.TemporaryDirectory() as td:
        os.makedirs(os.path.join(td, "run"))
        with open(os.path.join(td, "run", "x.py"), "w", encoding="utf-8") as fh:
            fh.write('def f(ah):\n    out = dict(ah)\n    out[\'review_note\'] = "硬编码旧叙事"\n')
        add("n7b 受控外硬编码 review_note 字面量 → 必红", bool(single_source_problems(td)))

    open_live = json.loads(json.dumps(live))
    open_live["modules"][0]["health"]["review_note"] = "本次巡检 c-x-1 仍未闭环"
    probs = retired_prose_problems(policy, open_live)
    add("n8 退役 concern 复现为「未闭环」→ 必红", bool(probs), "; ".join(probs[:1]))

    close_live = json.loads(json.dumps(live))
    close_live["modules"][0]["health"]["review_note"] = "本轮摘除 c-x-1（已闭环）"
    add("n9 正当表述「本轮摘除 c-x-1」→ 必绿", not retired_prose_problems(policy, close_live))

    other_live = json.loads(json.dumps(live))
    other_live["modules"][0]["health"]["review_note"] = "c-zzz-9 仍未闭环"
    add("n10 非退役 id 的「未闭环」→ 必绿（不越界）",
        not retired_prose_problems(policy, other_live))

    quote_live = json.loads(json.dumps(live))
    quote_live["modules"][0]["health"]["review_note"] = "上轮断言「c-x-1 未闭环」与代码矛盾，本轮订正"
    add("n11 引述式「『c-x-1 未闭环』」的订正句 → 必绿（不误伤）",
        not retired_prose_problems(policy, quote_live))

    ok = True
    for name, passed, detail in results:
        ok = ok and passed
        print("  %s %s  %s" % ("PASS" if passed else "FAIL", name, detail))
    print("n1–n11 %s" % ("全 PASS" if ok else "有 FAIL"))
    return ok


def main():
    ap = argparse.ArgumentParser(description="叙事（prose）同代判据（c-arch-17 / R3+R4）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--map", default=LIVE, help="默认 live map；fixture 无 prose，故不做默认载体")
    args = ap.parse_args()
    if args.selfcheck:
        return 0 if selfcheck() else 1
    return run_check(os.path.abspath(args.map))


if __name__ == "__main__":
    sys.exit(main())
