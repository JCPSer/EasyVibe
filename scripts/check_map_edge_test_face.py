#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""c-arch-15 / R4+R7：edging「测试面引用不构成依赖边」口径的纯 python3 CI 载体。

为什么需要它：`.github/workflows/asset-guard.yml` 是秒级轻量流水线（纯 python3，不跑 cargo）。
本仓 CI 无 `.easyvibe/` live map（被 gitignore），故本守卫**只依赖受版本控制文件**：
`scripts/map_edge_policy.json`（测试面冻结表/粗筛表）、`scripts/tests/fixtures/map_post_split.json`
（受版本控制地图固件）、`run/easyvibe_map_cli.py::product_files()` 与源码文件本身。

口径（与 `scripts/map_policy.py` 同源，零副本）：
  归属（枚举/coverage）与出边（edging 证据）**正交**——测试面文件仍被 product_files() 枚举、
  仍被模块 glob 归属，但按 `map_policy.is_test_face` 谓词**不参与出边证据提取**，故不构成依赖边。

--check（json 报告，ok:false ⇒ 退出 1）：
  G0 policy 装载：map_policy.test_face_files/test_face_globs 可读且非空；缺失/空 = fail-closed 红。
  G1 冻结表自洽：每条冻结路径在磁盘存在；冻结集合 == 粗筛命中集合（product_files() ∩ 粗筛 glob）。
  G2 物理事实：task_exec.rs 与 src/task_exec/**/*.rs **去注释后**（字符串内容保留）零字面量
                `easyvibe_db`——与 c-arch-7 守卫互补，给 CI 一条独立读法。
  G3 口径生效（对 fixture）：a) 每个测试面文件仍命中 policy.modules 的某个 files glob（归属保留）；
                b) 每条 fixture 边的 from 模块存在非测试面文件（供证非空，防口径误伤）。
  G4 本命题闭环：① fixture 存储边集**不得**存在 task-engine → persistence 边；② 以 fixture 的
                模块宇宙 + 同一 `extract_edges` 谓词重推的边集亦不得再推出该边（证明该边已无
                任何非测试面供证 ⇒ 在「测试面引用不构成依赖边」口径下消失）。任一含即红。

