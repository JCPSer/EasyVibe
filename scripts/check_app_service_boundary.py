#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""c-arch-10 / R7：easyvibe-app **编排层端口化** 与 **装配格单向** 的纯 python3 CI 载体。

为什么需要它：`.github/workflows/asset-guard.yml` 是秒级轻量流水线（纯 python3，不跑 cargo），
若 c-arch-10 只落 Rust 守卫（`crates/easyvibe-app/tests/arch_guard.rs`），则在 CI 语义下
「守卫不存在」。本脚本把**同一套判据**搬到 CI，并**解析 Rust 守卫的常量表**作为单一事实源
（零新清单文件、零双写）。

  --check     解析 tests/arch_guard.rs 常量（SERVICE_DB_RATCHET / SERVICE_BANNED_TYPES /
              ASSEMBLY_REVERSE）与 tests/module_size_guard.rs 常量（SERVICE_FROZEN_FILES）
              → 对 src/ 复判。任一常量缺失/解析为空 = fail-closed（ok:false，退出 1），
              绝不静默全绿。判据一律用**原始文本**（不去注释）——最严口径。
  --selfcheck 合成输入 N1–N8（不改仓库；判据直接施于内存构造的 {相对路径: 文本}），
              逐条自证「负例必红、正例必绿」，其中 N5 专测「常量解析失败必红」的 fail-closed 路径。

c-arch-13（R8）扩面：J5 —— `service/**` 每文件 ≤ `gates.single_file_loc_max`（默认 400；`mod.rs` ≤300），
且磁盘 service/ 文件集 == `SERVICE_FROZEN_FILES`（双向全等）；存量超标（`service/chat.rs` 490）按
`gates.single_file_loc_caps` 棘轮豁免（与 I10 同源）。

