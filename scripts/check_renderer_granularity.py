#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""R10（c-arch-2 收口）：地图 renderer 拆格后的**粒度守卫**（纯 python3 CI 载体）。

背景：`renderer-core`（89 文件）已拆为三格——
  renderer-runtime = easyvibe-renderer/src/runtime/** + src/api/**
  renderer-shared  = easyvibe-renderer/src/shared/** + src/types/**
  ui-kit           = easyvibe-renderer/src/components/ui/** + src/lib/utils.ts + src/hooks/use-mobile.ts
另 easyvibe-renderer/scripts/** 归 map-toolchain（非渲染器三格）。

本脚本把「拆分不再回退」落成一条 CI 可见的可执行判据（秒级、fail-closed）：

  --check [--map PATH]  默认读 .easyvibe/map/map.json；CI 传受版本控制的 fixture。
  --selfcheck           合成输入 N0–N4（不读 live map、不改仓库），逐条自证负例必红/正例必绿。

判据：
  G0 已退役的 renderer-core 不得回到模块 id 集合（防回归）。
  G1 三格均在册；用**探针文件表**断言每个真实探针恰被**恰好一个新格 glob** 命中
     （渲染器侧 runtime/api/shared/types/components-ui/lib/utils.ts/hooks/use-mobile.ts 各 ≥1），
     且 easyvibe-renderer/scripts/** 探针命中 map-toolchain、不被三格命中。
     探针文件不存在于磁盘 → fail-closed（不得静默跳过）。
  G2 反聚合哨兵（god_module 判据）：任一模块 files 同时命中
     easyvibe-renderer/src/runtime/host.ts 与 easyvibe-renderer/src/components/ui/button.tsx → 红。
  G3 粒度上限：三格 files glob 数量上限（runtime ≤4 / shared ≤4 / ui-kit ≤5），超限即红。
  G4 归位顺序：若 .easyvibe/map/emit_order.json 在册且含 ui-kit，
     断言 emit_order.modules 中 ui-kit 下标 < console-ui 下标（防 console-ui 的 lib/**、hooks/**
     抢走 lib/utils.ts / hooks/use-mobile.ts）；文件缺失或不同代（无 ui-kit）记为 SKIP，不因此红。

glob 口径：与 `.easyvibe/map/easyvibe_map_cli.py::glob_match` 同源——`**` 跨分隔符、`*` 不跨、
`?` 单字符非分隔符。为让 CI 无 live 依赖（.easyvibe/ 被 gitignore），此处**逐字复制**该实现。

退出码：--check 绿=0 / 红=1；--selfcheck 全 PASS=0 / 任一 FAIL=1。
本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""

import argparse
import json
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_MAP = os.path.join(REPO, ".easyvibe", "map", "map.json")
EMIT_ORDER = os.path.join(REPO, ".easyvibe", "map", "emit_order.json")
FIXTURE = os.path.join(REPO, "scripts", "tests", "fixtures", "map_post_split.json")

RETIRED = "renderer-core"
GRIDS = ("renderer-runtime", "renderer-shared", "ui-kit")
CAPS = {"renderer-runtime": 4, "renderer-shared": 4, "ui-kit": 5}
TOOLCHAIN = "map-toolchain"

HOST_PROBE = "easyvibe-renderer/src/runtime/host.ts"
UI_PROBE = "easyvibe-renderer/src/components/ui/button.tsx"

# (探针真实路径, 期望命中的新格)
PROBES = (
    ("easyvibe-renderer/src/runtime/host.ts", "renderer-runtime"),
    ("easyvibe-renderer/src/api/core.ts", "renderer-runtime"),
    ("easyvibe-renderer/src/shared/logic/layout.ts", "renderer-shared"),
    ("easyvibe-renderer/src/types/map.ts", "renderer-shared"),
    ("easyvibe-renderer/src/components/ui/button.tsx", "ui-kit"),
    ("easyvibe-renderer/src/lib/utils.ts", "ui-kit"),
    ("easyvibe-renderer/src/hooks/use-mobile.ts", "ui-kit"),
)
# 工具链探针：应命中 map-toolchain，且不被三格命中
TOOLCHAIN_PROBES = ("easyvibe-renderer/scripts/gen-types.mjs",)


# ------------------------------------------------------------------ glob
def glob_match(path, pattern):
    """与 .easyvibe/map/easyvibe_map_cli.py::glob_match 同源语义（逐字复制，CI 无 live 依赖）。

    `**` 跨分隔符；`*` 不跨；`?` 单字符非分隔符。path/pattern 均按 posix 分隔符处理。
    """
    path = str(path).replace("\\", "/")
    pattern = str(pattern).replace("\\", "/")
    rx = ""
    i = 0
    while i < len(pattern):
        c = pattern[i]
        if c == "*":
            if pattern[i:i + 2] == "**":
                rx += ".*"
                i += 2
                if i < len(pattern) and pattern[i] == "/":
                    i += 1
                continue
            else:
                rx += "[^/]*"
        elif c == "?":
            rx += "[^/]"
        else:
            rx += re.escape(c)
        i += 1
    return re.match("^" + rx + "$", path) is not None


# ------------------------------------------------------------------ helpers
def _modules(map_obj):
    if isinstance(map_obj, dict):
        mods = map_obj.get("modules")
    else:
        mods = map_obj
    return mods if isinstance(mods, list) else []


def _split_ids(map_obj):
    return {m.get("id") for m in _modules(map_obj) if isinstance(m, dict)}


def _is_post_split(map_obj):
    ids = _split_ids(map_obj)
    return RETIRED not in ids and set(GRIDS) <= ids


def load_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def load_emit_order(path):
    """返回 (status, data)：absent=文件不在册（SKIP）；bad=解析失败（fail-closed）；ok=可裁决。"""
    if not os.path.exists(path):
        return "absent", None
    try:
        return "ok", load_json(path)
    except (OSError, ValueError) as e:
        return "bad", str(e)


# ------------------------------------------------------------------ 判据
def evaluate(map_obj, emit_status, emit_obj, root, map_label=""):
    """施加 G0–G4，返回 (problems, skips, checks)。空 problems = 绿。"""
    problems, skips = [], []
    modules = [m for m in _modules(map_obj) if isinstance(m, dict)]
    by_id = {m.get("id"): [str(g) for g in (m.get("files") or [])] for m in modules}
    checks = {}

    # ---- G0 退役模块不得回归
    if RETIRED in by_id:
        problems.append("G0 已退役模块 `%s` 仍在册（拆分被回退）" % RETIRED)
        checks["G0"] = "FAIL"
    else:
        checks["G0"] = "PASS"

    # ---- G1 三格覆盖 + 探针唯一命中（先查探针存在性，fail-closed）
    missing_probes = [p for p, _ in PROBES] + list(TOOLCHAIN_PROBES)
    missing_probes = [p for p in missing_probes if not os.path.isfile(os.path.join(root, p))]
    if missing_probes:
        problems.append("G1 探针文件不在磁盘（fail-closed，不静默跳过）: %s" % missing_probes)
    absent_grids = [g for g in GRIDS if g not in by_id]
    if absent_grids:
        problems.append("G1 三格缺失: %s" % absent_grids)

    if not missing_probes and not absent_grids:
        for probe, expect in PROBES:
            hits = [(g, gl) for g in GRIDS for gl in by_id.get(g, []) if glob_match(probe, gl)]
            if len(hits) != 1:
                problems.append(
                    "G1 探针 %s 期望恰被 1 个新格 glob 命中，实际命中 %d 个: %s"
                    % (probe, len(hits), [(g, gl) for g, gl in hits])
                )
            elif hits[0][0] != expect:
                problems.append("G1 探针 %s 期望归 %s，实际归 %s（glob=%s）" % (probe, expect, hits[0][0], hits[0][1]))
        for probe in TOOLCHAIN_PROBES:
            grid_hits = [(g, gl) for g in GRIDS for gl in by_id.get(g, []) if glob_match(probe, gl)]
            tc_hit = any(glob_match(probe, gl) for gl in by_id.get(TOOLCHAIN, []))
            if grid_hits:
                problems.append("G1 工具链探针 %s 不应被三格命中: %s" % (probe, grid_hits))
            if not tc_hit:
                problems.append("G1 工具链探针 %s 未被 %s 命中" % (probe, TOOLCHAIN))
    checks["G1"] = "FAIL" if any(p.startswith("G1") for p in problems) else "PASS"

    # ---- G2 反聚合哨兵（runtime 与 components/ui 不得同格）
    g2 = []
    for m in modules:
        globs = [str(g) for g in (m.get("files") or [])]
        if any(glob_match(HOST_PROBE, g) for g in globs) and any(glob_match(UI_PROBE, g) for g in globs):
            g2.append(m.get("id"))
    if g2:
        problems.append("G2 反聚合哨兵：模块 %s 同时容纳 runtime 与 components/ui（god_module 复发）" % g2)
        checks["G2"] = "FAIL"
    else:
        checks["G2"] = "PASS"

    # ---- G3 粒度上限（三格 glob 数量）
    g3_fail = False
    for gid, cap in sorted(CAPS.items()):
        if gid not in by_id:
            continue  # 缺席由 G1 报
        n = len(by_id[gid])
        if n > cap:
            problems.append("G3 模块 %s 的 files glob 数 %d 超上限 %d" % (gid, n, cap))
            g3_fail = True
    checks["G3"] = "FAIL" if g3_fail else "PASS"

    # ---- G4 归位顺序
    if emit_status == "absent":
        skips.append("G4 emit_order 不在册 → SKIP")
        checks["G4"] = "SKIP"
    elif emit_status == "bad":
        problems.append("G4 emit_order 解析失败 → fail-closed: %s" % emit_obj)
        checks["G4"] = "FAIL"
    else:
        mods = emit_obj.get("modules") if isinstance(emit_obj, dict) else None
        if not isinstance(mods, list):
            problems.append("G4 emit_order.modules 非列表 → fail-closed")
            checks["G4"] = "FAIL"
        elif "ui-kit" not in mods:
            skips.append("G4 emit_order 与当前 map 不同代（无 ui-kit）→ SKIP")
            checks["G4"] = "SKIP"
        elif "console-ui" not in mods:
            problems.append("G4 emit_order 含 ui-kit 但缺 console-ui，无法裁决顺序 → fail-closed")
            checks["G4"] = "FAIL"
        elif mods.index("ui-kit") >= mods.index("console-ui"):
            problems.append(
                "G4 ui-kit 下标 %d 未先于 console-ui 下标 %d（lib/**、hooks/** 可能被抢走）"
                % (mods.index("ui-kit"), mods.index("console-ui"))
            )
            checks["G4"] = "FAIL"
        else:
            checks["G4"] = "PASS"

    return problems, skips, checks


# ------------------------------------------------------------------ --check
def cmd_check(map_path):
    try:
        map_obj = load_json(map_path)
    except (OSError, ValueError) as e:
        print(json.dumps({"ok": False, "stage": "load-map", "map": map_path,
                          "problems": ["地图加载失败（fail-closed）: %s" % e]}, ensure_ascii=False, indent=2))
        return 1
    emit_status, emit_obj = load_emit_order(EMIT_ORDER)
    problems, skips, checks = evaluate(map_obj, emit_status, emit_obj, REPO, map_path)
    ids = sorted(i for i in _split_ids(map_obj) if i)
    report = {
        "ok": not problems,
        "map": os.path.relpath(map_path, REPO) if os.path.isabs(map_path) else map_path,
        "modules": ids,
        "checks": checks,
        "problems": problems,
        "skips": skips,
    }
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if not problems else 1


# ------------------------------------------------------------------ --selfcheck
def _green_map():
    """合成「拆分后」绿图：三格 + map-toolchain + console-ui（含 lib/**、hooks/**）。"""
    return {"modules": [
        {"id": "map-toolchain", "files": ["run/**", "scripts/**", "easyvibe-renderer/scripts/**"]},
        {"id": "renderer-runtime", "files": ["easyvibe-renderer/src/runtime/**", "easyvibe-renderer/src/api/**"]},
        {"id": "renderer-shared", "files": ["easyvibe-renderer/src/shared/**", "easyvibe-renderer/src/types/**"]},
        {"id": "ui-kit", "files": ["easyvibe-renderer/src/components/ui/**",
                                   "easyvibe-renderer/src/lib/utils.ts",
                                   "easyvibe-renderer/src/hooks/use-mobile.ts"]},
        {"id": "console-ui", "files": ["easyvibe-renderer/src/App.tsx", "easyvibe-renderer/src/pages/**",
                                       "easyvibe-renderer/src/lib/**", "easyvibe-renderer/src/hooks/**"]},
    ]}


def cmd_selfcheck():
    cases = []  # (name, map_obj, expect_red, want_check)

    def add(name, map_obj, expect_red, want_check=None):
        cases.append((name, map_obj, expect_red, want_check))

    add("N0 合成拆分后三格（正例）", _green_map(), False)

    m1 = _green_map()
    m1["modules"].append({"id": "renderer-blob", "files": [
        "easyvibe-renderer/src/runtime/**", "easyvibe-renderer/src/components/ui/**"]})
    add("N1 runtime 与 components/ui 同格 → G2 必红", m1, True, "G2")

    m2 = _green_map()
    m2["modules"].append({"id": RETIRED, "files": ["easyvibe-renderer/src/runtime/**"]})
    add("N2 已退役 renderer-core 回册 → G0 必红", m2, True, "G0")

    m3 = _green_map()
    for mm in m3["modules"]:
        if mm["id"] == "ui-kit":
            mm["files"] = mm["files"] + [
                "easyvibe-renderer/src/lib/healthReport.ts",
                "easyvibe-renderer/src/lib/onboarding.ts",
                "easyvibe-renderer/src/lib/updater.ts",
            ]
    add("N3 ui-kit glob 超上限 → G3 必红", m3, True, "G3")

    failed = 0
    for name, map_obj, expect_red, want_check in cases:
        problems, _skips, checks = evaluate(map_obj, "absent", None, REPO)
        red = bool(problems)
        ok = red == expect_red
        if ok and want_check:
            ok = checks.get(want_check) == "FAIL"
        failed += 0 if ok else 1
        detail = ("%s=%s" % (want_check, checks.get(want_check))) if want_check else (
            problems[0] if problems else "无问题")
        print("%s %s  %s" % ("PASS" if ok else "FAIL", name, detail))

    # ---- N4：真值双向断言（断言的是「判据本身工作」，不是「图已拆分」）
    source = None
    if os.path.exists(FIXTURE):
        try:
            d = load_json(FIXTURE)
            if _is_post_split(d):
                source = (FIXTURE, d, "fixture")
        except (OSError, ValueError):
            pass
    if source is None and os.path.exists(DEFAULT_MAP):
        try:
            source = (DEFAULT_MAP, load_json(DEFAULT_MAP), "live")
        except (OSError, ValueError):
            source = None
    if source is None:
        print("FAIL N4 真值来源缺失（fixture 未拆分且 live 不可读）→ fail-closed")
        failed += 1
    else:
        path, d, src = source
        post = _is_post_split(d)
        estatus, eobj = load_emit_order(EMIT_ORDER)
        problems, _skips, _checks = evaluate(d, estatus, eobj, REPO)
        red = bool(problems)
        ok = red != post  # 拆分后图必须绿；拆分前图必须红
        failed += 0 if ok else 1
        shape = "拆分后(期望绿)" if post else "拆分前(期望红)"
        detail = "%s=%s %s 实际%s" % (src, os.path.basename(path), shape, "红" if red else "绿")
        if red and problems:
            detail += " ← " + problems[0]
        print("%s N4 真值双向断言（判据本身工作）  %s" % ("PASS" if ok else "FAIL", detail))

    print("N0–N4 %s" % ("全 PASS" if failed == 0 else "%d 项 FAIL" % failed))
    return 0 if failed == 0 else 1


def main():
    ap = argparse.ArgumentParser(description="renderer 拆格粒度守卫（R10）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--map", default=DEFAULT_MAP)
    args = ap.parse_args()
    if args.selfcheck:
        return cmd_selfcheck()
    return cmd_check(args.map)


if __name__ == "__main__":
    sys.exit(main())
