#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""c-arch-7 / R7：easyvibe-app **DB 边界守卫的纯 python3 CI 载体**。

为什么需要它：`.github/workflows/asset-guard.yml` 是秒级轻量流水线（纯 python3，不跑 cargo），
若 c-arch-7 只落一个 Rust 测试文件（`crates/easyvibe-app/tests/arch_guard.rs`），则在 CI 语义下
「守卫不存在」。本脚本把**同一套判据**（routes/** 与 task_exec 生产不得直连 easyvibe_db /
task_exec 文件集双向全等 / 豁免棘轮 / 别名再导出反绕过）搬到 CI，并**解析 Rust 守卫的常量表**
作为单一事实源（零新清单文件、零双写）。

  --check     解析 tests/arch_guard.rs 的常量（DB_BANNED / ALIAS_REEXPORT / TASK_ENGINE_PROD /
              TASK_ENGINE_TEST_EXEMPT / TASK_ENGINE_TEST_BUDGET / ROUTES_REPO_FIELDS）
              → 对 src/ 复判。任一常量缺失/解析为空 = fail-closed（ok:false，退出 1），
              绝不静默全绿。判据一律用**原始文本**（不去注释）——最严口径。
  --selfcheck 合成输入 N1–N6（不改仓库；判据直接施于内存构造的 {相对路径: 文本}），
              逐条自证「负例必红、正例必绿」，其中 N5 专测「常量解析失败必红」的 fail-closed 路径。

退出码：--check 绿=0 / 红=1；--selfcheck 全 PASS=0 / 任一 FAIL=1。
"""

import json
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
APP = os.path.join(REPO, "easyvibe-backend", "crates", "easyvibe-app")
GUARD = os.path.join(APP, "tests", "arch_guard.rs")
SRC = os.path.join(APP, "src")

# 需解析的常量（任一缺失/解析为空即 fail-closed）
ARRAY_CONSTS = ("TASK_ENGINE_PROD", "TASK_ENGINE_TEST_EXEMPT", "ROUTES_REPO_FIELDS")
STR_CONSTS = ("DB_BANNED", "ALIAS_REEXPORT")
INT_CONSTS = ("TASK_ENGINE_TEST_BUDGET",)


class GuardParseError(Exception):
    """守卫常量解析失败——必须 fail-closed，不得静默跳过。"""


def _const_rhs(text, name):
    """与 check_map_domain_guard.py 同款：取 `const NAME: … = &[ … ];` 的方括号内容。"""
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


def parse_guard(text):
    """从 Rust 守卫源码解析常量表。任一缺失/解析为空即抛错（fail-closed）。"""
    arrays = {}
    for name in ARRAY_CONSTS:
        vals = re.findall(r'"([^"]+)"', _const_rhs(text, name))
        if not vals:
            raise GuardParseError("const %s 解析为空——fail-closed" % name)
        arrays[name] = vals
    strs = {}
    for name in STR_CONSTS:
        v = _const_str(text, name)
        if not v:
            raise GuardParseError("const %s 解析为空——fail-closed" % name)
        strs[name] = v
    ints = {}
    for name in INT_CONSTS:
        ints[name] = _const_int(text, name)  # 缺失/非数字均抛错
    return {
        "db_banned": strs["DB_BANNED"],
        "alias_reexport": strs["ALIAS_REEXPORT"],
        "task_engine_prod": arrays["TASK_ENGINE_PROD"],
        "task_engine_test_exempt": arrays["TASK_ENGINE_TEST_EXEMPT"],
        "task_engine_test_budget": ints["TASK_ENGINE_TEST_BUDGET"],
        "routes_repo_fields": arrays["ROUTES_REPO_FIELDS"],
    }


def _src_rel(path):
    """`src/xxx` 前缀 → 相对 `src/` 的键；非该前缀原样返回。"""
    p = str(path).replace("\\", "/")
    return p[len("src/"):] if p.startswith("src/") else p


def _norm_ws(s):
    return " ".join(s.split())


def evaluate(disk, consts):
    """对 {相对 src/ 的路径: 原始文本} 施加 R7 五类判据，返回问题列表（空 = 绿）。

    disk 的键形如 "routes/agent.rs" / "task_exec.rs" / "task_exec/ports.rs"。
    """
    problems = []
    db = consts["db_banned"]
    alias = consts["alias_reexport"]
    prod = consts["task_engine_prod"]
    exempt = consts["task_engine_test_exempt"]
    budget = consts["task_engine_test_budget"]
    fields = consts["routes_repo_fields"]

    # ① routes/**（含 mod.rs；路由无测试豁免面）：不得直连 DB、不得直取组合根仓储句柄。
    for rel in sorted(disk):
        rest = rel[len("routes/"):] if rel.startswith("routes/") else None
        if rest is None or "/" in rest or not rest.endswith(".rs"):
            continue
        text = disk[rel]
        if db in text:
            problems.append(
                "routes/%s 直连 `%s`——跨域写库须经 crate::service（c-arch-7 R1/R4）" % (rest, db)
            )
        for h in fields:
            if h in text:
                problems.append(
                    "routes/%s 直取组合根仓储句柄 `%s`——handler 只做 HTTP 边界（R1 冻结规则）"
                    % (rest, h)
                )

    # ② task_exec 生产文件：不得直连 DB。声明的生产文件缺失亦为 problem（防「删文件逃逸」）。
    for p in prod:
        rel = _src_rel(p)
        text = disk.get(rel)
        if text is None:
            problems.append("声明的生产文件缺失: %s——守卫不得因文件消失而静默放行" % rel)
            continue
        if db in text:
            problems.append(
                "%s 直连 `%s`——task-engine 持久化须经切片内端口 task_exec::ports（c-arch-7 R2/R4）"
                % (rel, db)
            )

    # ③ task_exec 磁盘文件集 == 生产(仅 task_exec/ 子项) ∪ 豁免（双向全等，禁 glob）。
    disk_te = sorted(
        rel[len("task_exec/"):]
        for rel in disk
        if rel.startswith("task_exec/")
        and "/" not in rel[len("task_exec/"):]
        and rel.endswith(".rs")
    )
    declared = set()
    for p in list(prod) + list(exempt):
        r = _src_rel(p)
        if r.startswith("task_exec/"):
            declared.add(r[len("task_exec/"):])
    declared_te = sorted(declared)
    if disk_te != declared_te:
        extra = [f for f in disk_te if f not in declared_te]
        missing = [f for f in declared_te if f not in disk_te]
        problems.append(
            "task_exec 文件集漂移 extra=%s missing=%s——新增/删除/改名须同步 "
            "TASK_ENGINE_PROD / TASK_ENGINE_TEST_EXEMPT（豁免须显式登记）" % (extra, missing)
        )

    # ④ 豁免棘轮（只降不升）：豁免集 DB_BANNED 出现次数 > 预算即红。
    used = 0
    for p in exempt:
        rel = _src_rel(p)
        text = disk.get(rel)
        if text is None:
            problems.append("声明的豁免文件缺失: %s——豁免不得凭空登记" % rel)
            continue
        used += text.count(db)
    if used > budget:
        problems.append(
            "task_exec 测试夹具 `%s` 出现 %d 次 > 预算 %d——棘轮只降不升（c-arch-7 R4）"
            % (db, used, budget)
        )

    # ⑤ src/** 不得 `use easyvibe_db as <别名>` 再导出（归一化空白后子串，堵 crate::<别名>::X 逃逸）。
    for rel in sorted(disk):
        if not rel.endswith(".rs"):
            continue
        if alias in _norm_ws(disk[rel]):
            problems.append(
                "%s 出现 `%s<别名>` 再导出——禁 DB 边界判据可被 `crate::<别名>::X` 绕过（c-arch-7 R5a）"
                % (rel, alias)
            )

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
            consts = parse_guard(f.read())
    except (OSError, GuardParseError) as e:
        print(json.dumps({"ok": False, "stage": "parse-guard", "problems": [str(e)]}, ensure_ascii=False))
        return 1
    disk = read_src()
    problems = evaluate(disk, consts)
    report = {"ok": not problems, "files": len(disk), "problems": problems}
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if not problems else 1


# ---------------------------------------------------------------------------
# --selfcheck：合成输入（不读 / 不改仓库源码；判据直接施于内存构造的 {rel: 文本}）
# ---------------------------------------------------------------------------

def _green(consts):
    """全干净合成输入：生产/豁免/路由各就位，文本零 easyvibe_db、零别名再导出。"""
    d = {}
    for p in consts["task_engine_prod"]:
        d[_src_rel(p)] = "// synthetic prod\npub fn prod() {}\n"
    for p in consts["task_engine_test_exempt"]:
        d[_src_rel(p)] = "// synthetic test\n#[test]\nfn t() {}\n"
    d["routes/mod.rs"] = "// synthetic routes\npub mod agent;\n"
    d["lib.rs"] = "// synthetic lib\n"
    return d


def cmd_selfcheck():
    try:
        with open(GUARD, encoding="utf-8") as f:
            guard_text = f.read()
        consts = parse_guard(guard_text)
    except (OSError, GuardParseError) as e:
        print("FAIL 无法加载守卫常量: %s" % e)
        return 1

    cases = []  # (name, disk_or_none, expect_red, consts_used, raw_guard)

    base = _green(consts)

    # N1 生产/边界文件含 `use easyvibe_db::TaskRepository;` → 必红
    d1 = dict(base)
    d1[_src_rel(consts["task_engine_prod"][0])] += "use easyvibe_db::TaskRepository;\n"
    cases.append(("N1 生产文件直连 easyvibe_db", d1, True, consts, None))

    # N2 src 出现 `pub(crate) use easyvibe_db as db;` → 必红（别名再导出）
    d2 = dict(base)
    d2["lib.rs"] += "pub(crate) use easyvibe_db as db;\n"
    cases.append(("N2 别名再导出 use easyvibe_db as db", d2, True, consts, None))

    # N3 豁免文件含 easyvibe_db → 不红（在豁免面内，且计数在预算内）
    d3 = dict(base)
    d3[_src_rel(consts["task_engine_test_exempt"][0])] += "let r = easyvibe_db::SqliteTaskRepository::new(pool);\n"
    cases.append(("N3 豁免文件 easyvibe_db（预算内）不红", d3, False, consts, None))

    # N4 新增未登记 src/task_exec/tests_new.rs → 必红（清单双向）
    d4 = dict(base)
    d4["task_exec/tests_new.rs"] = "// unregistered\n#[test]\nfn t() {}\n"
    cases.append(("N4 未登记 task_exec 新文件", d4, True, consts, None))

    # N5 守卫常量被改名/删除 → 解析失败必红（fail-closed）
    mutated = re.sub(r"const\s+DB_BANNED\s*:[^;]*;", "", guard_text)
    cases.append(("N5 常量被删除 → 解析失败 fail-closed", None, True, consts, mutated))

    # N6 全干净合成输入 → 必绿
    cases.append(("N6 全干净合成输入", dict(base), False, consts, None))

    failed = 0
    for name, disk, expect_red, used, raw in cases:
        if raw is not None:
            try:
                parse_guard(raw)
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
    print("N1–N6 %s" % ("全 PASS" if failed == 0 else "%d 项 FAIL" % failed))
    return 0 if failed == 0 else 1


def main():
    arg = sys.argv[1] if len(sys.argv) > 1 else "--check"
    if arg == "--selfcheck":
        return cmd_selfcheck()
    if arg == "--check":
        return cmd_check()
    print("用法: check_app_db_boundary.py [--check|--selfcheck]")
    return 2


if __name__ == "__main__":
    sys.exit(main())
