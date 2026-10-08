#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""churn 口径复算与档位同代判据（c-arch-17 / R5）。

命题：churn 档位此前**没有口径登记**——「只算产品文件」这一限定（不限定则 desktop-shell 越 high 线、
server-api / console-ui 读数全变）未写进任何守卫或 policy，同一格内还并存「百分比档」与「阈值档」两套刻度。

本文件把口径固化并做成**命令级复算**：
  口径真值 = `policy.gates.churn_basis`（fail-closed：缺失即红，绝不静默默认）；
  复算 = 在 `cli.product_files()` 的产品文件上按 `git log --since=<window_days>d --name-only`
         统计 (commit × file) 触点，按模块 `files` glob 聚合，套 `bands`（≥high → high；≥medium → medium；否则 low），
         再与 live map 各模块 `health.churn` 逐格比对。
  取不到 git 时回落到 `.easyvibe/map/repo_profile.json::churn`（同一口径的快照），并在输出标注 source。

诚实声明：churn 是**随时间漂移的量**，本判据比对的是**档位**（登记的离散字段）而非精确触点；
精确触点只作输出参考（note 里的计数是描述性的，不进受控面）。

本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""
import argparse
import collections
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import map_policy  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LIVE = os.path.join(REPO, ".easyvibe", "map", "map.json")
PROFILE = os.path.join(REPO, ".easyvibe", "map", "repo_profile.json")


def band_of(touches, bands):
    if touches >= bands["high"]:
        return "high"
    if touches >= bands["medium"]:
        return "medium"
    return "low"


def basis_problems(basis):
    """口径自洽（s7）：bands 单调、window_days 为正、必备键齐。"""
    problems = []
    if not isinstance(basis, dict):
        return ["churn_basis 非 dict"]
    for k in ("files", "metric", "window_days", "bands", "source"):
        if k not in basis:
            problems.append("churn_basis 缺 %s" % k)
    bands = basis.get("bands")
    if not isinstance(bands, dict) or not isinstance(bands.get("high"), int) \
            or not isinstance(bands.get("medium"), int):
        problems.append("churn_basis.bands 须为 {high:int, medium:int}")
    elif bands["high"] <= bands["medium"]:
        problems.append("churn_basis.bands 非单调（high <= medium）")
    if not isinstance(basis.get("window_days"), int) or basis.get("window_days", 0) <= 0:
        problems.append("churn_basis.window_days 须为正整数")
    return problems


def _product_files(root):
    sys.path.insert(0, os.path.join(root, "run"))
    import easyvibe_map_cli as cli
    return set(cli.product_files())


def _git_touches(root, window_days):
    try:
        r = subprocess.run(["git", "log", "--since=%d.days" % window_days, "--name-only",
                            "--pretty=format:"], cwd=root, capture_output=True, text=True)
    except OSError:
        return None
    if r.returncode != 0:
        return None
    return collections.Counter(l.strip() for l in r.stdout.splitlines() if l.strip())


def _profile_touches(root):
    if not os.path.isfile(PROFILE):
        return None
    ch = (json.load(open(PROFILE, encoding="utf-8")) or {}).get("churn")
    return collections.Counter(ch) if isinstance(ch, dict) else None


def gather_touches(root, window_days):
    """返回 (Counter(file→touches), source)；限定在产品文件上（口径的唯一可行解释）。"""
    counter = _git_touches(root, window_days)
    source = "git"
    if counter is None:
        counter = _profile_touches(root)
        source = "profile"
    if counter is None:
        return None, "none"
    files = _product_files(root)
    return collections.Counter({f: n for f, n in counter.items() if f in files}), source


def module_touches(policy, counter):
    out = {}
    for mid, info in map_policy.modules_map(policy).items():
        out[mid] = sum(n for f, n in counter.items()
                       if any(map_policy.glob_match(f, g) for g in info["files"]))
    return out


def check(policy, live, root, touches=None):
    """返回 (problems, per_module)。touches 供 --selfcheck 注入确定性输入。"""
    problems = []
    try:
        basis = map_policy.churn_basis(policy)
    except map_policy.PolicyMissing as e:
        return ["P policy 缺字段（fail-closed）: %s" % e.dotted_key], {}
    problems += basis_problems(basis)
    if problems:
        return problems, {}
    source = "injected"
    if touches is None:
        touches, source = gather_touches(root, int(basis["window_days"]))
        if touches is None:
            return ["P 无法取得 churn 触点（git 与 repo_profile 均不可用）"], {}
        touches = module_touches(policy, touches)
    registered = {m.get("id"): (m.get("health") or {}).get("churn") for m in live.get("modules", [])}
    per_module = {}
    for mid, t in sorted(touches.items(), key=lambda kv: -kv[1]):
        b = band_of(t, basis["bands"])
        reg = registered.get(mid)
        per_module[mid] = {"touches": t, "band": b, "registered": reg, "ok": b == reg}
        if b != reg:
            problems.append("churn 档位漂移 @%s: 复算 %d 触点 → %s，登记 %s" % (mid, t, b, reg))
    return problems, per_module


