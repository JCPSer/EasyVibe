#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""R10③b（c-arch-1）：easyvibe-map 领域守卫的**纯 python3 CI 载体**。

为什么需要它：`.github/workflows/asset-guard.yml` 是秒级轻量流水线（纯 python3，不跑 cargo），
若 R10③ 只落一个 Rust 测试文件，则在 CI 语义下「守卫不存在」。本脚本把**同一套判据**
（文件集双向全等 / 单文件 LOC / 禁用依赖面 / 导出符号冻结）搬到 CI，并**解析 Rust 守卫
的常量表**作为单一事实源（零新清单文件、零双写）。

  --check     解析 crates/easyvibe-map/tests/module_size_guard.rs 的常量 → 对 src/ 复判。
              常量解析失败 = fail-closed（ok:false），绝不静默全绿。
              附带 Q-C 低强度一致性检查：地图文案若宣称 module_size_guard，则该文件必须存在。
  --selfcheck 合成输入 N1–N6（不读仓库、不改仓库），逐条自证「负例必红、正例必绿」，
              其中 N5 专测「常量解析失败必红」的 fail-closed 路径。

退出码：--check 绿=0 / 红=1；--selfcheck 全 PASS=0 / 任一 FAIL=1。
"""

import json
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GUARD = os.path.join(REPO, "easyvibe-backend", "crates", "easyvibe-map", "tests", "module_size_guard.rs")
SRC_DIR = os.path.join(REPO, "easyvibe-backend", "crates", "easyvibe-map", "src")
PARTS_DIR = os.path.join(REPO, ".easyvibe", "map", "parts")
NOTE_FIXTURE = os.path.join(REPO, "scripts", "tests", "fixtures", "map_post_ownership.json")
CRATE_PREFIX = "easyvibe-backend/crates/"

CONST_NAMES = ("FROZEN_FILES", "FORBIDDEN", "FROZEN_SYMBOLS", "LOC_LIMITS")


class GuardParseError(Exception):
    """守卫常量解析失败——必须 fail-closed，不得静默跳过。"""


def _const_rhs(text, name):
    m = re.search(r"const\s+%s\s*:[^=]*?=\s*&\[(.*?)\];" % re.escape(name), text, re.S)
    if not m:
        raise GuardParseError("const %s 未匹配到（守卫被重排/改名？fail-closed）" % name)
    return m.group(1)


def parse_guard(text):
    """从 Rust 守卫源码解析四张常量表。任一为空即抛错（fail-closed）。"""
    frozen = re.findall(r'"([^"]+)"', _const_rhs(text, "FROZEN_FILES"))
    forbidden = re.findall(r'"([^"]+)"', _const_rhs(text, "FORBIDDEN"))
    symbols = re.findall(r'"([^"]+)"', _const_rhs(text, "FROZEN_SYMBOLS"))
    loc_pairs = re.findall(r'\("([^"]+)",\s*(\d+)\)', _const_rhs(text, "LOC_LIMITS"))
    if not (frozen and forbidden and symbols and loc_pairs):
        raise GuardParseError(
            "常量解析为空（files=%d forbidden=%d symbols=%d loc=%d）——fail-closed"
            % (len(frozen), len(forbidden), len(symbols), len(loc_pairs))
        )
    return {
        "files": frozen,
        "forbidden": forbidden,
        "symbols": symbols,
        "loc": {name: int(n) for name, n in loc_pairs},
    }


def evaluate(files, consts):
    """对 {文件名: 文本} 施加四类判据，返回问题列表（空 = 绿）。"""
    problems = []
    expected = sorted(consts["files"])
    actual = sorted(files)
    if actual != expected:
        extra = [f for f in actual if f not in expected]
        missing = [f for f in expected if f not in actual]
        problems.append("文件集漂移 extra=%s missing=%s" % (extra, missing))

    default_lim = consts["loc"].get("*")
    for name in sorted(files):
        limit = consts["loc"].get(name, default_lim)
        if limit is None:
            problems.append("LOC_LIMITS 未覆盖 %s 且无默认值" % name)
            continue
        n = len(files[name].splitlines())
        if n > limit:
            problems.append("%s 超 %d 行: %d" % (name, limit, n))

    for name in sorted(files):
        for bad in consts["forbidden"]:
            if bad in files[name]:
                problems.append("%s 出现禁用依赖面 `%s`" % (name, bad))

    joined = "\n".join(files[n] for n in sorted(files))
    for sym in consts["symbols"]:
        if sym not in joined:
            problems.append("导出符号缺失 `%s`" % sym)
    return problems


def read_src():
    out = {}
    for name in sorted(os.listdir(SRC_DIR)):
        if name.endswith(".rs"):
            with open(os.path.join(SRC_DIR, name), encoding="utf-8") as f:
                out[name] = f.read()
    return out


def _crate_dir(globs):
    """从模块 files glob 推回 crate 目录（仅后端 crate；前端/其他返回 None）。"""
    for g in globs or []:
        g = str(g).replace("\\", "/")
        if g.startswith(CRATE_PREFIX):
            return os.path.join("easyvibe-backend", "crates", g[len(CRATE_PREFIX):].split("/")[0])
    return None


def note_consistency(modules):
    """Q-C 低强度断言：review_note 若宣称 module_size_guard，则该守卫文件必须存在。"""
    problems = []
    for m in modules:
        if not isinstance(m, dict):
            continue
        note = ((m.get("health") or {}) if isinstance(m.get("health"), dict) else {}).get("review_note") or ""
        if "module_size_guard" not in note:
            continue
        crate = _crate_dir(m.get("files"))
        if not crate:
            continue
        guard = os.path.join(REPO, crate, "tests", "module_size_guard.rs")
        if not os.path.exists(guard):
            problems.append("模块 %s 文案宣称自带 module_size_guard，但 %s 不存在" % (m.get("id"), guard))
    return problems


def collect_modules():
    mods = []
    if os.path.isdir(PARTS_DIR):
        for name in sorted(os.listdir(PARTS_DIR)):
            if not name.endswith(".json") or name.endswith(".edges.json") or name.startswith("_"):
                continue
            try:
                with open(os.path.join(PARTS_DIR, name), encoding="utf-8") as f:
                    mods.append(json.load(f))
            except (OSError, ValueError):
                continue
    if os.path.exists(NOTE_FIXTURE):
        try:
            with open(NOTE_FIXTURE, encoding="utf-8") as f:
                mods.extend(json.load(f).get("modules", []))
        except (OSError, ValueError):
            pass
    return mods


def cmd_check():
    try:
        with open(GUARD, encoding="utf-8") as f:
            consts = parse_guard(f.read())
    except (OSError, GuardParseError) as e:
        print(json.dumps({"ok": False, "stage": "parse-guard", "problems": [str(e)]}, ensure_ascii=False))
        return 1
    files = read_src()
    problems = evaluate(files, consts)
    problems += note_consistency(collect_modules())
    report = {
        "ok": not problems,
        "files": sorted(files),
        "loc": {n: len(t.splitlines()) for n, t in sorted(files.items())},
        "forbidden": consts["forbidden"],
        "frozen_symbols": len(consts["symbols"]),
        "problems": problems,
    }
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if not problems else 1


def _green(consts):
    body = "// synthetic\n" + "\n".join(consts["symbols"]) + "\n"
    return {f: body for f in consts["files"]}


def cmd_selfcheck():
    try:
        with open(GUARD, encoding="utf-8") as f:
            guard_text = f.read()
        consts = parse_guard(guard_text)
    except (OSError, GuardParseError) as e:
        print("FAIL 无法加载守卫常量: %s" % e)
        return 1

    cases = []

    def add(name, files_or_none, expect_red, consts_used=None, raw=None):
        cases.append((name, files_or_none, expect_red, consts_used or consts, raw))

    f = _green(consts)
    f1 = dict(f)
    f1["draft.rs"] = "// draft\n"
    add("N1 多一个文件 draft.rs", f1, True)

    f2 = dict(f)
    f2["lib.rs"] = "\n".join(["// pad"] * 451) + "\n"
    add("N2 lib.rs 451 行", f2, True)

    f3 = dict(f)
    f3["freshness.rs"] = f3["freshness.rs"] + "// axum\n"
    add("N3 文本含 axum", f3, True)

    f4 = {name: "\n".join(consts["symbols"][1:]) + "\n" for name in consts["files"]}
    add("N4 少一个冻结符号", f4, True)

    add("N5 常量解析失败", None, True, raw="const FROZEN_FILES: &[&str] = &[\n")

    add("N6 正例", dict(f), False)

    failed = 0
    for name, files, expect_red, used, raw in cases:
        if raw is not None:
            try:
                parse_guard(raw)
                red = False
                detail = "未按预期抛错"
            except GuardParseError:
                red = True
                detail = "解析失败 → fail-closed"
        else:
            probs = evaluate(files, used)
            red = bool(probs)
            detail = "" if not probs else probs[0]
        ok = red == expect_red
        failed += 0 if ok else 1
        print("%s %s%s" % ("PASS" if ok else "FAIL", name, ("  " + detail) if detail else ""))
    print("N1–N6 %s" % ("全 PASS" if failed == 0 else "%d 项 FAIL" % failed))
    return 0 if failed == 0 else 1


def main():
    arg = sys.argv[1] if len(sys.argv) > 1 else "--check"
    if arg == "--selfcheck":
        return cmd_selfcheck()
    if arg == "--check":
        return cmd_check()
    print("用法: check_map_domain_guard.py [--check|--selfcheck]")
    return 2


if __name__ == "__main__":
    sys.exit(main())
