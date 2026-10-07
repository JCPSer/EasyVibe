#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""c-arch-11 / c-renderer-runtime-3 收口：i18n **取词契约 / 词表数据**分离的架构守卫。

背景：自研 i18n 的词典曾以单文件承载全部 1423 key（3008 行，占 renderer-runtime 77% LOC），
与 74 行取词契约同驻，随每批翻译线性增长——渲染地基退化为数据容器，且并行翻译批次必然同文件冲突。
本守卫把「契约不随翻译膨胀 + 词表按域分片 + 无孤儿/重复 key」落成 CI 可见、秒级、fail-closed 的判据。

  --check [--map PATH]  读磁盘真实源码（--map 仅为对齐既有守卫范式，本守卫不依赖地图）。
  --selfcheck           合成输入 N0–N7（不读磁盘、不改仓库），逐条自证负例必红 / 正例必绿。

布局约定（甲案）：契约入口 = easyvibe-renderer/src/runtime/i18n/index.ts；
                  词表分片   = easyvibe-renderer/src/runtime/i18n/dict/<shard>.ts；
                  旧单文件   = easyvibe-renderer/src/runtime/i18n.ts 必须**不存在**（同形遮蔽 fail-closed）。

判据：
  P0 布局 fail-closed：入口存在；旧单文件不存在；dict 目录存在且 >=1 分片。
  P1 契约无数据：入口行数 <= ENTRY_MAX 且入口内「点分 key 词条字面量」命中数 == 0。
  P2 分片体量：每个分片行数 <= SHARD_MAX。
  P3 片内对齐：分片的 zh / en key 集合必须相等；zh 的命名空间集合 ⊆ 该片登记注释声明的命名空间集合。
  P4 全局划分：① 跨片 key 无重复；② 每个命名空间恰被一片声明且确有 key（互斥、无孤儿）；
               ③ 入口 `./dict/<stem>` 聚合的 stem 集合 == dict 目录 .ts 文件 stem 集合（双向全等）。
  P5 契约不收窄：入口 `t` 签名仍为 `key: string`（动态拼接 key 不得被静态字面量联合挡住）。

分片登记面：每分片首行 `// i18n-shard: <ns>[,<ns>...]`（纯注释、零运行时开销），守卫据此判归属。

退出码：--check 绿=0 / 红=1；--selfcheck 全 PASS=0 / 任一 FAIL=1。
本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""

import argparse
import json
import os
import re
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

ENTRY_REL = "easyvibe-renderer/src/runtime/i18n/index.ts"
OLD_REL = "easyvibe-renderer/src/runtime/i18n.ts"
DICT_REL = "easyvibe-renderer/src/runtime/i18n/dict"

ENTRY_MAX = 200      # 契约入口行数上限：入口只含逻辑 + 聚合 import，恒定不随翻译增长。
SHARD_MAX = 1000     # 单分片行数上限：沿用地图「单文件 >1000 行 = complexity high」口径。

KEY_LINE_RE = re.compile(r"^\s*'([^']+)'\s*:\s")
KEY_LITERAL_RE = re.compile(r"^\s*'[^']*\.[^']*'\s*:")
SHARD_HEADER_RE = re.compile(r"^//\s*i18n-shard:\s*(.+?)\s*$")
IMPORT_DICT_RE = re.compile(r"""from\s+['"]\./dict/([A-Za-z0-9_-]+)['"]""")
T_SIG_RE = re.compile(r"function\s+t\s*\(\s*key\s*:\s*string")

ZH_OPEN_RE = re.compile(r"^export\s+const\s+zh\s*=\s*\{\s*$")
ZH_CLOSE_RE = re.compile(r"^\}\s*as\s*const\s*$")
EN_OPEN_RE = re.compile(r"^export\s+const\s+en\s*:\s*Record<keyof\s+typeof\s+zh,\s*string>\s*=\s*\{\s*$")
EN_CLOSE_RE = re.compile(r"^\}\s*$")


def loc(text):
    """行数（与 wc -l 同义：以换行切分，末行计入）。"""
    return len(text.splitlines())


def extract_block_keys(text, open_re, close_re):
    """抽取 open_re..close_re 之间所有 `<key>:` 形态的 key；返回 (keys, found)。"""
    lines = text.splitlines()
    start = None
    for i, ln in enumerate(lines):
        if open_re.match(ln):
            start = i + 1
            break
    if start is None:
        return [], False
    keys = []
    for ln in lines[start:]:
        if close_re.match(ln):
            return keys, True
        m = KEY_LINE_RE.match(ln)
        if m:
            keys.append(m.group(1))
    return keys, False


