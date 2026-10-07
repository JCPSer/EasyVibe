#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""R10（c-arch-2 收口）：地图 renderer 拆格后的**粒度守卫**（纯 python3 CI 载体）。

背景：`renderer-core`（89 文件）已拆为四格——
  renderer-runtime = easyvibe-renderer/src/runtime/**
  renderer-api     = easyvibe-renderer/src/api/**
  renderer-shared  = easyvibe-renderer/src/shared/** + src/types/**
  ui-kit           = easyvibe-renderer/src/components/ui/** + src/lib/utils.ts + src/hooks/use-mobile.ts
另 easyvibe-renderer/scripts/** 归 map-toolchain（非渲染器四格）。

本脚本把「拆分不再回退」落成一条 CI 可见的可执行判据（秒级、fail-closed）：

  --check [--map PATH]  默认读 .easyvibe/map/map.json；CI 传受版本控制的 fixture。
  --selfcheck           合成输入 N0–N5（不读 live map、不改仓库），逐条自证负例必红/正例必绿。

判据：
  G0 已退役的 renderer-core 不得回到模块 id 集合（防回归）。
  G1 四格均在册；用**探针文件表**断言每个真实探针恰被**恰好一个新格 glob** 命中
     （渲染器侧 runtime/api/shared/types/components-ui/lib/utils.ts/hooks/use-mobile.ts 各 ≥1），
     且 easyvibe-renderer/scripts/** 探针命中 map-toolchain、不被四格命中。
     探针文件不存在于磁盘 → fail-closed（不得静默跳过）。
  N5（--selfcheck 反回归负例）把 easyvibe-renderer/src/api/** 回并 renderer-runtime → 必红（G1）。
  G2 反聚合哨兵（god_module 判据）：任一模块 files 同时命中
     easyvibe-renderer/src/runtime/host.ts 与 easyvibe-renderer/src/components/ui/button.tsx → 红。
  G2b 反吞探针（c-arch-5）：src/host-adapter/register.ts 是 app-entry 层适配模块的探针，
     不得被四格任何 glob 命中（尤其 renderer-runtime）——否则宿主实现被 renderer 层吞并、逆边复活。
     探针不在磁盘 → fail-closed。
  G3 粒度上限：四格 files glob 数量上限（runtime ≤4 / api ≤2 / shared ≤4 / ui-kit ≤5），超限即红。
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
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_MAP = os.path.join(REPO, ".easyvibe", "map", "map.json")
EMIT_ORDER = os.path.join(REPO, ".easyvibe", "map", "emit_order.json")
FIXTURE = os.path.join(REPO, "scripts", "tests", "fixtures", "map_post_split.json")

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import map_policy  # noqa: E402
import _arch_synthetic as A  # noqa: E402

def load_renderer_cfg(root):
    """从 policy 读 renderer 粒度配置（grids / caps / 退役 id）。缺字段 → fail-closed 字符串。"""
    try:
        g = map_policy.granularity(map_policy.load_policy(root), "renderer")
        return {"grids": tuple(g["grids"]), "caps": dict(g["caps"]),
                "retired": tuple(g.get("retired_ids", []))}, None
    except (map_policy.PolicyMissing, KeyError, TypeError, ValueError) as e:
        return None, "policy.granularity.renderer 缺字段（fail-closed）: %s" % e


# c-arch-12 / 审查 P2：grids/caps/退役 id 的规范值只在 policy.granularity.renderer。
# 此处**不留**硬编码回退值：policy 缺失 ⇒ GRIDS 空 / RETIRED None ⇒ G0 必红（fail-closed）。
_CFG, _CFG_ERR = load_renderer_cfg(REPO)
GRIDS = tuple(_CFG["grids"]) if _CFG else ()
CAPS = dict(_CFG["caps"]) if _CFG else {}
RETIRED = (_CFG["retired"][0] if _CFG and _CFG["retired"] else None)
TOOLCHAIN = "map-toolchain"

HOST_PROBE = "easyvibe-renderer/src/runtime/host.ts"
UI_PROBE = "easyvibe-renderer/src/components/ui/button.tsx"

# (探针真实路径, 期望命中的新格)
PROBES = (
    ("easyvibe-renderer/src/runtime/host.ts", "renderer-runtime"),
    ("easyvibe-renderer/src/api/core.ts", "renderer-api"),
    ("easyvibe-renderer/src/shared/logic/layout.ts", "renderer-shared"),
    ("easyvibe-renderer/src/types/map.ts", "renderer-shared"),
    ("easyvibe-renderer/src/components/ui/button.tsx", "ui-kit"),
    ("easyvibe-renderer/src/lib/utils.ts", "ui-kit"),
    ("easyvibe-renderer/src/hooks/use-mobile.ts", "ui-kit"),
)
# 工具链探针：应命中 map-toolchain，且不被四格命中
TOOLCHAIN_PROBES = ("easyvibe-renderer/scripts/gen-types.mjs",)

# G2b 反吞探针（c-arch-5）：app-entry 层宿主能力适配器，不得被渲染器四格任何 glob 命中
# （尤其 renderer-runtime）——否则宿主实现被 renderer 层吞并、DV 逆边复活。
ANTI_ENGULF_PROBES = ("easyvibe-renderer/src/host-adapter/register.ts",)


# ------------------------------------------------------------------ glob
# c-arch-12 / 审查 P2：glob 语义只有一份实现（scripts/map_policy.py::glob_match）。
# 本模块此前持有一份逐字副本，现改为直接别名，避免「去重事实副本」的任务反而增副本。
glob_match = map_policy.glob_match


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
    if not GRIDS or RETIRED is None:
        return False  # policy 缺失：不认任何图为「拆分后」，由 G0 报红（fail-closed）
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
    if _CFG_ERR:
        problems.append("G0 %s" % _CFG_ERR)

    # ---- G0 退役模块不得回归
    if RETIRED in by_id:
        problems.append("G0 已退役模块 `%s` 仍在册（拆分被回退）" % RETIRED)
        checks["G0"] = "FAIL"
    else:
        checks["G0"] = "PASS"

    # ---- G1 四格覆盖 + 探针唯一命中（先查探针存在性，fail-closed）
    missing_probes = [p for p, _ in PROBES] + list(TOOLCHAIN_PROBES)
    missing_probes = [p for p in missing_probes if not os.path.isfile(os.path.join(root, p))]
    if missing_probes:
        problems.append("G1 探针文件不在磁盘（fail-closed，不静默跳过）: %s" % missing_probes)
    absent_grids = [g for g in GRIDS if g not in by_id]
    if absent_grids:
        problems.append("G1 四格缺失: %s" % absent_grids)

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
                problems.append("G1 工具链探针 %s 不应被四格命中: %s" % (probe, grid_hits))
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

    # ---- G2b 反吞探针（c-arch-5）：宿主适配器不得被四格（尤其 renderer-runtime）吞并
    missing_engulf = [p for p in ANTI_ENGULF_PROBES if not os.path.isfile(os.path.join(root, p))]
    if missing_engulf:
        problems.append("G2b 反吞探针不在磁盘（fail-closed）: %s" % missing_engulf)
        checks["G2b"] = "FAIL"
    else:
        engulfed = [(p, g, gl) for p in ANTI_ENGULF_PROBES for g in GRIDS
                    for gl in by_id.get(g, []) if glob_match(p, gl)]
        if engulfed:
            problems.append("G2b 宿主适配器探针被渲染器格吞并（逆边复活风险）: %s" % engulfed)
            checks["G2b"] = "FAIL"
        else:
            checks["G2b"] = "PASS"

    # ---- G3 粒度上限（四格 glob 数量）
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
def policy_probe_problems(root):
    """R8②「探针归属 ⊆ policy」：policy 网格必须把每个探针归到守卫期望的格。

    四格探针须恰被其期望格命中；工具链探针须归 map-toolchain；反吞探针不得被任何四格命中。
    policy 丢格 / 改 glob → 直接红。
    """
    problems = []
    try:
        mm = map_policy.modules_map(map_policy.load_policy(root))
    except map_policy.PolicyMissing as e:
        return ["G5 policy 缺字段（fail-closed）: %s" % e.dotted_key]
    by_id = {mid: v["files"] for mid, v in mm.items()}

    def hits(probe, grids):
        return sorted(g for g in grids for gl in by_id.get(g, []) if glob_match(probe, gl))

    for probe, expect in PROBES:
        h = hits(probe, GRIDS)
        if h != [expect]:
            problems.append("G5 policy 中探针 %s 期望归 [%s]，实际 %s" % (probe, expect, h))
    for probe in TOOLCHAIN_PROBES:
        if hits(probe, GRIDS):
            problems.append("G5 policy 中工具链探针 %s 被四格命中: %s" % (probe, hits(probe, GRIDS)))
        if TOOLCHAIN not in hits(probe, (TOOLCHAIN,)):
            problems.append("G5 policy 中工具链探针 %s 未被 %s 命中" % (probe, TOOLCHAIN))
    for probe in ANTI_ENGULF_PROBES:
        if hits(probe, GRIDS):
            problems.append("G5 policy 中反吞探针 %s 被四格命中: %s" % (probe, hits(probe, GRIDS)))
    return problems


def cmd_check(map_path):
    try:
        map_obj = load_json(map_path)
    except (OSError, ValueError) as e:
        print(json.dumps({"ok": False, "stage": "load-map", "map": map_path,
                          "problems": ["地图加载失败（fail-closed）: %s" % e]}, ensure_ascii=False, indent=2))
        return 1
    emit_status, emit_obj = load_emit_order(EMIT_ORDER)
    problems, skips, checks = evaluate(map_obj, emit_status, emit_obj, REPO, map_path)
    # R8②「探针归属 ⊆ policy」
    problems = policy_probe_problems(REPO) + problems
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
    """合成「拆分后」四格绿图（手写构件取自 scripts/_arch_synthetic.py，glob 单副本）。"""
    return A.renderer_green_map()


def cmd_selfcheck():
    cases = []  # (name, map_obj, expect_red, want_check)

    def add(name, map_obj, expect_red, want_check=None):
        cases.append((name, map_obj, expect_red, want_check))

    add("N0 合成拆分后四格（正例）", _green_map(), False)

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

    m4 = _green_map()
    for mm in m4["modules"]:
        if mm["id"] == "renderer-runtime":
            mm["files"] = mm["files"] + ["easyvibe-renderer/src/api/**"]
    add("N5 api 回并 renderer-runtime → G1 必红", m4, True, "G1")

    m6 = _green_map()
    for mm in m6["modules"]:
        if mm["id"] == "renderer-runtime":
            mm["files"] = mm["files"] + ["easyvibe-renderer/src/host-adapter/**"]
    add("N6 renderer-runtime 吞并 host-adapter → G2b 必红", m6, True, "G2b")

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

    print("N0–N5 %s" % ("全 PASS" if failed == 0 else "%d 项 FAIL" % failed))
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