def projection_problems(policy, proj, fixture, touches=None):
    """R1：受控叙事投影快照的 churn 档位 ↔ 复算（有 git）或 fixture 真值（无 git，Q1-丁退路）。

    fail-closed：快照缺失 / 档位键集不等 / 档位非法 / 口径缺失 ⇒ 红。
    """
    problems = []
    try:
        basis = map_policy.churn_basis(policy)
    except map_policy.PolicyMissing as e:
        return ["P policy 缺字段（fail-closed）: %s" % e.dotted_key]
    problems += basis_problems(basis)
    if not isinstance(proj, dict) or not isinstance(proj.get("churn_bands"), dict) or not proj["churn_bands"]:
        return problems + ["P 投影快照缺 churn_bands（fail-closed）"]
    bands = proj["churn_bands"]
    mids = map_policy.module_ids(policy)
    if set(bands) != set(mids):
        problems.append("P 投影 churn_bands 键集 != policy.modules: policy-only=%s snapshot-only=%s"
                        % (sorted(set(mids) - set(bands)), sorted(set(bands) - set(mids))))
    bad = sorted(k for k, v in bands.items() if v not in ("high", "medium", "low"))
    if bad:
        problems.append("P 投影 churn_bands 含非法档位: %s" % bad)
    if problems:
        return problems
    if touches is None:
        touches, source = gather_touches(REPO, int(basis["window_days"]))
        if touches is not None:
            touches = module_touches(policy, touches)
    else:
        source = "injected"
    if touches is None:
        # Q1-丁退路：零 git 依赖——只比「投影档位 ↔ fixture health.churn」（两者同为受控面）
        registered = {m.get("id"): (m.get("health") or {}).get("churn") for m in fixture.get("modules", [])}
        for mid in sorted(bands):
            if bands[mid] != registered.get(mid):
                problems.append("P[fixture-退路] churn 档位漂移 @%s: 投影 %s，fixture %s"
                                % (mid, bands[mid], registered.get(mid)))
        return problems
    recomputed = {mid: band_of(t, basis["bands"]) for mid, t in touches.items()}
    for mid in sorted(bands):
        if recomputed.get(mid) != bands[mid]:
            problems.append("P churn 档位与复算不同代 @%s: 复算 %s，投影 %s"
                            % (mid, recomputed.get(mid), bands[mid]))
    return problems


def run_projection(policy, proj_path, fixture_path):
    problems = ["F0 %s" % p for p in map_policy.validate_policy(policy)]
    if not os.path.isfile(proj_path):
        problems.append("P 投影快照不存在: %s（fail-closed）" % proj_path)
        proj = None
    else:
        try:
            proj = json.load(open(proj_path, encoding="utf-8"))
        except ValueError as e:
            problems.append("P 投影快照不可解析: %s" % e)
            proj = None
    try:
        fixture = json.load(open(fixture_path, encoding="utf-8"))
    except (OSError, ValueError) as e:
        problems.append("P fixture 不可读: %s" % e)
        fixture = {}
    if proj is not None:
        problems += projection_problems(policy, proj, fixture)
    report = {"ok": not problems, "mode": "projection",
              "projection": os.path.relpath(proj_path, REPO),
              "fixture": os.path.relpath(fixture_path, REPO), "problems": problems}
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["ok"] else 1


def run_check(map_path):
    policy = map_policy.load_policy(REPO)
    if not os.path.isfile(map_path):
        print(json.dumps({"ok": False, "problems": ["map 不存在: %s" % map_path]}, ensure_ascii=False))
        return 1
    live = json.load(open(map_path, encoding="utf-8"))
    problems, per_module = check(policy, live, REPO)
    report = {"ok": not problems, "modules": len(per_module),
              "top": [{"id": k, "touches": v["touches"], "band": v["band"]} for k, v in
                      list(per_module.items())[:5]],
              "problems": problems}
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["ok"] else 1


