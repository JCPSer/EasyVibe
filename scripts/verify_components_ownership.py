#!/usr/bin/env python3
"""c-arch-3 关闭验收：把「前端组件归属已收口」做成只读、可复跑、可趋势的判据（M1–M6）。

  --check              读 live `.easyvibe/map/map.json`（本地/归纳期真值）
  --map <fixture>      读受版本控制快照（CI 用；不依赖被 gitignore 的 `.easyvibe/`）
  --selfcheck          负例自证（不依赖 live map、不改仓库）

断言：
  M1 顶层 health.concerns 不含 c-arch-3
  M2 modules[console-ui].health.concerns 不含 c-console-ui-1
  M3 modules[map-canvas].health.concerns 不含 c-map-canvas-1
  M4 console-ui.files / map-canvas.files 无单段 components/<Name>.tsx 条目（防地图回退）
  M5 map-canvas.files 覆盖 canvas/**；console-ui.files 覆盖 pages/**
  M6 趋势双口径输出（地图自评分 + 巡检分来源说明，同既有轮次口径）

本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""
import argparse
import json
import os
import re
import sys

ROOT_ENTRY = re.compile(r"^easyvibe-renderer/src/components/[^/]+\.tsx$")
CLOSED_ARCH = "c-arch-3"
CLOSED_MODULE = {"console-ui": "c-console-ui-1", "map-canvas": "c-map-canvas-1"}


def check(m):
    """纯函数判决。返回 (problems, report)。"""
    problems = []
    mods = {x["id"]: x for x in m.get("modules", [])}
    arch = m.get("health") or {}
    arch_concerns = [c.get("id") for c in arch.get("concerns", [])]
    if CLOSED_ARCH in arch_concerns:
        problems.append("M1 架构 concern %s 仍未闭环" % CLOSED_ARCH)

    for mid, cid in CLOSED_MODULE.items():
        mod = mods.get(mid)
        if mod is None:
            problems.append("M2/M3 图内找不到模块 %s" % mid)
            continue
        ids = [c.get("id") for c in (mod.get("health") or {}).get("concerns", [])]
        if cid in ids:
            problems.append("M%d %s 模块 concern %s 仍未闭环" % (2 if mid == "console-ui" else 3, mid, cid))

    for mid in ("console-ui", "map-canvas"):
        mod = mods.get(mid)
        if not mod:
            continue
        bad = [f for f in mod.get("files", []) if ROOT_ENTRY.match(f)]
        if bad:
            problems.append("M4 %s.files 仍含根级组件条目: %s" % (mid, bad))

    mc = mods.get("map-canvas", {}).get("files", [])
    cu = mods.get("console-ui", {}).get("files", [])
    if not any(f.endswith("components/canvas/**") for f in mc):
        problems.append("M5 map-canvas.files 未覆盖 components/canvas/**")
    if not any(f.endswith("src/pages/**") for f in cu):
        problems.append("M5 console-ui.files 未覆盖 src/pages/**")

    report = {
        "arch_concerns": arch_concerns,
        "map_self_score": arch.get("score"),
        "patrol_score": "见 HealthPage（巡检分存于库内 patrol_runs，按测量时间新旧裁决，不写回地图）",
        "console_ui_files": len(cu), "map_canvas_files": len(mc),
        "modules": len(m.get("modules", [])), "edges": len(m.get("edges", [])),
    }
    return problems, report


def synthetic_case(arch_concerns=(), module_concerns=None, root_entry=False):
    """selfcheck 用合成输入：不读仓库、不依赖 live map。"""
    module_concerns = module_concerns or {}
    files = {
        "console-ui": ["easyvibe-renderer/src/pages/**"],
        "map-canvas": ["easyvibe-renderer/src/components/canvas/**"],
    }
    if root_entry:
        files["console-ui"].append("easyvibe-renderer/src/components/GitPage.tsx")
    mods = []
    for mid in ("console-ui", "map-canvas"):
        mods.append({"id": mid, "health": {"concerns": [{"id": c} for c in module_concerns.get(mid, [])],
                                           "decay_flags": []}, "files": files[mid]})
    return {"modules": mods, "edges": [], "health": {"score": 88, "concerns": [{"id": c} for c in arch_concerns]}}


def selfcheck():
    results = []
    p1, _ = check(synthetic_case(arch_concerns=[CLOSED_ARCH]))
    results.append(("N1 顶层含 c-arch-3 → 必红", any(x.startswith("M1") for x in p1), "; ".join(p1[:1])))
    p2, _ = check(synthetic_case(module_concerns={"console-ui": ["c-console-ui-1"]}))
    results.append(("N2 console-ui 含 c-console-ui-1 → 必红", any(x.startswith("M2") for x in p2), "; ".join(p2[:1])))
    p3, _ = check(synthetic_case(module_concerns={"map-canvas": ["c-map-canvas-1"]}))
    results.append(("N3 map-canvas 含 c-map-canvas-1 → 必红", any(x.startswith("M3") for x in p3), "; ".join(p3[:1])))
    p4, _ = check(synthetic_case(root_entry=True))
    results.append(("N4 地图含根级组件条目 → 必红", any(x.startswith("M4") for x in p4), "; ".join(p4[:1])))
    p5, _ = check(synthetic_case())
    results.append(("N5 正例 → 必绿", not p5, "; ".join(p5[:1])))
    ok = True
    for name, passed, detail in results:
        ok = ok and passed
        print("  %s %s  %s" % ("PASS" if passed else "FAIL", name, detail))
    return ok


def main():
    ap = argparse.ArgumentParser(description="c-arch-3 组件归属收口验收（M1–M6）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--map", default=None)
    ap.add_argument("--root", default=os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    args = ap.parse_args()
    root = os.path.abspath(args.root)
    if args.selfcheck:
        print("▶ 组件归属收口验收自检 N1–N5")
        return 0 if selfcheck() else 1

    path = args.map or os.path.join(root, ".easyvibe", "map", "map.json")
    if not os.path.isfile(path):
        print(json.dumps({"ok": False, "problems": ["map not found: %s" % path]}, ensure_ascii=False))
        return 1
    with open(path, encoding="utf-8") as fh:
        m = json.load(fh)
    problems, report = check(m)
    print(json.dumps({"ok": not problems, **report, "problems": problems}, ensure_ascii=False, indent=2))
    return 0 if not problems else 1


if __name__ == "__main__":
    sys.exit(main())
