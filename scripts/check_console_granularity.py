#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""console 侧拆格粒度守卫（R7）：防 settings 域回并 console-ui / 防新格再吞装配壳。

背景：`c-arch-6` 的 console 子句（`c-console-ui-3`）——`components/settings/**`（8 文件）
在 archGuard 断言组 4（`SETTINGS_FILES` 双向全等）与 componentGuard 断言组 4
（`SUBDIR_FILES.settings` 8 + LOC ≤600 + 反向 import 禁）早有独立守卫，却与 pages/**
（页面装配）同处 `console-ui` 一个健康分。本轮按同一范式析出 `settings-ui`
（`presentation`，`files=["easyvibe-renderer/src/components/settings/**"]`）。
本脚本把「拆格不再回退」落成一条 CI 可见的可执行判据（秒级、fail-closed）：

  --check [--map PATH]  默认读 .easyvibe/map/map.json；CI 传受版本控制的 fixture。
  --selfcheck           合成输入 N0–N6（不读 live map、不改仓库、不落盘），逐条自证负例必红/正例必绿。

判据：
  C0 退役态防回归：探针 `src/components/settings/SettingsPanel.tsx` 不得被 `console-ui`
     的任何 glob 命中（settings 回并即红）。
  C1 探针唯一命中：探针恰被**恰好一个模块 glob**命中，且该模块为 `settings-ui`；
     `settings-ui` 缺失即红。探针文件不存在于磁盘 → fail-closed（不得静默跳过）。
  C2 防重复覆盖假绿：`console-ui.files` 不得含 `src/components/settings/**`（去重不看重叠）。
  C3 粒度上限：`settings-ui.files` glob 数 ≤ 1（防再吞 shell/overlays/gate）。
     C3b 反吞探针（双向）：shell/overlays/pages 探针必须命中 `console-ui` 且**不被
     `settings-ui` 命中**。
  C4（--selfcheck）反回归负例：把 `settings/**` 回并 `console-ui` → C0/C1 必红。
  N4 真值双向断言：对受版本控制 fixture（拆分后）必绿；对合成「拆分前图」必红
     —— 断言的是「判据本身在工作」，而非「图已拆分」。
  C5 窄依赖白名单（Δ4/Q5）：`settings-ui` 归属文件的实际 import 只许落在
     相对路径 `./` ｜ `@/runtime/**` ｜ `@/api/**` ｜ `@/components/ui/**` ｜ 外部包；
     禁 `@/pages/**`、`@/App`、其他业务域（chat/taskworkflow/canvas/shell/overlays/gate）。
     C5 因果说明：i18n 已落 `@/runtime/**`（上一轮越界环闭环），故 settings-ui 的
     翻译引用本就在白名单内，无须扩前缀——白名单保持「窄」。
  C6 同层环判据（图边侧）：`settings-ui` 的出边目标集不得含 `console-ui`（map / fixture
     两入口同判；该图无 edges 字段时回退读 settings-ui.dependencies）。与 C5 互补：
     C5 扫**磁盘 import 文本**（防越界前缀）、C6 判**图边**（防同层环）；只改盘不改图 → C6 红，
     只改图不改盘 → C5 红。

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
FIXTURE = os.path.join(REPO, "scripts", "tests", "fixtures", "map_post_split.json")

GRID = "settings-ui"
CONSOLE = "console-ui"
SETTINGS_GLOB = "easyvibe-renderer/src/components/settings/**"
PROBE = "easyvibe-renderer/src/components/settings/SettingsPanel.tsx"

# C3 粒度上限与 C5 依赖白名单的规范值只在 scripts/map_edge_policy.json::granularity.console
# （c-arch-12 同源读取，fail-closed）：此处**不留**任何硬编码默认值，避免第二份事实。
# C3b 反吞探针：必须归 console-ui（页面装配 / 装配壳），不得被 settings-ui 吞走
ANTI_PROBES = (
    "easyvibe-renderer/src/components/shell/AppShell.tsx",
    "easyvibe-renderer/src/components/overlays/ViewsPanel.tsx",
    "easyvibe-renderer/src/pages/GitPage.tsx",
)

IMPORT_RE = re.compile(r"""(?:^|\n)\s*import\s+(?:type\s+)?(?:[^'"\n]*?\sfrom\s+)?['"]([^'"]+)['"]""")
REQUIRE_RE = re.compile(r"""require\(\s*['"]([^'"]+)['"]\s*\)""")

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import map_policy  # noqa: E402
import _arch_synthetic as A  # noqa: E402


def load_console_cfg(root):
    """从 policy 读 console 粒度配置（caps / 白名单 / 退役 id）。缺字段 → fail-closed 字符串。"""
    try:
        g = map_policy.granularity(map_policy.load_policy(root), "console")
        return {"cap": int(g["caps"][GRID]),
                "prefixes": tuple(g["allowed_import_prefixes"]),
                "retired": set(g.get("retired_ids", []))}, None
    except (map_policy.PolicyMissing, KeyError, TypeError, ValueError) as e:
        return None, "policy.granularity.console 缺字段（fail-closed）: %s" % e


def policy_probe_problems(root):
    """R8②「探针归属 ⊆ policy」：探针在其 policy 网格中的归属必须与守卫期望一致。

    policy 丢格 / 改 glob → 直接红（不再要求守卫等到运行时地图同错）。
    """
    problems = []
    try:
        mm = map_policy.modules_map(map_policy.load_policy(root))
    except map_policy.PolicyMissing as e:
        return ["C7 policy 缺字段（fail-closed）: %s" % e.dotted_key]
    owners = {}
    for mid, v in mm.items():
        for g in v["files"]:
            owners.setdefault(mid, []).append(g)

    def own(probe):
        return sorted(mid for mid, gls in owners.items() if any(glob_match(probe, g) for g in gls))

    o = own(PROBE)
    if o != [GRID]:
        problems.append("C7 policy 中探针 %s 期望归 [%s]，实际 %s" % (PROBE, GRID, o))
    for p in ANTI_PROBES:
        op = own(p)
        if op != [CONSOLE]:
            problems.append("C7 policy 中反吞探针 %s 期望归 [%s]，实际 %s" % (p, CONSOLE, op))
    return problems


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


def _by_id(map_obj):
    return {m.get("id"): [str(g) for g in (m.get("files") or [])]
            for m in _modules(map_obj) if isinstance(m, dict)}


def _is_post_split(map_obj):
    ids = set(_by_id(map_obj))
    return GRID in ids and SETTINGS_GLOB not in _by_id(map_obj).get(CONSOLE, [])


def load_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


# ------------------------------------------------------------------ C5 import 扫描
def _iter_settings_files(root, globs):
    """枚举 settings-ui glob 命中的实际文件（posix 相对路径）。"""
    out = []
    for base, _dirs, files in os.walk(root):
        for fn in files:
            p = os.path.join(base, fn)
            rel = os.path.relpath(p, root).replace(os.sep, "/")
            if any(glob_match(rel, g) for g in globs):
                out.append(rel)
    return sorted(out)


def c5_scan(root, globs, overrides=None, prefixes=None):
    """返回 (problems, scanned)。对 settings-ui 归属源码做 import 白名单复核。

    overrides：只读注入面 {posix 相对路径: 文本}；命中即以内存文本替代磁盘内容
    （--selfcheck 的负例靠它注入越界 import，不落盘、不污染仓库）。
    prefixes：白名单前缀（必传，来自 policy.granularity.console）；空即 fail-closed 红。
    """
    problems = []
    if not prefixes:
        return ["C5 settings-ui import 白名单为空（policy 缺失，fail-closed）"], []
    if not globs:
        return ["C5 settings-ui 无 glob，无法裁决依赖白名单"], []
    files = _iter_settings_files(root, globs)
    files = [f for f in files if f.endswith((".ts", ".tsx"))]
    if not files:
        return ["C5 settings-ui glob 未命中任何源码文件（fail-closed）"], []
    overrides = overrides or {}
    for rel in files:
        if rel in overrides:
            text = overrides[rel]
        else:
            try:
                with open(os.path.join(root, rel), encoding="utf-8") as fh:
                    text = fh.read()
            except (UnicodeDecodeError, OSError) as e:
                problems.append("C5 读取失败（fail-closed）%s: %s" % (rel, e))
                continue
        specs = IMPORT_RE.findall(text) + REQUIRE_RE.findall(text)
        for s in specs:
            if s.startswith("."):
                continue  # 相对路径允许
            if s.startswith("@/"):
                if not s.startswith(prefixes):
                    problems.append("C5 %s 越界 import: %s" % (rel, s))
                continue
            # 其余视为外部包（react / lucide-react / @scope/pkg）
    return problems, files


# ------------------------------------------------------------------ 判据
def _settings_targets(map_obj):
    """settings-ui 的出边目标集：优先读 edges，无 edges 字段时回退读 dependencies。"""
    edges = map_obj.get("edges") if isinstance(map_obj, dict) else None
    if isinstance(edges, list) and edges:
        return [e.get("to") for e in edges
                if isinstance(e, dict) and e.get("from") == GRID and e.get("to")]
    for m in _modules(map_obj):
        if isinstance(m, dict) and m.get("id") == GRID:
            return [str(t) for t in (m.get("dependencies") or [])]
    return []


def evaluate(map_obj, root, map_label="", overrides=None):
    """施加 C0–C3b + C5 + C6，返回 (problems, checks)。空 problems = 绿。"""
    problems, checks = [], []
    by_id = _by_id(map_obj)
    ids = set(by_id)
    # c-arch-12：粒度 caps / 白名单从 policy 同源读取（缺字段 → fail-closed）
    cfg, cfg_err = load_console_cfg(root)
    if cfg_err:
        problems.append("C0 %s" % cfg_err)
        # fail-closed 哨兵：cap=-1 使 C3 必红、prefixes=() 使任何 @/ import 必红（无硬编码回退）
        cfg = {"cap": -1, "prefixes": ()}

    # ---- 探针存在性（fail-closed）
    probe_abs = os.path.join(root, PROBE)
    anti_missing = [p for p in ANTI_PROBES if not os.path.isfile(os.path.join(root, p))]
    if not os.path.isfile(probe_abs):
        problems.append("C1 探针文件不在磁盘（fail-closed，不静默跳过）: %s" % PROBE)
    if anti_missing:
        problems.append("C3b 反吞探针不在磁盘（fail-closed）: %s" % anti_missing)

    console_globs = by_id.get(CONSOLE, [])

    # ---- C0 退役态防回归：探针不得被 console-ui 命中
    if CONSOLE not in ids:
        problems.append("C0 console-ui 不在册（fail-closed）")
    elif any(glob_match(PROBE, g) for g in console_globs):
        problems.append("C0 探针 %s 仍被 console-ui 命中（settings 回并 → 拆格被回退）" % PROBE)

    # ---- C1 探针唯一命中恰为 settings-ui
    if GRID not in ids:
        problems.append("C1 新格 %s 缺失（settings 域未析出 / 被回并）" % GRID)
    else:
        hits = [mid for mid, globs in by_id.items()
                if any(glob_match(PROBE, g) for g in globs)]
        if len(hits) != 1:
            problems.append("C1 探针 %s 期望恰被 1 个模块 glob 命中，实际 %d: %s"
                            % (PROBE, len(hits), sorted(hits)))
        elif hits[0] != GRID:
            problems.append("C1 探针 %s 期望归 %s，实际归 %s" % (PROBE, GRID, hits[0]))
        if SETTINGS_GLOB not in by_id.get(GRID, []):
            problems.append("C1 %s 的 files 未含 %s（归属未闭合）" % (GRID, SETTINGS_GLOB))

    # ---- C2 防重复覆盖假绿
    if SETTINGS_GLOB in console_globs:
        problems.append("C2 console-ui.files 仍含 %s（重复覆盖 → coverage 假绿）" % SETTINGS_GLOB)

    # ---- C3 粒度上限（上限读 policy.granularity.console.caps）
    if GRID in ids and len(by_id[GRID]) > cfg["cap"]:
        problems.append("C3 %s 的 files glob 数 %d 超上限 %d（防再吞 shell/overlays/gate）"
                        % (GRID, len(by_id[GRID]), cfg["cap"]))

    # ---- C3b 反吞探针（双向）
    if not anti_missing and GRID in ids and CONSOLE in ids:
        settings_globs = by_id[GRID]
        for p in ANTI_PROBES:
            if not any(glob_match(p, g) for g in console_globs):
                problems.append("C3b 反吞探针 %s 未被 console-ui 命中" % p)
            if any(glob_match(p, g) for g in settings_globs):
                problems.append("C3b 反吞探针 %s 被 settings-ui 吞走" % p)

    # ---- C5 窄依赖白名单
    if GRID in ids:
        c5_problems, _files = c5_scan(root, by_id[GRID], overrides, prefixes=cfg["prefixes"])
        problems += c5_problems

    # ---- C6 同层环判据（图边侧）：settings-ui 不得指向 console-ui
    if GRID in ids:
        targets = _settings_targets(map_obj)
        if CONSOLE in targets:
            problems.append("C6 %s 出边指向 %s（同层环，SCC 复发）" % (GRID, CONSOLE))

    for cid in ("C0", "C1", "C2", "C3", "C5", "C6"):
        checks.append((cid, "FAIL" if any(p.startswith(cid + " ") for p in problems) else "PASS"))
    checks.append(("C3b", "FAIL" if any(p.startswith("C3b ") for p in problems) else "PASS"))
    return problems, dict(checks)


# ------------------------------------------------------------------ --check
def cmd_check(map_path):
    try:
        map_obj = load_json(map_path)
    except (OSError, ValueError) as e:
        print(json.dumps({"ok": False, "stage": "load-map", "map": map_path,
                          "problems": ["地图加载失败（fail-closed）: %s" % e]}, ensure_ascii=False, indent=2))
        return 1
    problems, checks = evaluate(map_obj, REPO, map_path)
    # R8②「探针归属 ⊆ policy」：policy 网格必须把探针归到与守卫期望一致的格
    problems = policy_probe_problems(REPO) + problems
    report = {
        "ok": not problems,
        "map": os.path.relpath(map_path, REPO) if os.path.isabs(map_path) else map_path,
        "modules": sorted(i for i in _by_id(map_obj) if i),
        "checks": checks,
        "problems": problems,
    }
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if not problems else 1


# ------------------------------------------------------------------ --selfcheck
def _green_map():
    """合成「拆分后」绿图（手写构件取自 scripts/_arch_synthetic.py，glob 单副本）。"""
    return A.console_green_map()


def cmd_selfcheck():
    cases = []  # (name, map_obj, expect_red, want_check, overrides)

    def add(name, map_obj, expect_red, want_check=None, overrides=None):
        cases.append((name, map_obj, expect_red, want_check, overrides))

    add("N0 合成拆分后图（正例）", _green_map(), False)

    # N1 settings-ui 缺失（回并仅表现为新格消失）→ C1/C3b 必红
    m1 = _green_map()
    m1["modules"] = [x for x in m1["modules"] if x["id"] != GRID]
    add("N1 settings-ui 缺失 → C1 必红", m1, True, "C1")

    # N4-负例 settings/** 回并 console-ui → C0/C1/C2 必红
    m2 = _green_map()
    for x in m2["modules"]:
        if x["id"] == CONSOLE:
            x["files"] = x["files"] + [SETTINGS_GLOB]
        if x["id"] == GRID:
            x["files"] = []  # 新格被吞空
    add("N4a settings/** 回并 console-ui → C0 必红", m2, True, "C0")

    # N2 新格再吞 shell/** → C3 上限必红
    m3 = _green_map()
    for x in m3["modules"]:
        if x["id"] == GRID:
            x["files"] = x["files"] + ["easyvibe-renderer/src/components/shell/**"]
    add("N2 settings-ui 再吞 shell/** → C3/C3b 必红", m3, True, "C3")

    # N5 同层环回归（图边侧）：settings-ui 出边指回 console-ui → C6 必红
    m5 = _green_map()
    m5["edges"] = [{"id": "ex1", "from": GRID, "to": CONSOLE, "type": "import"}]
    add("N5 settings-ui 出边指回 console-ui → C6 必红", m5, True, "C6")

    # N6 越界 import 回归（盘侧，只读注入）：settings 源文件改回越界前缀 → C5 必红
    # 注入文本不落盘：c5_scan 以内存文本替代磁盘内容，仓库/CI 不受污染。
    add("N6 settings 源文件改回越界前缀 → C5 必红",
        _green_map(), True, "C5",
        {PROBE: "import { useLang } from '@/lib/i18n'\nexport const x = useLang\n"})

    failed = 0
    for name, map_obj, expect_red, want_check, overrides in cases:
        problems, checks = evaluate(map_obj, REPO, overrides=overrides)
        red = bool(problems)
        ok = red == expect_red
        if ok and want_check:
            ok = checks.get(want_check) == "FAIL"
        failed += 0 if ok else 1
        detail = ("%s=%s" % (want_check, checks.get(want_check))) if want_check else (
            problems[0] if problems else "无问题")
        print("%s %s  %s" % ("PASS" if ok else "FAIL", name, detail))

    # ---- N4 真值双向断言（断言的是「判据本身工作」，不是「图已拆分」）
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
        problems, _checks = evaluate(d, REPO)
        # 真值来源的另一半：把 settings 回并 console-ui 合成「拆分前图」→ 必红
        pre = load_json(path)
        for x in pre["modules"]:
            if x.get("id") == CONSOLE:
                x["files"] = list(x["files"]) + [SETTINGS_GLOB]
            if x.get("id") == GRID:
                x["files"] = [g for g in x["files"] if g != SETTINGS_GLOB]
        pre_problems, _ = evaluate(pre, REPO)
        red_post = bool(problems)
        red_pre = bool(pre_problems)
        ok = (not red_post) and red_pre
        failed += 0 if ok else 1
        detail = "%s %s 拆分后实际%s / 合成拆分前实际%s" % (
            src, os.path.basename(path), "红" if red_post else "绿", "红" if red_pre else "绿")
        if red_post and problems:
            detail += " ← " + problems[0]
        print("%s N4 真值双向断言（判据本身工作）  %s" % ("PASS" if ok else "FAIL", detail))

    print("N0–N6 %s" % ("全 PASS" if failed == 0 else "%d 项 FAIL" % failed))
    return 0 if failed == 0 else 1


def main():
    ap = argparse.ArgumentParser(description="console 拆格粒度守卫（R7）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--map", default=DEFAULT_MAP)
    args = ap.parse_args()
    if args.selfcheck:
        return cmd_selfcheck()
    return cmd_check(args.map)


if __name__ == "__main__":
    sys.exit(main())