def parse_shard(text):
    """解析一个分片：返回 (declared_ns, zh_keys, en_keys, zh_found, en_found)。"""
    declared = []
    first = text.splitlines()[0] if text.strip() else ""
    m = SHARD_HEADER_RE.match(first)
    if m:
        declared = [s.strip() for s in m.group(1).split(",") if s.strip()]
    zh_keys, zh_ok = extract_block_keys(text, ZH_OPEN_RE, ZH_CLOSE_RE)
    en_keys, en_ok = extract_block_keys(text, EN_OPEN_RE, EN_CLOSE_RE)
    return declared, zh_keys, en_keys, zh_ok, en_ok


def ns_of(key):
    return key.split(".", 1)[0] if "." in key else ""


def evaluate(read, exists, shard_stems, entry_rel=ENTRY_REL, old_rel=OLD_REL):
    """施加 P0–P5。read(rel)->str|None；exists(rel)->bool；shard_stems=dict 下 .ts 文件名（去扩展）。"""
    problems = []
    checks = {}

    def fail(code, msg):
        problems.append("%s %s" % (code, msg))

    # ---------------------------------------------------------------- P0 布局
    entry = read(entry_rel)
    p0 = False
    if entry is None:
        fail("P0", "契约入口不存在（fail-closed，不静默跳过）: %s" % entry_rel)
        p0 = True
    if exists(old_rel):
        fail("P0", "旧单文件仍存在（同形遮蔽：文件解析优先于目录 index，分片永不生效）: %s" % old_rel)
        p0 = True
    if not shard_stems:
        fail("P0", "分片目录不存在或无 .ts 分片（fail-closed）: %s" % DICT_REL)
        p0 = True
    checks["P0"] = "FAIL" if p0 else "PASS"

    # ---------------------------------------------------------------- P1 契约无数据
    p1 = False
    if entry is not None:
        n = loc(entry)
        if n > ENTRY_MAX:
            fail("P1", "契约入口 %d 行 > 上限 %d（契约被数据拖着膨胀）" % (n, ENTRY_MAX))
            p1 = True
        literals = [ln for ln in entry.splitlines() if KEY_LITERAL_RE.match(ln)]
        if literals:
            fail("P1", "契约入口内出现 %d 条词条字面量（数据未分离），首条: %s"
                 % (len(literals), literals[0].strip()))
            p1 = True
    checks["P1"] = "FAIL" if p1 else "PASS"

    # ---------------------------------------------------------------- P2 分片体量
    p2 = False
    shard_keys = {}   # stem -> zh keys
    for stem in shard_stems:
        rel = "%s/%s.ts" % (DICT_REL, stem)
        text = read(rel)
        if text is None:
            fail("P2", "分片文件不可读（fail-closed）: %s" % rel)
            p2 = True
            continue
        n = loc(text)
        if n > SHARD_MAX:
            fail("P2", "分片 %s %d 行 > 上限 %d" % (rel, n, SHARD_MAX))
            p2 = True
    checks["P2"] = "FAIL" if p2 else "PASS"

    # ---------------------------------------------------------------- P3 片内对齐
    p3 = False
    declared_by = {}   # ns -> [stem...]
    for stem in shard_stems:
        rel = "%s/%s.ts" % (DICT_REL, stem)
        text = read(rel)
        if text is None:
            continue
        declared, zh_keys, en_keys, zh_ok, en_ok = parse_shard(text)
        if not zh_ok or not en_ok:
            fail("P3", "分片 %s 缺少 zh/en 词表块" % rel)
            p3 = True
            continue
        shard_keys[stem] = zh_keys
        if len(zh_keys) != len(set(zh_keys)):
            fail("P3", "分片 %s 内 zh key 重复" % rel)
            p3 = True
        if sorted(zh_keys) != sorted(en_keys):
            missing = sorted(set(zh_keys) - set(en_keys))
            extra = sorted(set(en_keys) - set(zh_keys))
            fail("P3", "分片 %s zh/en key 不齐（zh 缺 %d / en 多 %d），例: %s"
                 % (rel, len(missing), len(extra), (missing + extra)[:3]))
            p3 = True
        ns_set = {ns_of(k) for k in zh_keys}
        undeclared = sorted(ns_set - set(declared))
        if undeclared:
            fail("P3", "分片 %s 含未登记命名空间 %s（登记面: %s）" % (rel, undeclared, declared))
            p3 = True
        for ns in declared:
            declared_by.setdefault(ns, []).append(stem)
    checks["P3"] = "FAIL" if p3 else "PASS"

    # ---------------------------------------------------------------- P4 全局划分
    p4 = False
    all_keys = []
    for stem in shard_stems:
        all_keys.extend(shard_keys.get(stem, []))
    if len(all_keys) != len(set(all_keys)):
        dupes = sorted({k for k in all_keys if all_keys.count(k) > 1})
        fail("P4", "跨片 key 重复 %d 条，例: %s" % (len(dupes), dupes[:3]))
        p4 = True
    multi = sorted(ns for ns, stems in declared_by.items() if len(stems) > 1)
    if multi:
        fail("P4", "命名空间被多片声明（归属不互斥）: %s" % multi)
        p4 = True
    declared_all = set(declared_by)
    empty_decl = sorted(ns for ns in declared_all if not any(ns_of(k) == ns for k in all_keys))
    if empty_decl:
        fail("P4", "登记了命名空间却无对应 key（孤儿登记）: %s" % empty_decl)
        p4 = True
    if entry is not None:
        entry_stems = sorted(set(IMPORT_DICT_RE.findall(entry)))
        disk_stems = sorted(set(shard_stems))
        if entry_stems != disk_stems:
            fail("P4", "入口聚合的 ./dict/<stem> 集合 != dict 目录分片集合：入口 %s vs 磁盘 %s"
                 % (entry_stems, disk_stems))
            p4 = True
    checks["P4"] = "FAIL" if p4 else "PASS"

    # ---------------------------------------------------------------- P5 契约不收窄
    p5 = False
    if entry is not None and not T_SIG_RE.search(entry):
        fail("P5", "入口 t() 签名未保留 `key: string`（动态拼接 key 会被静态联合挡住）")
        p5 = True
    checks["P5"] = "FAIL" if p5 else "PASS"

    return problems, checks


