#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""c-arch-7 / R7：easyvibe-app **DB 边界守卫的纯 python3 CI 载体**。

为什么需要它：`.github/workflows/asset-guard.yml` 是秒级轻量流水线（纯 python3，不跑 cargo），
若 c-arch-7 只落一个 Rust 测试文件（`crates/easyvibe-app/tests/arch_guard.rs`），则在 CI 语义下
「守卫不存在」。本脚本把**同一套判据**（routes/** 与 task_exec 生产不得直连 easyvibe_db /
task_exec 文件集双向全等 / 豁免棘轮 / 别名再导出反绕过）搬到 CI，并**解析 Rust 守卫的常量表**
作为单一事实源（零新清单文件、零双写）。

  --check     解析 tests/arch_guard.rs 的常量（DB_BANNED / ALIAS_REEXPORT / TASK_ENGINE_PROD /
              TASK_ENGINE_TEST_EXEMPT / TASK_ENGINE_TEST_BUDGET / ROUTES_REPO_FIELDS
              / DB_DIRECT_LITERAL / DB_DIRECT_TOTAL / DB_DIRECT_FOCUS_MAX / DB_DIRECT_FOCUS_FILES_MAX
              / DB_PORTS_DOMAIN_OWNERSHIP（嵌套表））
              → 对 src/ 复判。任一常量缺失/解析为空 = fail-closed（ok:false，退出 1），
              绝不静默全绿。判据一律用**原始文本**（不去注释）——最严口径。
  --selfcheck 合成输入 N1–N24（不改仓库；判据直接施于内存构造的 {相对路径: 文本}），
              逐条自证「负例必红、正例必绿」，其中 N5 专测「常量解析失败必红」的 fail-closed 路径；
              N7/N8/N9 为 c-arch-13 焦点面直连集中度（I11b 超限 / 登记值正例 / I11a 总数漂移）；
              N23/N24 为 c-arch-16 R6（db_ports 跨域符号必红 / 结构化嵌套表解析失败 fail-closed）。

c-arch-13（R8）扩面：焦点面（`src/db_ports.rs` 或 `src/db_ports/**` + `src/state.rs`）复判三量——
I11a 守恒律（`easyvibe_db::` 总处数 == DB_DIRECT_TOTAL）/ I11b 单文件最大 == DB_DIRECT_FOCUS_MAX
（**双边全等**）/ I11c 有直连文件数 == DB_DIRECT_FOCUS_FILES_MAX；`--check` 输出 `db_direct_by_file`。

c-arch-16（R6）扩面：解析 `arch_guard.rs::DB_PORTS_DOMAIN_OWNERSHIP`（**嵌套表**，外层平衡方括号
定界 + 逐行结构化解析，解析失败 fail-closed）复判——每个 `db_ports/*.rs` 出现的
`easyvibe_db::<符号>` 必须落在该文件的允许集内，且磁盘文件集与冻结表键集双向全等（禁 glob）。

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
STR_CONSTS = ("DB_BANNED", "ALIAS_REEXPORT", "DB_DIRECT_LITERAL")
# c-arch-13：焦点面直连集中度三量（I11a 守恒 / I11b 单文件最大 / I11c 落点文件数）
INT_CONSTS = ("TASK_ENGINE_TEST_BUDGET", "DB_DIRECT_TOTAL", "DB_DIRECT_FOCUS_MAX",
              "DB_DIRECT_FOCUS_FILES_MAX")


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


def _strip_rs_comments(s):
    """去 Rust 行/块注释（保留字符串字面量）；供结构化表解析在无注释文本上定位括号。"""
    out = []
    i, n = 0, len(s)
    state = 0  # 0=code 1=line 2=block 3=str
    while i < n:
        c = s[i]
        nxt = s[i + 1] if i + 1 < n else ""
        if state == 0:
            if c == "/" and nxt == "/":
                state = 1
                i += 2
                continue
            if c == "/" and nxt == "*":
                state = 2
                i += 2
                continue
            if c == '"':
                state = 3
            out.append(c)
        elif state == 1:
            if c == "\n":
                state = 0
                out.append(c)
        elif state == 2:
            if c == "*" and nxt == "/":
                state = 0
                i += 2
                continue
        else:  # str
            out.append(c)
            if c == "\\" and nxt:
                out.append(nxt)
                i += 2
                continue
            if c == '"':
                state = 0
        i += 1
    return "".join(out)


# 结构化嵌套表常量（`&[(file, &[symbols])]`）。ΔS5：**不可**用扁平 findall——
# 那会把文件名与符号名混进同一列表并静默错判。改由 _const_rows 外层定界 + 行解析，失败即红。
TABLE_CONSTS = ("DB_PORTS_DOMAIN_OWNERSHIP",)


def _const_rows(text, name):
    """解析 `const NAME: &[(&str, &[&str])] = &[ ("f", &["s", …]), … ];` → {file: [symbols]}。

    外层用**平衡方括号**定界（避免与内层 `&[…]` 混淆）；逐行结构化匹配。任一行/整体形态不符
    ⇒ GuardParseError（**fail-closed**；绝不静默回退到扁平 findall）。
    """
    stripped = _strip_rs_comments(text)
    m = re.search(r"const\s+%s\s*:[^=]*?=\s*&\[" % re.escape(name), stripped, re.S)
    if not m:
        raise GuardParseError("const %s 未匹配到（守卫被重排/改名？fail-closed）" % name)
    depth, j, n = 1, m.end(), len(stripped)
    while j < n and depth > 0:
        ch = stripped[j]
        if ch == "[":
            depth += 1
        elif ch == "]":
            depth -= 1
        j += 1
    if depth != 0:
        raise GuardParseError("const %s 方括号不平衡（fail-closed）" % name)
    outer = stripped[m.end():j - 1]
    rows, cur, depth, in_str, esc = [], [], 0, False, False
    for ch in outer:
        if in_str:
            cur.append(ch)
            if esc:
                esc = False
            elif ch == "\\":
                esc = True
            elif ch == '"':
                in_str = False
            continue
        if ch == '"':
            in_str = True
            cur.append(ch)
        elif ch in "([":
            depth += 1
            cur.append(ch)
        elif ch in ")]":
            depth -= 1
            cur.append(ch)
        elif ch == "," and depth == 0:
            rows.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    if "".join(cur).strip():
        rows.append("".join(cur))
    rows = [r.strip() for r in rows if r.strip()]
    if not rows:
        raise GuardParseError("const %s 解析为空——fail-closed" % name)
    table, order = {}, []
    for r in rows:
        rm = re.match(r'^\(\s*"([^"]+)"\s*,\s*&\[(.*)\]\s*,?\s*\)$', r, re.S)
        if not rm:
            raise GuardParseError(
                "const %s 行结构不符（须为 (\"file\", &[\"sym\", …])，fail-closed）: %r"
                % (name, r[:80])
            )
        fname = rm.group(1)
        if fname in table:
            raise GuardParseError("const %s 文件名重复 `%s`（fail-closed）" % (name, fname))
        table[fname] = re.findall(r'"([^"]+)"', rm.group(2))
        order.append(fname)
    return table, order


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
    tables = {}
    for name in TABLE_CONSTS:
        tables[name] = _const_rows(text, name)[0]  # 结构化解析；失败即 GuardParseError
    return {
        "db_banned": strs["DB_BANNED"],
        "alias_reexport": strs["ALIAS_REEXPORT"],
        "db_direct_literal": strs["DB_DIRECT_LITERAL"],
        "task_engine_prod": arrays["TASK_ENGINE_PROD"],
        "task_engine_test_exempt": arrays["TASK_ENGINE_TEST_EXEMPT"],
        "task_engine_test_budget": ints["TASK_ENGINE_TEST_BUDGET"],
        "routes_repo_fields": arrays["ROUTES_REPO_FIELDS"],
        # c-arch-13
        "db_direct_total": ints["DB_DIRECT_TOTAL"],
        "db_direct_focus_max": ints["DB_DIRECT_FOCUS_MAX"],
        "db_direct_focus_files_max": ints["DB_DIRECT_FOCUS_FILES_MAX"],
        # c-arch-16 R6：db_ports 单端口域归属表（{文件: [允许的 easyvibe_db 符号]}）
        "db_ports_domain_ownership": tables["DB_PORTS_DOMAIN_OWNERSHIP"],
    }


def focus_direct_table(disk, literal):
    """焦点面逐文件 `easyvibe_db::` 出现次数（键 = 相对 src/ 路径）。

    焦点面 = `db_ports.rs`（目录化前）或 `db_ports/**`（目录化后）+ `state.rs`——以磁盘实际为准
    （两者取存在者；read_src 已把 db_ports/ 展开为 "db_ports/<f>.rs"）。
    """
    out = {}
    for rel in sorted(disk):
        if rel == "state.rs" or rel == "db_ports.rs" or (rel.startswith("db_ports/") and rel.endswith(".rs")):
            out[rel] = disk[rel].count(literal)
    return out


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

    # ⑥ c-arch-13：焦点面（db_ports.rs 或 db_ports/** + state.rs）直连集中度三量（I11a/b/c 的 CI 镜像）。
    #    常量表（DB_DIRECT_*）是单一事实源；判据一律**原始文本**最严口径。
    focus = focus_direct_table(disk, consts.get("db_direct_literal", "easyvibe_db::"))
    total = sum(focus.values())
    mx = max(focus.values()) if focus else 0
    with_direct = len([1 for v in focus.values() if v > 0])
    if total != consts["db_direct_total"]:
        problems.append("I11a 焦点面直连总处数 %d != 登记 %d（守恒律破：搬运丢行/重复/新写直连）"
                        % (total, consts["db_direct_total"]))
    if mx != consts["db_direct_focus_max"]:
        problems.append("I11b 焦点面单文件最大直连数 %d != 登记 %d（双边全等，须同步登记）"
                        % (mx, consts["db_direct_focus_max"]))
    if with_direct != consts["db_direct_focus_files_max"]:
        problems.append("I11c 焦点面有直连的文件数 %d != 登记 %d（落点面漂移）"
                        % (with_direct, consts["db_direct_focus_files_max"]))

    # ⑦ c-arch-16 R6：db_ports/** 单端口域断言（与 arch_guard.rs 的 DB_PORTS_DOMAIN_OWNERSHIP 同源）。
    #    ① 磁盘 `db_ports/*.rs` 与冻结表键集**双向全等**（禁 glob）；
    #    ② 每个文件中出现的 `easyvibe_db::<符号>` 必须落在该文件的允许集内（跨域巨型适配器复发即红）。
    table = consts.get("db_ports_domain_ownership") or {}
    dbp_disk = sorted(
        rel[len("db_ports/"):]
        for rel in disk
        if rel.startswith("db_ports/") and rel.endswith(".rs") and "/" not in rel[len("db_ports/"):]
    )
    dbp_declared = sorted(table)
    if dbp_disk != dbp_declared:
        extra = [f for f in dbp_disk if f not in dbp_declared]
        missing = [f for f in dbp_declared if f not in dbp_disk]
        problems.append(
            "db_ports 文件集漂移（R6 双向全等）extra=%s missing=%s——新增/删除/改名须同步 "
            "DB_PORTS_DOMAIN_OWNERSHIP" % (extra, missing)
        )
    lit = consts.get("db_direct_literal", "easyvibe_db::")
    for rel in sorted(disk):
        rest = rel[len("db_ports/"):] if rel.startswith("db_ports/") else None
        if rest is None or "/" in rest or not rest.endswith(".rs"):
            continue
        allowed = set(table.get(rest, ()))
        for sym in re.findall(re.escape(lit) + r"([A-Za-z_][A-Za-z0-9_]*)", disk[rel]):
            if sym not in allowed:
                problems.append(
                    "db_ports/%s 出现跨域符号 `%s%s`——单端口域断言破（c-arch-16 R6；允许集 %s）"
                    % (rest, lit, sym, sorted(allowed))
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
    report = {
        "ok": not problems,
        "files": len(disk),
        # c-arch-13：焦点面逐文件直连真值表（与 verify_arch_split 同名同义，人工可读）
        "db_direct_by_file": focus_direct_table(disk, consts["db_direct_literal"]),
        "problems": problems,
    }
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if not problems else 1


# ---------------------------------------------------------------------------
# --selfcheck：合成输入（不读 / 不改仓库源码；判据直接施于内存构造的 {rel: 文本}）
# ---------------------------------------------------------------------------

# c-arch-16 R6：合成 db_ports 分布。文件名必须 ⊆ DB_PORTS_DOMAIN_OWNERSHIP（R6 双向全等），
# 符号取本域允许集首项（R6 允许）。分布对齐重登记后的 I11 三量：
# 13 个有直连文件（12 db_ports + state.rs）/ 单文件最大 15 / 总数 91。
_SYNTH_DBP_COUNTS = {
    "agent_slot.rs": 2, "approval.rs": 4, "conversation.rs": 15, "dto.rs": 2, "event.rs": 5,
    "health.rs": 14, "mod.rs": 0, "repo.rs": 3, "session_attribution.rs": 2, "settings.rs": 6,
    "task.rs": 9, "task_engine.rs": 14, "task_engine_approval.rs": 4,
}
_SYNTH_STATE_COUNT = 11


def _db_ports_lines(consts, fname, n):
    """合成 `db_ports/<fname>` 正文：n 处 `easyvibe_db::<本域允许符号>`（R6 允许集内取首符号）。"""
    lit = consts.get("db_direct_literal", "easyvibe_db::")
    if n <= 0:
        return "// zero direct\n"
    allowed = consts["db_ports_domain_ownership"].get(fname) or []
    if not allowed:
        raise GuardParseError("合成输入构造失败：db_ports/%s 允许集为空却需 %d 处直连" % (fname, n))
    return ("let _ = %s%s::x();\n" % (lit, allowed[0])) * n


def _green(consts):
    """全干净合成输入：生产/豁免/路由各就位，文本零 easyvibe_db、零别名再导出。

    c-arch-13：焦点面须满足 I11a/b/c 登记值（13 文件 / 每文件 ≤15 / 总数 91）——构造合成焦点面。
    c-arch-16 R6：db_ports 合成面由 DB_PORTS_DOMAIN_OWNERSHIP 驱动（文件名 ⊆ 登记、符号 ∈ 本域允许集）。
    """
    d = {}
    for p in consts["task_engine_prod"]:
        d[_src_rel(p)] = "// synthetic prod\npub fn prod() {}\n"
    for p in consts["task_engine_test_exempt"]:
        d[_src_rel(p)] = "// synthetic test\n#[test]\nfn t() {}\n"
    d["routes/mod.rs"] = "// synthetic routes\npub mod agent;\n"
    d["lib.rs"] = "// synthetic lib\n"
    for fname in consts["db_ports_domain_ownership"]:
        d["db_ports/%s" % fname] = _db_ports_lines(consts, fname, _SYNTH_DBP_COUNTS.get(fname, 0))
    d["state.rs"] = _focus_line(consts, _SYNTH_STATE_COUNT)
    return d


def _focus_line(consts, n):
    """合成焦点面正文：n 处 `easyvibe_db::` 直连（口径与判据同源；state.rs 非 db_ports，不受 R6 约束）。"""
    lit = consts.get("db_direct_literal", "easyvibe_db::")
    return ("let _ = %sRow::x();\n" % lit) * n


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

    # N3 豁免文件含 easyvibe_db → 必红（c-arch-15：预算已归零，棘轮对任何测试面直连生效）
    d3 = dict(base)
    d3[_src_rel(consts["task_engine_test_exempt"][0])] += "let r = easyvibe_db::SqliteTaskRepository::new(pool);\n"
    cases.append(("N3 豁免文件 easyvibe_db → 必红（预算归零后棘轮生效）", d3, True, consts, None))

    # N4 新增未登记 src/task_exec/tests_new.rs → 必红（清单双向）
    d4 = dict(base)
    d4["task_exec/tests_new.rs"] = "// unregistered\n#[test]\nfn t() {}\n"
    cases.append(("N4 未登记 task_exec 新文件", d4, True, consts, None))

    # N5 守卫常量被改名/删除 → 解析失败必红（fail-closed）
    mutated = re.sub(r"const\s+DB_BANNED\s*:[^;]*;", "", guard_text)
    cases.append(("N5 常量被删除 → 解析失败 fail-closed", None, True, consts, mutated))

    # N6 全干净合成输入 → 必绿
    cases.append(("N6 全干净合成输入", dict(base), False, consts, None))

    # ---- c-arch-13：焦点面直连集中度（I11a/b/c）----
    # N7 单文件 23 处直连（>15）→ 必红（总数守恒 91：conversation +8 / task_engine −8，隔离 I11b）
    d7 = dict(base)
    d7["db_ports/conversation.rs"] = _db_ports_lines(consts, "conversation.rs", 23)
    d7["db_ports/task_engine.rs"] = _db_ports_lines(consts, "task_engine.rs", 6)
    cases.append(("N7 焦点面单文件 23 处直连 → 必红", d7, True, consts, None))

    # N8 13 文件 / 每文件 ≤15 / 总数 91 → 必绿
    cases.append(("N8 焦点面 13 文件/最大 15/总数 91 → 必绿", dict(base), False, consts, None))

    # N9 总数 92 → 必红（守恒律；approval +1，最大/落点不变，隔离 I11a）
    d9 = dict(base)
    d9["db_ports/approval.rs"] = _db_ports_lines(consts, "approval.rs", 5)
    cases.append(("N9 焦点面总数 92 → 必红", d9, True, consts, None))

    # ---- c-arch-16 R6：db_ports 单端口域断言 ----
    # N23 dto.rs 出现跨域符号 TaskRow（计数不变 2，隔离 R6，不误触 I11a/b/c）→ 必红
    d23 = dict(base)
    d23["db_ports/dto.rs"] = ("let _ = %sTaskRow::x();\n" % consts["db_direct_literal"]) * 2
    cases.append(("N23 db_ports/dto.rs 跨域符号 TaskRow → 必红（R6）", d23, True, consts, None))

    # N24 结构化嵌套表行解析失败（dto.rs 行缺 `&[…]`）→ 解析失败必红（fail-closed）
    mutated_tbl = re.sub(
        r'\("dto\.rs"\s*,\s*&\["ConversationMessageRow"\]\)',
        '("dto.rs", "ConversationMessageRow")',
        guard_text,
    )
    cases.append(("N24 结构化表行解析失败 → 必红（fail-closed）", None, True, consts, mutated_tbl))

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
    print("N1–N24 %s" % ("全 PASS" if failed == 0 else "%d 项 FAIL" % failed))
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