def selfcheck():
    results = []

    def add(name, passed, detail=""):
        results.append((name, bool(passed), detail))

    bands = {"high": 80, "medium": 30}
    add("s1 触点 80 → high", band_of(80, bands) == "high")
    add("s2 触点 79 → medium", band_of(79, bands) == "medium")
    add("s3 触点 30 → medium", band_of(30, bands) == "medium")
    add("s4 触点 29 → low", band_of(29, bands) == "low")

    pol = {"modules": [{"id": "m1", "name": "m1", "layer": "l", "emit_order": 0, "files": ["a/**"]}],
           "gates": {"churn_basis": {"files": "cli.product_files()", "metric": "commit×file touches",
                                     "window_days": 92, "bands": dict(bands), "source": "git log"}}}
    live_bad = {"modules": [{"id": "m1", "health": {"churn": "high"}}]}
    probs, _ = check(pol, live_bad, REPO, touches={"m1": 10})
    add("s5 复算 low 而登记 high → 必红", bool(probs), "; ".join(probs[:1]))
    live_ok = {"modules": [{"id": "m1", "health": {"churn": "low"}}]}
    probs2, _ = check(pol, live_ok, REPO, touches={"m1": 10})
    add("s5b 复算 low 且登记 low → 必绿", not probs2, "; ".join(probs2[:1]))

    no_basis = {"modules": pol["modules"], "gates": {}}
    probs3, _ = check(no_basis, live_ok, REPO, touches={"m1": 10})
    add("s6 缺 gates.churn_basis → 必红（fail-closed）", bool(probs3), "; ".join(probs3[:1]))

    bad_bands = dict(pol)
    bad_bands["gates"] = {"churn_basis": dict(pol["gates"]["churn_basis"], bands={"high": 30, "medium": 80})}
    probs4, _ = check(bad_bands, live_ok, REPO, touches={"m1": 10})
    add("s7 bands 非单调（high <= medium）→ 必红", bool(probs4), "; ".join(probs4[:1]))

    # ---- R1 投影快照 ↔ churn 档位（s8/s9）----
    pol2 = {"modules": [{"id": "m1", "name": "m1", "layer": "l", "emit_order": 0, "files": ["a/**"]}],
            "gates": {"churn_basis": dict(pol["gates"]["churn_basis"])}}
    fx_ok = {"modules": [{"id": "m1", "health": {"churn": "low"}}]}
    proj_ok = {"churn_bands": {"m1": "low"}}
    add("s8b 投影档位 == 复算 → 必绿",
        not projection_problems(pol2, proj_ok, fx_ok, touches={"m1": 10}))
    proj_drift = {"churn_bands": {"m1": "high"}}
    probs5 = projection_problems(pol2, proj_drift, fx_ok, touches={"m1": 10})
    add("s8 投影 churn 档位与复算差一档 → 必红", bool(probs5), "; ".join(probs5[:1]))
    # 无 git（touches=None 且注入为 None）→ 退路比 fixture；fixture 与投影一致 ⇒ 绿
    add("s8c git 不可用时退路比 fixture（一致 → 必绿）",
        not projection_problems(pol2, proj_ok, fx_ok, touches=None))
    probs6 = projection_problems(pol2, {"churn_bands": {}}, fx_ok, touches={"m1": 10})
    add("s9 投影缺 churn_bands → 必红（fail-closed）",
        any("fail-closed" in p for p in probs6), "; ".join(probs6[:1]))
    probs7 = projection_problems(pol2, None, fx_ok, touches={"m1": 10})
    add("s9b 投影快照缺失（None）→ 必红（fail-closed）",
        any("fail-closed" in p for p in probs7), "; ".join(probs7[:1]))

    ok = True
    for name, passed, detail in results:
        ok = ok and passed
        print("  %s %s  %s" % ("PASS" if passed else "FAIL", name, detail))
    print("s1–s9b %s" % ("全 PASS" if ok else "有 FAIL"))
    return ok


def main():
    ap = argparse.ArgumentParser(description="churn 口径复算与档位同代判据（c-arch-17/18 / R5+R1）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--map", default=LIVE)
    ap.add_argument("--projection", default=None,
                    help="只比对受版本控制的叙事投影快照的 churn 档位（CI 入口）")
    ap.add_argument("--fixture", default=os.path.join(REPO, "scripts", "tests", "fixtures",
                                                      "map_post_split.json"))
    args = ap.parse_args()
    if args.selfcheck:
        return 0 if selfcheck() else 1
    if args.projection:
        return run_projection(map_policy.load_policy(REPO), os.path.abspath(args.projection),
                              os.path.abspath(args.fixture))
    return run_check(os.path.abspath(args.map))


if __name__ == "__main__":
    sys.exit(main())