--selfcheck（合成输入，不读仓库源码；逐条 PASS/FAIL + 汇总）：
  S1 src 内夹具（tests_*.rs）→ is_test_face 命中且不产边；抹掉谓词则产边（红绿差异只来自谓词）。
  S2 crate 级 tests/** → 命中且不产边；抹掉谓词则产边。
  S3 前端 __tests__/*.test.ts → 命中且不产边；抹掉谓词则产边。
  S4 生产文件（state.rs → easyvibe_db::）→ 产边（谓词不误伤）。
  S5 抹掉 policy.gates.test_face_files ⇒ test_face_files/is_test_face 抛 PolicyMissing（fail-closed 必红）。

退出码：--check 绿=0 / 红=1；--selfcheck 全 PASS=0 / 任一 FAIL=1。
"""

import copy
import json
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCRIPTS = os.path.join(REPO, "scripts")
RUN = os.path.join(REPO, "run")
FIXTURE = os.path.join(SCRIPTS, "tests", "fixtures", "map_post_split.json")
TASK_EXEC_MAIN = os.path.join(REPO, "easyvibe-backend", "crates", "easyvibe-app", "src", "task_exec.rs")
TASK_EXEC_DIR = os.path.join(REPO, "easyvibe-backend", "crates", "easyvibe-app", "src", "task_exec")

# run/easyvibe_map_cli.py 在 import 期以 REPO_ROOT（默认 cwd）定位仓库；显式钉死为脚本所在仓。
os.environ.setdefault("REPO_ROOT", REPO)
for _p in (SCRIPTS, RUN):
    if _p not in sys.path:
        sys.path.insert(0, _p)
import map_policy  # noqa: E402
import easyvibe_map_cli  # noqa: E402

# crate → 模块映射（与 .easyvibe/map/_tools/scan_edges.py 同源）。
CRATE2MOD = {
    "easyvibe_common": "contract-foundation", "easyvibe_api_types": "contract-foundation",
    "easyvibe_db": "persistence", "easyvibe_map": "map-domain",
    "easyvibe_ai_agent": "agent-runtime", "easyvibe_session": "agent-runtime",
    "easyvibe_event_bus": "event-bus", "easyvibe_git": "easyvibe-git",
    "easyvibe_pipeline": "easyvibe-pipeline", "easyvibe_app": "server-api",
}

DB_LITERAL = "easyvibe_db"
TS_SPEC = re.compile(r"""(?:from\s+|import\s*\(\s*|import\s+|require\(\s*)(['"])([^'"]+)\1""")


def strip_rust_comments(text):
    """剔除 Rust `//` 行注释与 `/* */` 块注释；**字符串字面量内容保留**（不误删、不误判）。"""
    out = []
    i, n = 0, len(text)
    while i < n:
        if text[i] == '"':
            out.append(text[i]); i += 1
            while i < n:
                out.append(text[i])
                if text[i] == "\\":
                    i += 1
                    if i < n:
                        out.append(text[i])
                elif text[i] == '"':
                    i += 1
                    break
                i += 1
            continue
        if text[i] == "'":
            # 区分字符字面量（'x' / '\n' / '"'）与生命周期（'a）：前者成对跳过，后者放行。
            j = i + 1
            if j < n and text[j] == "\\":
                j += 2
                while j < n and text[j] != "'":
                    j += 1
                if j < n:
                    out.append(text[i:j + 1]); i = j + 1; continue
            elif j + 1 < n and text[j + 1] == "'":
                out.append(text[i:i + 3]); i += 3; continue
            out.append(text[i]); i += 1
            continue
        if text[i:i + 2] == "//":
            while i < n and text[i] != "\n":
                i += 1
            continue
        if text[i:i + 2] == "/*":
            i += 2
            while i < n and text[i:i + 2] != "*/":
                i += 1
            i += 2
            continue
        out.append(text[i]); i += 1
    return "".join(out)


def resolve_ts(spec, rel, owners):
    """把 TS 模块说明符解析为 owners 中的实际仓库文件（`@/` 与相对路径）。"""
    if spec.startswith("@/"):
        base = "easyvibe-renderer/src/" + spec[2:]
    elif spec.startswith("."):
        base = os.path.normpath(os.path.join(os.path.dirname(rel), spec)).replace(os.sep, "/")
    else:
        return None
    for cand in (base, base + ".ts", base + ".tsx", base + ".js", base + ".mjs",
                 base + "/index.ts", base + "/index.tsx"):
        if cand in owners:
            return cand
    return None


def extract_edges(files, owners, policy):
    """极小 edging：Rust 去注释后 `\\b(easyvibe_[a-z_]+)::` → CRATE2MOD；TS `from '@/…'` → owner 模块。

    `files` = {相对路径: 文本}（内存构造或磁盘读入）；`owners` = {相对路径: 模块 id}。
    **先经 map_policy.is_test_face 过滤**：测试面文件一律不产出边（R4 谓词）。
    返回 {(from, to)}。S1–S4 与 G4 共用本函数 —— 红绿差异只来自谓词。
    """
    edges = set()
    for rel in sorted(files):
        if map_policy.is_test_face(policy, rel):
            continue
        src = owners.get(rel)
        if not src:
            continue
        text = files[rel]
        ext = os.path.splitext(rel)[1].lower()
        if ext == ".rs":
            code = strip_rust_comments(text)
            for crate in set(re.findall(r"\b(easyvibe_[a-z_]+)::", code)):
                tgt = CRATE2MOD.get(crate)
                if tgt and tgt != src:
                    edges.add((src, tgt))
        elif ext in (".ts", ".tsx", ".js", ".mjs"):
            for _q, spec in TS_SPEC.findall(text):
                tgt_file = resolve_ts(spec, rel, owners)
                if tgt_file is None:
                    continue
                tgt = owners.get(tgt_file)
                if tgt and tgt != src:
                    edges.add((src, tgt))
    return edges


def owners_of(modules, files):
    """按给定模块表（顺序即优先级）把文件归属到模块；未命中不登记。"""
    out = {}
    for rel in files:
        for m in modules:
            if any(map_policy.glob_match(rel, g) for g in m.get("files", [])):
                out[rel] = m["id"]
                break
    return out


def read_texts(rels):
    out = {}
    for rel in rels:
        try:
            with open(os.path.join(REPO, rel), encoding="utf-8", errors="replace") as f:
                out[rel] = f.read()
        except OSError:
            continue
    return out


def load_fixture():
    with open(FIXTURE, encoding="utf-8") as f:
        return json.load(f)


# ---------------------------------------------------------------------------
# --check
# ---------------------------------------------------------------------------

def cmd_check():
    # ---- G0 policy 装载（fail-closed）----
    try:
        policy = map_policy.load_policy(REPO)
        tff = map_policy.test_face_files(policy)
        tfg = map_policy.test_face_globs(policy)
    except Exception as e:  # noqa: BLE001 —— 任何装载异常都必须 fail-closed
        print(json.dumps({"ok": False, "stage": "G0-policy",
                          "problems": ["G0 测试面字段不可读（fail-closed）: %s" % e]},
                         ensure_ascii=False, indent=2))
        return 1
    if not tff or not tfg:
        print(json.dumps({"ok": False, "stage": "G0-policy",
                          "problems": ["G0 gates.test_face_files / test_face_globs 为空（fail-closed）"]},
                         ensure_ascii=False, indent=2))
        return 1

    problems = []
    detail = {"G0": {"test_face_files": len(tff), "test_face_globs": len(tfg)}}
    files_all = easyvibe_map_cli.product_files()

    # ---- G1 冻结表自洽 ----
    missing = sorted(p for p in tff if not os.path.isfile(os.path.join(REPO, p)))
    if missing:
        problems.append("G1 冻结表路径在磁盘不存在: %s" % missing)
    coarse = sorted(f for f in files_all if any(map_policy.glob_match(f, g) for g in tfg))
    frozen = sorted(tff)
    table_only = [f for f in frozen if f not in set(coarse)]
    coarse_only = [f for f in coarse if f not in set(frozen)]
    if table_only or coarse_only:
        problems.append("G1 冻结集合 != 粗筛命中集合: 表内多余=%s 粗筛未见=%s" % (table_only, coarse_only))
    detail["G1"] = {"frozen": len(frozen), "coarse": len(coarse)}

    # ---- G2 物理事实（去注释后零字面量）----
    rs = [TASK_EXEC_MAIN]
    for root, _dirs, names in os.walk(TASK_EXEC_DIR):
        for nm in names:
            if nm.endswith(".rs"):
                rs.append(os.path.join(root, nm))
    hits = {}
    for full in rs:
        try:
            with open(full, encoding="utf-8", errors="replace") as f:
                code = strip_rust_comments(f.read())
        except OSError:
            problems.append("G2 声明的 task_exec 文件缺失: %s" % full)
            continue
        n = code.count(DB_LITERAL)
        if n:
            hits[os.path.relpath(full, REPO).replace(os.sep, "/")] = n
    if hits:
        total = sum(hits.values())
        problems.append("G2 task_exec 去注释后仍有 `%s` 字面量 %d 处: %s"
                        % (DB_LITERAL, total, hits))
    detail["G2"] = {"files": len(rs), "hits": hits}

    # ---- G3 口径生效（对 fixture）----
    fx = load_fixture()
    fx_modules = fx.get("modules", [])
    pol_modules = [{"id": mid, "files": mv["files"]}
                   for mid, mv in map_policy.modules_map(policy).items()]

    unowned = [f for f in frozen
               if not any(map_policy.glob_match(f, g) for m in pol_modules for g in m["files"])]
    if unowned:
        problems.append("G3a 测试面文件未被任何模块 glob 归属（coverage 会掉）: %s" % unowned)

    fx_owners = owners_of(fx_modules, files_all)
    test_face_set = set(frozen)
    empty_supply = []
    for e in fx.get("edges", []):
        frm = e.get("from")
        owned = [f for f, m in fx_owners.items() if m == frm]
        if not [f for f in owned if f not in test_face_set]:
            empty_supply.append((e.get("id"), frm, e.get("to")))
    if empty_supply:
        problems.append("G3b 边的 from 模块无任何非测试面文件（供证被误伤）: %s" % empty_supply)
    detail["G3"] = {"test_face_files": len(frozen), "fixture_edges": len(fx.get("edges", [])),
                    "unowned": unowned, "empty_supply": empty_supply}

    # ---- G4 本命题闭环（fixture 图不得存在 task-engine → persistence）----
    # 双读法：① fixture 存储边集（受版本控制固件的事实）；② 以同一 extract_edges 谓词从
    # fixture 模块宇宙重推的边集（证明「测试面引用不构成依赖边」口径下该边消失）。任一含即红。
    derived = extract_edges(read_texts(fx_owners.keys()), fx_owners, policy)
    stored = {(e.get("from"), e.get("to")) for e in fx.get("edges", [])}
    prop = ("task-engine", "persistence")
    in_stored = prop in stored
    in_derived = prop in derived
    if in_stored:
        problems.append("G4 fixture 图仍表达 %s → %s 边（本命题未闭环）" % prop)
    if in_derived:
        problems.append("G4 谓词闭合后仍推出 %s → %s 边（生产文件复现 easyvibe_db 引用？）" % prop)
    detail["G4"] = {"derived_edges": len(derived), "fixture_edges": len(stored),
                    "task_engine_persistence_in_fixture": in_stored,
                    "task_engine_persistence_in_derived": in_derived}

    report = {"ok": not problems, "product_files": len(files_all), "detail": detail,
              "problems": problems}
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if not problems else 1


# ---------------------------------------------------------------------------
# --selfcheck：合成输入（不读仓库源码；红绿差异只来自谓词）
# ---------------------------------------------------------------------------

def _base_policy():
    return map_policy.load_policy(REPO)


def _predicates_off(policy):
    """同代 policy 的「谓词关闭」副本：两表清空 ⇒ is_test_face 恒 False（红绿对照用）。"""
    p = copy.deepcopy(policy)
    p["gates"]["test_face_files"] = []
    p["gates"]["test_face_globs"] = []
    return p


def _run_case(name, files, owners, expect_hit, expect_edge, results):
    policy = _base_policy()
    off = _predicates_off(policy)
    rels = list(files)
    hit = all(map_policy.is_test_face(policy, r) for r in rels)
    on_edges = extract_edges(files, owners, policy)
    off_edges = extract_edges(files, owners, off)
    edge = bool(on_edges)
    ok = (hit == expect_hit) and (edge == expect_edge)
    if expect_hit and expect_edge is False:
        # 负例：红绿差异必须来自谓词 —— 关闭谓词后应当产边
        ok = ok and bool(off_edges)
    detail = "is_test_face=%s 产边=%s 关闭谓词后产边=%s edges=%s" % (
        hit, sorted(on_edges), sorted(off_edges), sorted(on_edges))
    results.append((name, ok, detail))


def cmd_selfcheck():
    results = []
    policy = _base_policy()

    # S1 src 内夹具（tests_*.rs）→ 命中且不产边；关谓词则产边
    f1 = {"easyvibe-backend/crates/easyvibe-app/src/task_exec/tests_x.rs":
          "use easyvibe_db::TaskRepository;\n"}
    _run_case("S1 src 内夹具 tests_x.rs → 命中且不产边（关谓词则产边）",
              f1, {list(f1)[0]: "task-engine"}, True, False, results)

    # S2 crate 级 tests/** → 命中且不产边；关谓词则产边
    f2 = {"easyvibe-backend/crates/easyvibe-app/tests/y.rs": "easyvibe_db::TaskRow\n"}
    _run_case("S2 crate 级 tests/y.rs → 命中且不产边（关谓词则产边）",
              f2, {list(f2)[0]: "server-api"}, True, False, results)

    # S3 前端 __tests__/*.test.ts → 命中且不产边；关谓词则产边
    f3 = {"easyvibe-renderer/src/lib/__tests__/x.test.ts":
          "import a from '@/components/shell/App'\n"}
    o3 = {list(f3)[0]: "console-ui",
          "easyvibe-renderer/src/components/shell/App.tsx": "ui-kit"}  # 合成归属：仅作红绿对照
    _run_case("S3 前端 __tests__/x.test.ts → 命中且不产边（关谓词则产边）",
              f3, o3, True, False, results)

    # S4 生产文件 → 产边（谓词不误伤）
    f4 = {"easyvibe-backend/crates/easyvibe-app/src/state.rs":
          "let r = easyvibe_db::SqliteTaskRepository::new(pool);\n"}
    _run_case("S4 生产文件 state.rs → 产边（谓词不误伤）",
              f4, {list(f4)[0]: "server-api"}, False, True, results)

    # S5 fail-closed：抹掉 gates.test_face_files ⇒ 抛 PolicyMissing
    p5 = copy.deepcopy(policy)
    del p5["gates"]["test_face_files"]
    try:
        map_policy.test_face_files(p5)
        ok5, d5 = False, "未抛错（fail-closed 失效）"
    except map_policy.PolicyMissing as e:
        try:
            map_policy.is_test_face(p5, "easyvibe-backend/crates/easyvibe-app/src/task_exec/tests_x.rs")
            ok5, d5 = False, "is_test_face 未抛错（fail-closed 失效）"
        except map_policy.PolicyMissing:
            ok5, d5 = True, "test_face_files/is_test_face 均抛 PolicyMissing: %s" % e.dotted_key
    results.append(("S5 抹掉 gates.test_face_files → PolicyMissing（fail-closed）", ok5, d5))

    failed = 0
    for name, ok, detail in results:
        failed += 0 if ok else 1
        print("%s %s  %s" % ("PASS" if ok else "FAIL", name, detail))
    print("S1–S5 %s" % ("全 PASS" if failed == 0 else "%d 项 FAIL" % failed))
    return 0 if failed == 0 else 1


def main():
    arg = sys.argv[1] if len(sys.argv) > 1 else "--check"
    if arg == "--selfcheck":
        return cmd_selfcheck()
    if arg == "--check":
        return cmd_check()
    print("用法: check_map_edge_test_face.py [--check|--selfcheck]")
    return 2


if __name__ == "__main__":
    sys.exit(main())