# ------------------------------------------------------------------ --check
def cmd_check(map_path=None):
    def read(rel):
        p = os.path.join(REPO, rel)
        if not os.path.isfile(p):
            return None
        with open(p, encoding="utf-8") as fh:
            return fh.read()

    def exists(rel):
        return os.path.exists(os.path.join(REPO, rel))

    stems = []
    d = os.path.join(REPO, DICT_REL)
    if os.path.isdir(d):
        stems = sorted(fn[:-3] for fn in os.listdir(d)
                       if fn.endswith(".ts") and not fn.endswith(".d.ts"))

    problems, checks = evaluate(read, exists, stems)
    report = {
        "ok": not problems,
        "entry": ENTRY_REL,
        "old_single_file_absent": not exists(OLD_REL),
        "shards": stems,
        "row_limits": {"entry_max": ENTRY_MAX, "shard_max": SHARD_MAX},
        "checks": checks,
        "problems": problems,
    }
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if not problems else 1


# ------------------------------------------------------------------ --selfcheck
def _entry(imports, extra=""):
    body = "\n".join("import { zh as %s, en as %s } from './dict/%s'" % (s, s, s) for s in imports)
    return (
        "// 合成契约入口（selfcheck）\n"
        + body + "\n"
        "export type Lang = 'zh' | 'en'\n"
        + extra +
        "export const zhDict = Object.assign({}, %s)\n" % ", ".join(imports) +
        "export const enDict = Object.assign({}, %s)\n" % ", ".join(imports) +
        "export function t(key: string, vars?: Record<string, string | number>): string { return String(key) }\n"
    )


def _shard(ns_list, keys):
    header = "// i18n-shard: %s\n" % ",".join(ns_list)
    zh = "\n".join("  '%s': '甲'," % k for k in keys)
    en = "\n".join("  '%s': 'A'," % k for k in keys)
    return "%sexport const zh = {\n%s\n} as const\n\nexport const en: Record<keyof typeof zh, string> = {\n%s\n}\n" % (
        header, zh, en)


def _build(shards, entry_text=None, old=False):
    """shards: {stem: content}；返回 (read, exists, stems)。"""
    files = {}
    for stem, content in shards.items():
        files["%s/%s.ts" % (DICT_REL, stem)] = content
    files[ENTRY_REL] = entry_text if entry_text is not None else _entry(sorted(shards))
    if old:
        files[OLD_REL] = "// 旧单文件（同形遮蔽负例）\nexport const zh = {}\n"
    return (lambda rel: files.get(rel),
            lambda rel: rel in files,
            sorted(shards))


def _with(files, entry_override=None, old=False, extra_shards=None):
    files = dict(files)
    stems = sorted({_stem(p) for p in files if p.startswith(DICT_REL + "/")} | set(extra_shards or []))
    if entry_override is not None:
        files[ENTRY_REL] = entry_override
    elif ENTRY_REL not in files:
        files[ENTRY_REL] = _entry(stems)
    if old:
        files[OLD_REL] = "// 旧单文件\n"
    return (lambda rel: files.get(rel), lambda rel: rel in files, stems)