退出码：--check 绿=0 / 红=1；--selfcheck 全 PASS=0 / 任一 FAIL=1。
"""

import json
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
APP = os.path.join(REPO, "easyvibe-backend", "crates", "easyvibe-app")
GUARD = os.path.join(APP, "tests", "arch_guard.rs")
# c-arch-13：SERVICE_FROZEN_FILES 的单一事实源在 Rust 冻结表；阈值/棘轮单一事实源在 policy.gates。
SIZE_GUARD = os.path.join(APP, "tests", "module_size_guard.rs")
POLICY = os.path.join(REPO, "scripts", "map_edge_policy.json")
SRC = os.path.join(APP, "src")
APP_SRC_PREFIX = "easyvibe-backend/crates/easyvibe-app/src/"
SERVICE_REL = "service/"

# 装配格探针（J3）：文件缺失 = fail-closed（装配格被并回 server-api 时必红）
ASSEMBLY_PROBES = ["assembly/mod.rs", "assembly/bridges.rs", "assembly/schedulers.rs", "assembly/static_host.rs"]


class GuardParseError(Exception):
    """守卫常量解析失败——必须 fail-closed，不得静默跳过。"""


def _const_rhs(text, name):
    m = re.search(r"const\s+%s\s*:[^=]*?=\s*&\[(.*?)\];" % re.escape(name), text, re.S)
    if not m:
        raise GuardParseError("const %s 未匹配到（守卫被重排/改名？fail-closed）" % name)
    return m.group(1)


def _const_str(text, name):
    m = re.search(r'const\s+%s\s*:[^=]*?=\s*"([^"]*)"\s*;' % re.escape(name), text, re.S)
    if not m:
        raise GuardParseError("const %s 未匹配到（守卫被重排/改名？fail-closed）" % name)
    return m.group(1)


def _const_int(text, name):
    m = re.search(r"const\s+%s\s*:[^=]*?=\s*(\d+)\s*;" % re.escape(name), text, re.S)
    if not m:
        raise GuardParseError("const %s 未匹配到（守卫被重排/改名？fail-closed）" % name)
    return int(m.group(1))


def parse_guard(text, size_text):
    """从 Rust 守卫源码解析常量表。任一缺失/解析为空即抛错（fail-closed）。"""
    banned = re.findall(r'"([^"]+)"', _const_rhs(text, "SERVICE_BANNED_TYPES"))
    if not banned:
        raise GuardParseError("const SERVICE_BANNED_TYPES 解析为空——fail-closed")
    reverse = _const_str(text, "ASSEMBLY_REVERSE")
    if not reverse:
        raise GuardParseError("const ASSEMBLY_REVERSE 解析为空——fail-closed")
    frozen = re.findall(r'"([^"]+)"', _const_rhs(size_text, "SERVICE_FROZEN_FILES"))
    if not frozen:
        raise GuardParseError("const SERVICE_FROZEN_FILES 解析为空——fail-closed")
    return {
        "db_ratchet": _const_int(text, "SERVICE_DB_RATCHET"),
        "banned": banned,
        "assembly_reverse": reverse,
        "service_frozen": frozen,
    }


def load_loc_gates():
    """读 `scripts/map_edge_policy.json::gates` 的单文件行数阈值与棘轮（c-arch-13；缺失 fail-closed）。

    返回 (single_file_loc_max:int, loc_caps:{src 相对路径: cap})——棘轮键按 `src/` 相对路径归一，
    与 evaluate/disk 的键空间一致。
    """
    with open(POLICY, encoding="utf-8") as f:
        gates = json.load(f)["gates"]
    loc_max = int(gates["single_file_loc_max"])
    caps = {}
    for k, v in dict(gates["single_file_loc_caps"]).items():
        kk = k[len(APP_SRC_PREFIX):] if k.startswith(APP_SRC_PREFIX) else k
        caps[kk] = int(v)
    return loc_max, caps


def evaluate(disk, consts):
    """对 {相对 src/ 的路径: 原始文本} 施加 c-arch-10 判据，返回问题列表（空 = 绿）。"""
    problems = []
    ratchet = consts["db_ratchet"]
    banned = consts["banned"]
    reverse = consts["assembly_reverse"]

    # J1/J2：service/** 不得直连 easyvibe_db、不得出现具体仓储类型名。
    used = 0
    for rel in sorted(disk):
        rest = rel[len(SERVICE_REL):] if rel.startswith(SERVICE_REL) else None
        if rest is None or "/" in rest or not rest.endswith(".rs"):
            continue
        text = disk[rel]
        used += text.count("easyvibe_db")
        for t in banned:
            if t in text:
                problems.append("%s 出现具体仓储类型名 `%s`——service 须只见 crate::db_ports 端口与本地 DTO（J2）" % (rel, t))
    if used > ratchet:
        problems.append("service/** 直连 `easyvibe_db` 出现 %d 次 > 棘轮 %d（只降不升，J1）" % (used, ratchet))

    # J3：装配格探针必须存在（缺失 = 装配格被并回 server-api，fail-closed）。
    for probe in ASSEMBLY_PROBES:
        if probe not in disk:
            problems.append("装配格探针缺失: %s——装配格被并回 server-api（J3，fail-closed）" % probe)

    # J4a：装配格不得 `crate::routes`（原 bootstrap.rs 手工构造 State/Path 调 start_patrol 的层次倒置）。
    # 注：router.rs 是边界层的 merge 聚合点，`use crate::routes::{…}` 是其本职，不在判据内。
    for rel in sorted(disk):
        if rel.startswith("assembly/"):
            if "use crate::routes" in disk[rel] or "crate::routes::" in disk[rel]:
                problems.append("%s 出现 `crate::routes`——装配不得反向依赖路由边界（J4）" % rel)
    # J4b：非装配格 src 文件不得 `crate::assembly`（装配 → server-api 单向）。
    for rel in sorted(disk):
        if rel.startswith("assembly/") or rel == "main.rs":
            continue
        if reverse in disk[rel]:
            problems.append("%s 出现 `%s`——server-api 反向引用装配格（J4）" % (rel, reverse))

    # J5（c-arch-13）：service/** 每文件 ≤ single_file_loc_max（mod.rs ≤300）+ 文件集双向全等。
    loc_max = consts["loc_max"]
    caps = consts["loc_caps"]
    disk_svc = sorted(
        rel[len(SERVICE_REL):]
        for rel in disk
        if rel.startswith(SERVICE_REL) and "/" not in rel[len(SERVICE_REL):] and rel.endswith(".rs")
    )
    for f in disk_svc:
        rel = SERVICE_REL + f
        n = len(disk[rel].splitlines())
        lim = 300 if f == "mod.rs" else int(caps.get(rel, loc_max))
        if n > lim:
            problems.append("%s = %d 行 > 上限 %d（service/** 每文件须 ≤%d；存量超标须登记 "
                            "single_file_loc_caps；J5）" % (rel, n, lim, loc_max))
    declared_svc = sorted(set(consts["service_frozen"]))
    if disk_svc != declared_svc:
        extra = [f for f in disk_svc if f not in declared_svc]
        missing = [f for f in declared_svc if f not in disk_svc]
        problems.append("service/ 文件集漂移 extra=%s missing=%s——新增/改名须同步 "
                        "SERVICE_FROZEN_FILES（c-arch-13 R7/J5）" % (extra, missing))

    return problems


def read_src():
    """递归读取 src/**/*.rs，键 = 相对 src/ 的 posix 路径。"""
    out = {}
    for root, _dirs, files in os.walk(SRC):
        for name in files:
            if not name.endswith(".rs"):
                continue
            full = os.path.join(root, name)
            rel = os.path.relpath(full, SRC).replace(os.sep, "/")
            with open(full, encoding="utf-8") as f:
                out[rel] = f.read()
    return out


def cmd_check():
    try:
        with open(GUARD, encoding="utf-8") as f:
            consts = parse_guard(f.read(), open(SIZE_GUARD, encoding="utf-8").read())
        consts["loc_max"], consts["loc_caps"] = load_loc_gates()
    except (OSError, GuardParseError, KeyError, ValueError) as e:
        print(json.dumps({"ok": False, "stage": "parse-guard", "problems": [str(e)]}, ensure_ascii=False))
        return 1
    disk = read_src()
    problems = evaluate(disk, consts)
    report = {
        "ok": not problems,
        "files": len(disk),
        "service_files": sorted(
            rel[len(SERVICE_REL):] for rel in disk
            if rel.startswith(SERVICE_REL) and "/" not in rel[len(SERVICE_REL):] and rel.endswith(".rs")
        ),
        "problems": problems,
    }
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if not problems else 1


# ---------------------------------------------------------------------------
# --selfcheck：合成输入（不读 / 不改仓库源码；判据直接施于内存构造的 {rel: 文本}）
# ---------------------------------------------------------------------------

def _green(consts):
    """全干净合成输入：service 端口化、装配格探针齐备、无反向引用、service 文件集齐备且 ≤400。"""
    d = {}
    for probe in ASSEMBLY_PROBES:
        d[probe] = "// synthetic assembly\n"
    for f in consts["service_frozen"]:
        d["service/%s" % f] = "// synthetic service\npub fn f() {}\n"
    d["main.rs"] = "mod assembly;\n"
    d["router.rs"] = "pub fn build_router() {}\n"
    return d


def cmd_selfcheck():
    try:
        with open(GUARD, encoding="utf-8") as f:
            guard_text = f.read()
        with open(SIZE_GUARD, encoding="utf-8") as f:
            size_text = f.read()
        consts = parse_guard(guard_text, size_text)
        consts["loc_max"], consts["loc_caps"] = load_loc_gates()
    except (OSError, GuardParseError, KeyError, ValueError) as e:
        print("FAIL 无法加载守卫常量: %s" % e)
        return 1

    cases = []  # (name, disk_or_none, expect_red, consts_used, raw_guard)
    base = _green(consts)

    # N1 service 回连 easyvibe_db → 必红
    d1 = dict(base)
    d1["service/task.rs"] += "use easyvibe_db::TaskRepository;\n"
    cases.append(("N1 service 直连 easyvibe_db", d1, True, consts, None))

    # N2 service 出现具体仓储类型名 → 必红
    d2 = dict(base)
    d2["service/chat.rs"] = "let r = SqliteConversationRepository::new(pool);\n"
    cases.append(("N2 service 出现具体仓储类型名", d2, True, consts, None))

    # N3 装配格探针缺失（回并 server-api）→ 必红
    d3 = dict(base)
    d3.pop("assembly/mod.rs")
    cases.append(("N3 装配格探针缺失", d3, True, consts, None))

    # N4 非装配格文件反向引用装配格 → 必红
    d4 = dict(base)
    d4["service/map.rs"] = "use crate::assembly::static_host;\n"
    cases.append(("N4 server-api 反向引用装配格", d4, True, consts, None))

    # N5 守卫常量被改名/删除 → 解析失败必红（fail-closed）
    mutated = re.sub(r"const\s+SERVICE_BANNED_TYPES\s*:[^;]*;", "", guard_text)
    cases.append(("N5 常量被删除 → 解析失败 fail-closed", None, True, consts, mutated))

    # N6 全干净合成输入 → 必绿
    cases.append(("N6 全干净合成输入", dict(base), False, consts, None))

    # N7（c-arch-13）：合成 401 行 service/x.rs → 必红（J5 每文件 ≤400）
    d7 = dict(base)
    d7["service/x.rs"] = "pub fn f() {}\n" * 401
    cases.append(("N7 合成 401 行 service/x.rs → 必红", d7, True, consts, None))

    # N8（c-arch-13）：全 service 文件在限内且文件集齐备 → 必绿
    cases.append(("N8 service 文件集齐备且 ≤400 → 必绿", dict(base), False, consts, None))

    failed = 0
    for name, disk, expect_red, used, raw in cases:
        if raw is not None:
            try:
                parse_guard(raw, size_text)
                red = False
                detail = "未按预期抛错（改名/删除未被 fail-closed 拦截）"
            except GuardParseError as e:
                red = True
                detail = "解析失败 → fail-closed: %s" % e
        else:
            probs = evaluate(disk, used)
            red = bool(probs)
            detail = "" if not probs else probs[0]
        ok = red == expect_red
        failed += 0 if ok else 1
        print("%s %s%s" % ("PASS" if ok else "FAIL", name, ("  " + detail) if detail else ""))
    print("N1–N8 %s" % ("全 PASS" if failed == 0 else "%d 项 FAIL" % failed))
    return 0 if failed == 0 else 1


def main():
    arg = sys.argv[1] if len(sys.argv) > 1 else "--check"
    if arg == "--selfcheck":
        return cmd_selfcheck()
    if arg == "--check":
        return cmd_check()
    print("用法: check_app_service_boundary.py [--check|--selfcheck]")
    return 2


if __name__ == "__main__":
    sys.exit(main())
