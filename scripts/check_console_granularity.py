#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""console 侧拆格粒度守卫（R7 → c-arch-14 泛化为多格）：防格回并 console-ui / 防新格再吞装配壳。

背景：`c-arch-6` 的 console 子句（`c-console-ui-3`）——`components/settings/**`（8 文件）
在 archGuard 断言组 4 与 componentGuard 断言组 4 早有独立守卫，却与 pages/**（页面装配）
同处 `console-ui` 一个健康分。析出 `settings-ui` 后，本脚本把「拆格不再回退」落成一条
CI 可见的可执行判据（秒级、fail-closed）。

c-arch-14 演进：呈现层单格过重的根因是 `console-ui` 同时装「壳 + 页面 + 门禁」与
`lib/**` 下健康报告 / 引导状态 / 更新器三类与装配无关的能力。本轮把三类能力外提为
独立呈现格（`health-report` / `onboarding-state` / `app-updater`），并把本守卫从
**单格硬编码**泛化为「**N 格 + 逐格探针 + 逐格白名单**」——settings 的既有判据语义
逐字不变（settings 组负例条数不减），新格获得同构的归属/上限/白名单/同层环判据。

  --check [--map PATH]  默认读 .easyvibe/map/map.json；CI 传受版本控制的 fixture。
  --selfcheck           合成输入（不读 live map、不改仓库、不落盘），逐条自证负例必红/正例必绿。

判据（逐格参数化；对 settings-ui 的语义与上一代逐字一致）：
  C0 退役态防回归：各格探针不得被 `console-ui` 的任何 glob 命中（格回并即红）。
  C1 探针唯一命中：探针恰被**恰好一个模块 glob**命中且该模块 == 期望格；格缺失即红；
     探针文件不存在于磁盘 → fail-closed（不得静默跳过）；格 files 未含期望 glob 即红。
  C2 防重复覆盖假绿：`console-ui.files` 不得含任一格的期望 glob；另断言
     `src/lib/utils.ts` 唯一归 `ui-kit`（ΔS5：console-ui 删 `src/lib/**` 后重复覆盖消失）。
  C3 粒度上限（逐格）：每格 files glob 数 ≤ policy.granularity.console.caps[格]
     （语义 = glob 条数上限；三新格 cap=1，防再吞 shell/overlays/gate）。
     C3b 反吞探针（双向）：shell/overlays/pages 探针必须命中 `console-ui` 且**不被任一格**命中。
  C5 窄依赖白名单（逐格）：从 policy.granularity.console.import_prefixes_by_grid[格]
     逐格读取（键缺失 fail-closed）；扫描该格 glob 命中的真实源码。空列表 = 只许外部包。
  C6 同层环判据（图边侧，逐格）：任一格出边目标集不得含 `console-ui`（map / fixture 两入口）。
  C7 探针归属 ⊆ policy：policy 网格中每格探针的归属必须与守卫期望一致（防守卫内硬编码探针
     与 policy 改 glob 后被架空）；policy grids 集合必须与守卫期望的格集严格相等。

glob 口径：与 `scripts/map_policy.py::glob_match` 同源（唯一实现）。

退出码：--check 绿=0 / 红=1；--selfcheck 全 PASS=0 / 任一 FAIL=1。
本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""

import argparse
import copy
import json
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_MAP = os.path.join(REPO, ".easyvibe", "map", "map.json")
FIXTURE = os.path.join(REPO, "scripts", "tests", "fixtures", "map_post_split.json")

CONSOLE = "console-ui"

# 逐格探针（守卫**期望**；规范 caps/前缀在 scripts/map_edge_policy.json::granularity.console，
# c-arch-12 同源读取，fail-closed——此处不留任何硬编码 caps/前缀默认值）。
# ★ 探针的归属由 C7 与 policy 网格对齐：policy 丢格 / 改 glob → 直接红。
GRID_PROBES = {
    "settings-ui":      "easyvibe-renderer/src/components/settings/SettingsPanel.tsx",
    "health-report":    "easyvibe-renderer/src/lib/healthReport.ts",
    "onboarding-state": "easyvibe-renderer/src/lib/onboarding.ts",
    "app-updater":      "easyvibe-renderer/src/lib/updater.ts",
}
# 各格的期望文件 glob（归属闭合 / 回并判据的锚）
GRID_GLOBS = {
    "settings-ui":      "easyvibe-renderer/src/components/settings/**",
    "health-report":    "easyvibe-renderer/src/lib/healthReport.ts",
    "onboarding-state": "easyvibe-renderer/src/lib/onboarding.ts",
    "app-updater":      "easyvibe-renderer/src/lib/updater.ts",
}
NEW_GRIDS = ("health-report", "onboarding-state", "app-updater")
CONSOLE_LIB_ALL = "easyvibe-renderer/src/lib/**"
# ΔS5：重复覆盖清零探针（utils.ts 必须唯一归 ui-kit，不得被 console-ui 再吞）
UTILS_PROBE = "easyvibe-renderer/src/lib/utils.ts"
UTILS_OWNER = "ui-kit"

# C3b 反吞探针：必须归 console-ui（页面装配 / 装配壳），不得被任一格吞走
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
    """从 policy 读 console 多格粒度配置（逐格 caps / 白名单）。

    返回 (cfg, None)：cfg = {grid: {"cap": int, "prefixes": tuple, "retired": set}}
    —— prefixes 为 `()` 表示**合法空白名单**（如 onboarding-state 零 @/ 依赖）；
    键缺失才 fail-closed（返回 (None, err)），两者语义不同（ΔS4）。
    """
    try:
        g = map_policy.granularity(map_policy.load_policy(root), "console")
        grids = list(g["grids"])
        expected = set(GRID_PROBES)
        if set(grids) != expected:
            raise KeyError("grids 集合与守卫期望不一致: %s"
                           % sorted(set(grids) ^ expected))
        out = {}
        for gid in grids:
            out[gid] = {"cap": int(g["caps"][gid]),                       # 缺 → KeyError
                        "prefixes": tuple(g["import_prefixes_by_grid"][gid]),  # 缺 → KeyError
                        "retired": set(g.get("retired_ids", []))}
        return out, None
    except (map_policy.PolicyMissing, KeyError, TypeError, ValueError) as e:
        return None, "policy.granularity.console 缺字段（fail-closed）: %s" % e


def policy_probe_problems(root):
    """C7「探针归属 ⊆ policy」：逐格探针在 policy 网格中的归属必须与守卫期望一致。

    policy 丢格 / 改 glob → 直接红（不再要求守卫等到运行时地图同错）。
    """
    problems = []
    try:
        mm = map_policy.modules_map(map_policy.load_policy(root))
        g = map_policy.granularity(map_policy.load_policy(root), "console")
    except map_policy.PolicyMissing as e:
        return ["C7 policy 缺字段（fail-closed）: %s" % e.dotted_key]
    if set(g.get("grids", [])) != set(GRID_PROBES):
        problems.append("C7 policy grids 与守卫期望格集不一致: %s"
                        % sorted(set(g.get("grids", [])) ^ set(GRID_PROBES)))
    owners = {}
    for mid, v in mm.items():
        for gl in v["files"]:
            owners.setdefault(mid, []).append(gl)

    def own(probe):
        return sorted(mid for mid, gls in owners.items() if any(glob_match(probe, g_) for g_ in gls))

    for gid, probe in GRID_PROBES.items():
        o = own(probe)
        if o != [gid]:
            problems.append("C7 policy 中探针 %s 期望归 [%s]，实际 %s" % (probe, gid, o))
    if own(UTILS_PROBE) != [UTILS_OWNER]:
        problems.append("C7 policy 中 %s 期望归 [%s]，实际 %s"
                        % (UTILS_PROBE, UTILS_OWNER, own(UTILS_PROBE)))
    for p in ANTI_PROBES:
        op = own(p)
        if op != [CONSOLE]:
            problems.append("C7 policy 中反吞探针 %s 期望归 [%s]，实际 %s" % (p, CONSOLE, op))
    return problems


# ------------------------------------------------------------------ glob
# c-arch-12 / 审查 P2：glob 语义只有一份实现（scripts/map_policy.py::glob_match）。
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
    by = _by_id(map_obj)
    if CONSOLE_LIB_ALL in by.get(CONSOLE, []):
        return False
    return all(gid in by for gid in GRID_PROBES)


def load_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


# ------------------------------------------------------------------ C5 import 扫描
def _literal_base(glob):
    """glob 的通配符前最长目录前缀（性能：只走命中面，不再全库 os.walk）。"""
    cut = len(glob)
    for i, ch in enumerate(glob):
        if ch in "*?":
            cut = i
            break
    base = glob[:cut]
    j = base.rfind("/")
    return base[:j] if j >= 0 else ""


def _iter_glob_files(root, globs):
    """枚举 globs 命中的实际文件（posix 相对路径）。

    每条 glob 只从**其字面前缀目录**起步遍历（node_modules/target/.git 等永不入面），
    命中集合与「全库遍历 + glob_match」逐字等价，但把 N 格 × M 例的扫描从全库降到子树。
    """
    out = set()
    for g in globs:
        base = _literal_base(g)
        d = os.path.join(root, base) if base else root
        if not os.path.isdir(d):
            if os.path.isfile(os.path.join(root, g)):
                out.add(g)
            continue
        for dirpath, _dirs, files in os.walk(d):
            for fn in files:
                rel = os.path.relpath(os.path.join(dirpath, fn), root).replace(os.sep, "/")
                if glob_match(rel, g):
                    out.add(rel)
    return sorted(out)


def c5_scan(root, globs, overrides=None, prefixes=None, label="settings-ui"):
    """对某格归属源码做 import 白名单复核。返回 (problems, scanned)。

    overrides：只读注入面 {posix 相对路径: 文本}；命中即以内存文本替代磁盘内容
    （--selfcheck 的负例靠它注入越界 import，不落盘、不污染仓库）。
    prefixes：白名单前缀元组。`None` = 键缺失（fail-closed 红）；`()` = 合法空白名单
    （只许外部包 / 相对路径，任何 `@/…` 越界即红）。
    """
    problems = []
    if prefixes is None:
        return ["C5 %s import 白名单缺失（policy 缺字段，fail-closed）" % label], []
    if not globs:
        return ["C5 %s 无 glob，无法裁决依赖白名单" % label], []
    files = _iter_glob_files(root, globs)
    files = [f for f in files if f.endswith((".ts", ".tsx"))]
    if not files:
        return ["C5 %s glob 未命中任何源码文件（fail-closed）" % label], []
    overrides = overrides or {}
    for rel in files:
        if rel in overrides:
            text = overrides[rel]
        else:
            try:
                with open(os.path.join(root, rel), encoding="utf-8") as fh:
                    text = fh.read()
            except (UnicodeDecodeError, OSError) as e:
                problems.append("C5 %s 读取失败（fail-closed）%s: %s" % (label, rel, e))
                continue
        specs = IMPORT_RE.findall(text) + REQUIRE_RE.findall(text)
        for s in specs:
            if s.startswith("."):
                continue  # 相对路径允许
            if s.startswith("@/"):
                if not s.startswith(prefixes):
                    problems.append("C5 %s %s 越界 import: %s" % (label, rel, s))
                continue
            # 其余视为外部包（react / lucide-react / @scope/pkg）
    return problems, files


# ------------------------------------------------------------------ 判据
def _grid_targets(map_obj, gid):
    """某格的出边目标集：优先读 edges，无 edges 字段时回退读 dependencies。"""
    edges = map_obj.get("edges") if isinstance(map_obj, dict) else None
    if isinstance(edges, list) and edges:
        return [e.get("to") for e in edges
                if isinstance(e, dict) and e.get("from") == gid and e.get("to")]
    for m in _modules(map_obj):
        if isinstance(m, dict) and m.get("id") == gid:
            return [str(t) for t in (m.get("dependencies") or [])]
    return []


def evaluate(map_obj, root, map_label="", overrides=None):
    """施加 C0–C3b + C5–C7（逐格），返回 (problems, checks)。空 problems = 绿。"""
    problems, checks = [], []
    by_id = _by_id(map_obj)
    ids = set(by_id)
    # c-arch-12：逐格 caps / 白名单从 policy 同源读取（缺字段 → fail-closed）
    cfg, cfg_err = load_console_cfg(root)
    if cfg_err:
        problems.append("C0 %s" % cfg_err)
        cfg = {}

    # ---- 探针存在性（fail-closed）
    for gid, probe in GRID_PROBES.items():
        if not os.path.isfile(os.path.join(root, probe)):
            problems.append("C1 %s 探针文件不在磁盘（fail-closed，不静默跳过）: %s" % (gid, probe))
    if not os.path.isfile(os.path.join(root, UTILS_PROBE)):
        problems.append("C2 utils.ts 探针不在磁盘（fail-closed）: %s" % UTILS_PROBE)
    anti_missing = [p for p in ANTI_PROBES if not os.path.isfile(os.path.join(root, p))]
    if anti_missing:
        problems.append("C3b 反吞探针不在磁盘（fail-closed）: %s" % anti_missing)

    console_globs = by_id.get(CONSOLE, [])
    if CONSOLE not in ids:
        problems.append("C0 console-ui 不在册（fail-closed）")

    for gid, probe in GRID_PROBES.items():
        want_glob = GRID_GLOBS[gid]
        # ---- C0 退役态防回归：探针不得被 console-ui 命中
        if CONSOLE in ids and any(glob_match(probe, g) for g in console_globs):
            problems.append("C0 %s 探针 %s 仍被 console-ui 命中（拆格被回退）" % (gid, probe))
        # ---- C2 防重复覆盖假绿（逐格 glob）
        if want_glob in console_globs:
            problems.append("C2 console-ui.files 仍含 %s（重复覆盖 → coverage 假绿）" % want_glob)
        if gid not in ids:
            # ---- C1 格缺失即红
            problems.append("C1 新格 %s 缺失（未析出 / 被回并）" % gid)
            continue
        # ---- C1 探针唯一命中恰为本格
        hits = [mid for mid, globs in by_id.items()
                if any(glob_match(probe, g) for g in globs)]
        if len(hits) != 1:
            problems.append("C1 %s 探针 %s 期望恰被 1 个模块 glob 命中，实际 %d: %s"
                            % (gid, probe, len(hits), sorted(hits)))
        elif hits[0] != gid:
            problems.append("C1 %s 探针 %s 期望归 %s，实际归 %s" % (gid, probe, gid, hits[0]))
        if want_glob not in by_id.get(gid, []):
            problems.append("C1 %s 的 files 未含 %s（归属未闭合）" % (gid, want_glob))
        # ---- C3 粒度上限（上限读 policy.granularity.console.caps[gid]）
        cap = cfg.get(gid, {}).get("cap", -1)
        if len(by_id[gid]) > cap:
            problems.append("C3 %s 的 files glob 数 %d 超上限 %d（防再吞 shell/overlays/gate）"
                            % (gid, len(by_id[gid]), cap))
        # ---- C5 窄依赖白名单（逐格）
        prefixes = cfg.get(gid, {}).get("prefixes", None)
        c5_problems, _files = c5_scan(root, by_id[gid], overrides, prefixes=prefixes, label=gid)
        problems += c5_problems
        # ---- C6 同层环判据（图边侧）：本格不得指向 console-ui
        if CONSOLE in _grid_targets(map_obj, gid):
            problems.append("C6 %s 出边指向 %s（同层环，SCC 复发）" % (gid, CONSOLE))

    # ---- C2 utils.ts 唯一归 ui-kit（ΔS5）
    u_hits = [mid for mid, globs in by_id.items()
              if any(glob_match(UTILS_PROBE, g) for g in globs)]
    if u_hits != [UTILS_OWNER]:
        problems.append("C2 %s 期望唯一归 %s，实际 %s" % (UTILS_PROBE, UTILS_OWNER, sorted(u_hits)))

    # ---- C3b 反吞探针（双向）
    if not anti_missing and CONSOLE in ids:
        for p in ANTI_PROBES:
            if not any(glob_match(p, g) for g in console_globs):
                problems.append("C3b 反吞探针 %s 未被 console-ui 命中" % p)
            for gid in GRID_PROBES:
                if gid in ids and any(glob_match(p, g) for g in by_id[gid]):
                    problems.append("C3b 反吞探针 %s 被 %s 吞走" % (p, gid))

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
    # C7「探针归属 ⊆ policy」：policy 网格必须把每格探针归到与守卫期望一致的格
    problems = policy_probe_problems(REPO) + problems
    report = {
        "ok": not problems,
        "map": os.path.relpath(map_path, REPO) if os.path.isabs(map_path) else map_path,
        "grids": sorted(GRID_PROBES),
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


def _merge_back(map_obj):
    """把每一格回并 console-ui（合成「拆分前图」），并恢复 console-ui 的 src/lib/**。"""
    pre = copy.deepcopy(map_obj)
    for m in pre["modules"]:
        if m.get("id") == CONSOLE:
            m["files"] = list(m["files"]) + [CONSOLE_LIB_ALL] + [GRID_GLOBS[g] for g in GRID_PROBES]
        elif m.get("id") in GRID_PROBES:
            m["files"] = [g for g in m["files"] if g != GRID_GLOBS[m["id"]]]
    return pre


def cmd_selfcheck():
    cases = []  # (name, map_obj, expect_red, want_check, overrides)

    def add(name, map_obj, expect_red, want_check=None, overrides=None):
        cases.append((name, map_obj, expect_red, want_check, overrides))

    add("N0 合成拆分后图（正例）", _green_map(), False)

    # ---- settings 组（既有 5 条，逐条保留、条数不减 = 判据未被削弱的证据）
    m1 = _green_map()
    m1["modules"] = [x for x in m1["modules"] if x["id"] != "settings-ui"]
    add("N1 settings-ui 缺失 → C1 必红", m1, True, "C1")

    m2 = _green_map()
    for x in m2["modules"]:
        if x["id"] == CONSOLE:
            x["files"] = x["files"] + [GRID_GLOBS["settings-ui"]]
        if x["id"] == "settings-ui":
            x["files"] = []
    add("N4a settings/** 回并 console-ui → C0 必红", m2, True, "C0")

    m3 = _green_map()
    for x in m3["modules"]:
        if x["id"] == "settings-ui":
            x["files"] = x["files"] + ["easyvibe-renderer/src/components/shell/**"]
    add("N2 settings-ui 再吞 shell/** → C3 必红", m3, True, "C3")

    m5 = _green_map()
    m5["edges"] = [{"id": "ex1", "from": "settings-ui", "to": CONSOLE, "type": "import"}]
    add("N5 settings-ui 出边指回 console-ui → C6 必红", m5, True, "C6")

    add("N6 settings 源文件改回越界前缀 → C5 必红",
        _green_map(), True, "C5",
        {GRID_PROBES["settings-ui"]: "import { useLang } from '@/lib/i18n'\nexport const x = useLang\n"})

    # ---- 三新格组（每格 4 类：缺失 / 回并 / 再吞 / 越界 import）
    for gid in NEW_GRIDS:
        gm = _green_map()
        gm["modules"] = [x for x in gm["modules"] if x["id"] != gid]
        add("N1' %s 缺失 → C1 必红" % gid, gm, True, "C1")

        gm = _green_map()
        for x in gm["modules"]:
            if x["id"] == CONSOLE:
                x["files"] = x["files"] + [GRID_GLOBS[gid]]
            if x["id"] == gid:
                x["files"] = []
        add("N4a' %s 回并 console-ui → C0 必红" % gid, gm, True, "C0")

        gm = _green_map()
        for x in gm["modules"]:
            if x["id"] == gid:
                x["files"] = x["files"] + ["easyvibe-renderer/src/components/shell/**"]
        add("N2' %s 再吞 shell/** → C3 必红" % gid, gm, True, "C3")

        add("N6' %s 注入越界 import → C5 必红" % gid, _green_map(), True, "C5",
            {GRID_PROBES[gid]: "import { x } from '@/pages/routes'\nexport const y = x\n"})

    # ---- 新格同层环（图边侧）
    m7 = _green_map()
    m7["edges"] = [{"id": "ex2", "from": "app-updater", "to": CONSOLE, "type": "import"}]
    add("N5' app-updater 出边指回 console-ui → C6 必红", m7, True, "C6")

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

    # ---- 真值双向断言（断言的是「判据本身工作」，不是「图已拆分」）
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
        problems, _checks = evaluate(d, REPO)
        pre_problems, _ = evaluate(_merge_back(d), REPO)
        red_post = bool(problems)
        red_pre = bool(pre_problems)
        ok = (not red_post) and red_pre
        failed += 0 if ok else 1
        detail = "%s %s 拆分后实际%s / 合成拆分前实际%s" % (
            src, os.path.basename(path), "红" if red_post else "绿", "红" if red_pre else "绿")
        if red_post and problems:
            detail += " ← " + problems[0]
        print("%s N4 真值双向断言（判据本身工作）  %s" % ("PASS" if ok else "FAIL", detail))

    total = len(cases) + 1
    print("selfcheck %d 例 %s" % (total, "全 PASS" if failed == 0 else "%d 项 FAIL" % failed))
    return 0 if failed == 0 else 1


def main():
    ap = argparse.ArgumentParser(description="console 拆格粒度守卫（R7 / c-arch-14 多格）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--map", default=DEFAULT_MAP)
    args = ap.parse_args()
    if args.selfcheck:
        return cmd_selfcheck()
    return cmd_check(args.map)


if __name__ == "__main__":
    sys.exit(main())