def _stem(path):
    return os.path.basename(path)[:-3]


def cmd_selfcheck():
    a_keys = ["alpha.one", "alpha.two"]
    b_keys = ["beta.one"]
    base = {
        "%s/a.ts" % DICT_REL: _shard(["alpha"], a_keys),
        "%s/b.ts" % DICT_REL: _shard(["beta"], b_keys),
    }
    cases = []  # (name, (read, exists, stems), expect_red, want_check)

    def green_files():
        return dict(base)

    def add(name, files, expect_red, want_check=None, entry_override=None, old=False, extra_shards=None):
        cases.append((name, _with(files, entry_override, old, extra_shards), expect_red, want_check))

    # N0 正例
    add("N0 合规模拟布局（2 片 + 契约入口）", green_files(), False)

    # N1 入口塞回词条字面量 → P1
    add("N1 入口内塞回一条词条字面量 → P1 必红", green_files(), True, "P1",
        entry_override=_entry(["a", "b"], extra="  'alpha.one': '甲',\n"))

    # N2 入口超 200 行 → P1
    add("N2 入口行数超上限 → P1 必红", green_files(), True, "P1",
        entry_override=_entry(["a", "b"], extra="// pad\n" * 260))

    # N3 某分片 > 1000 行 → P2
    big_keys = ["big.k%04d" % i for i in range(1001)]
    big = dict(green_files())
    big["%s/big.ts" % DICT_REL] = _shard(["big"], big_keys)
    add("N3 某分片超 1000 行 → P2 必红", big, True, "P2")
    # 该案同时增加 big 分片 → 入口需聚合；单独给入口
    cases[-1] = ("N3 某分片超 1000 行 → P2 必红",
                 _with(big, entry_override=_entry(["a", "b", "big"])), True, "P2")

    # N4 同一 key 出现在两片 → P4
    dup = dict(green_files())
    dup["%s/b.ts" % DICT_REL] = _shard(["beta"], ["alpha.one", "beta.one"])
    add("N4 同 key 出现在两片 → P4 必红", dup, True, "P4")

    # N5 某片 en 少一个 key → P3
    half = dict(green_files())
    half["%s/b.ts" % DICT_REL] = (
        "// i18n-shard: beta\n"
        "export const zh = {\n  'beta.one': '甲',\n} as const\n\n"
        "export const en: Record<keyof typeof zh, string> = {\n}\n")
    add("N5 某片 en 少一个 key → P3 必红", half, True, "P3")

    # N6 旧单文件与目录并存 → P0
    add("N6 旧单文件残留（同形遮蔽）→ P0 必红", green_files(), True, "P0", old=True)

    # N7 新增已登记分片并移入部分 key、入口同步聚合 → 绿（防过严）
    n7 = dict(green_files())
    n7["%s/a.ts" % DICT_REL] = _shard(["alpha"], ["alpha.one"])
    n7["%s/c.ts" % DICT_REL] = _shard(["gamma"], ["alpha.two", "gamma.one"])  # 会因 ns 不符红
    # 正确形态：把 alpha.two 留给 alpha，另立 gamma 只含自身 key
    n7["%s/a.ts" % DICT_REL] = _shard(["alpha"], ["alpha.one", "alpha.two"])
    n7["%s/c.ts" % DICT_REL] = _shard(["gamma"], ["gamma.one"])
    add("N7 新增已登记分片并同步聚合 → 绿（防过严）", n7, False)

    failed = 0
    for name, (read, exists, stems), expect_red, want_check in cases:
        problems, checks = evaluate(read, exists, stems)
        red = bool(problems)
        ok = red == expect_red
        if ok and want_check:
            ok = checks.get(want_check) == "FAIL"
        failed += 0 if ok else 1
        detail = ("%s=%s" % (want_check, checks.get(want_check))) if want_check else (
            problems[0] if problems else "无问题")
        print("%s %s  %s" % ("PASS" if ok else "FAIL", name, detail))

    print("N0–N7 %s" % ("全 PASS" if failed == 0 else "%d 项 FAIL" % failed))
    return 0 if failed == 0 else 1


def main():
    ap = argparse.ArgumentParser(description="i18n 取词契约 / 词表分片分离架构守卫（c-arch-11）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--map", default=None, help="仅为对齐既有守卫范式；本守卫不依赖地图")
    args = ap.parse_args()
    if args.selfcheck:
        return cmd_selfcheck()
    return cmd_check(args.map)


if __name__ == "__main__":
    sys.exit(main())
