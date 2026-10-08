#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""叙事（prose）同代判据（c-arch-17 / R3+R4）。

命题：prose（`health.review_note`）此前**没有任何机器判据**，且只住在 gitignored 的 `.easyvibe/map/**`
⇒ 上一代 note 写出「c-server-api-3 未闭环」而无人报警。本文件把「叙事漂移」变成命令级红绿：

  I14 prose 摘要闸门（照 `edges.sha256` 既有范式）：
      live map 的 `health.review_note`（顶层 + 逐格）经规范化后的摘要，
      必须逐字等于 `policy.gates.notes_sha256` / `gates.notes_sha256_by_module`。
      投影字段与规范化口径登记在 `policy.gates.notes_projection`（唯一真值）。
  J  退役 concern 不得在 prose 中复现为「未闭环」：
      `policy.gates.retired_concerns` 中的 id 若与其**同分句**出现未闭环/待处理语义，即红。
      引号内的引述（「…」『…』"…"）先剥离——本轮 note 正是以「上轮断言『c-X 未闭环』与代码矛盾」的
      形式**订正**漂移，属正当表述，不得误伤。
  K  计数型 prose 规则（R2）：note 不得写**滑动窗口量**的精确值（churn 触点计数、全库排名、
      产品文件数、文件行数）——真值 = `policy.gates.prose_quantity_rules`（families 正则表）。
      离散档位（high/medium/low）、policy 键名引用、日期与 id 不受限；引号内的**引述**先剥离
      （复用 J 的同一实现），故「上轮曾写『255 次』，本轮订正」不误伤。
  L  叙事基准同代（R6）：prose 中的 `HEAD <sha>` 必须等于 `meta.provenance.head_short`
      （生成期注入，不再手写）；`meta.provenance` 缺失或键不全即 fail-closed 红。
  INV-5 叙事单源：`run/**` 不得出现对 review_note 的**字面量赋值**——叙事唯一写入方 = live map。

  `--projection <快照>`（R1，CI 用）：不读 live map，改比对**受版本控制的叙事投影快照**
      `scripts/tests/fixtures/map_prose_projection.json` 与 policy 是否同代（摘要与口径全等）。
      CI 无 `.easyvibe/` live map，故这是叙事面在 CI 上「可见、可红、可复检」的落点。

fail-closed：policy 缺 `gates.notes_projection` / `notes_sha256` / `notes_sha256_by_module` /
`prose_quantity_rules` / `prose_provenance_keys` 即红，绝不静默回退到默认值（否则闸门自身成为
新的歧义源）。

边界（诚实声明）：I14 只能判「文本是否**变化**」；「文本是否为**真**」由 J（语义）+ K（计数型措辞
规则）+ L（基准同代）+ 既有判据 H + 人工评审共同承担——K/L 是 R2/R6 引入的「为真」判据的一半。

本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""
import argparse
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import map_policy  # noqa: E402

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LIVE = os.path.join(REPO, ".easyvibe", "map", "map.json")

# 未闭环/待处理语义标记（判据 J）：与退役 id 同分句出现即视为「复现为未闭环」
OPENNESS_MARKERS = ["未闭环", "仍未", "仍存在", "待处理", "尚未", "未解决", "未清零",
                    "仍需", "未收敛", "未纳入", "未修", "待闭环", "仍缺"]
_QUOTED = re.compile(r"[「『\"'][^」』\"']*[」』\"']")
_UNIT = re.compile(r"[。；\n]+")
# INV-5：`review_note`（可带 ['...'] 下标）后紧跟字面量赋值
_ASSIGN = re.compile(r"review_note'?\]?\s*=\s*[\"']")


def _units(note):
    for unit in _UNIT.split(note or ""):
        if unit.strip():
            yield unit.strip()


def _strip_quotes(unit):
    return _QUOTED.sub(" ", unit)


def digest_problems(policy, live):
    """I14：prose 摘要闸门。返回问题列表（空 = 同代）。"""
    try:
        want_arch = map_policy.notes_sha256(policy)
        want_mods = map_policy.notes_sha256_by_module(policy)
    except map_policy.PolicyMissing as e:
        return ["F 摘要字段缺失（fail-closed）: %s" % e.dotted_key]
    problems = []
    ids = [m.get("id") for m in live.get("modules", [])]
    if set(want_mods) != set(ids):
        problems.append("F 格集不等: policy-only=%s map-only=%s"
                        % (sorted(set(want_mods) - set(ids)), sorted(set(ids) - set(want_mods))))
    got_arch = map_policy.prose_arch_digest(live)
    if got_arch != want_arch:
        problems.append("F arch note 漂移: policy=%s live=%s" % (want_arch[:16], got_arch[:16]))
    for mid in ids:
        if mid not in want_mods:
            continue
        got = map_policy.prose_module_digest(mid, live)
        if got != want_mods[mid]:
            problems.append("F note 漂移@%s: policy=%s live=%s" % (mid, want_mods[mid][:16], got[:16]))
    return problems


def retired_prose_problems(policy, live):
    """J：退役 concern 不得在 prose 中复现为「未闭环」。"""
    problems = []
    retired = sorted(map_policy.retired_concerns(policy))
    if not retired:                       # 唯一允许的「空即跳过」
        return problems
    notes = [("arch", (live.get("health") or {}).get("review_note", ""))]
    notes += [(m.get("id"), (m.get("health") or {}).get("review_note", ""))
              for m in live.get("modules", [])]
    for owner, note in notes:
        for unit in _units(note):
            bare = _strip_quotes(unit)
            hits = [r for r in retired if r in bare]
            if hits and any(mk in bare for mk in OPENNESS_MARKERS):
                problems.append("J 退役 concern 在 prose 中复现为「未闭环」@%s: %s" % (owner, hits))
    return problems


def _prose_units(live):
    """全部受判 prose 单元：顶层 review_note + 逐格 review_note + 逐格 notes。"""
    units = [("arch", (live.get("health") or {}).get("review_note", ""))]
    for m in live.get("modules", []):
        units.append((m.get("id"), (m.get("health") or {}).get("review_note", "")))
        units.append(("%s.notes" % m.get("id"), m.get("notes", "")))
    return units


def quantity_problems(policy, live):
    """K：note 不得含滑动窗口量的精确值（families 来自 policy，fail-closed）。"""
    try:
        rules = map_policy.prose_quantity_rules(policy)
        ppk = map_policy.prose_provenance_keys(policy)
    except map_policy.PolicyMissing as e:
        return ["F 口径字段缺失（fail-closed）: %s" % e.dotted_key]
    if rules.get("mode") != "digits-forbidden":
        return ["K mode 非 digits-forbidden（fail-closed）: %r" % rules.get("mode")]
    fams = rules.get("families")
    if not isinstance(fams, dict) or not fams:
        return ["K families 为空（fail-closed）"]
    compiled = []
    for fid, pat in fams.items():
        try:
            compiled.append((fid, re.compile(pat)))
        except re.error as e:
            return ["K families.%s 正则不可编译（fail-closed）: %s" % (fid, e)]
    marker = rules.get("exempt_marker") or ""
    problems = []
    for owner, note in _prose_units(live):
        for unit in _units(note):
            if marker and marker in unit:
                continue
            bare = _strip_quotes(unit)          # 引述白名单（whitelist: quoted-citation）
            for fid, rx in compiled:
                m = rx.search(bare)
                if m:
                    problems.append("K 滑动窗口量进 prose@%s[%s]: %r" % (owner, fid, m.group(0)))
    return problems


_SHA = re.compile(r"HEAD\s+([0-9a-f]{7,40})")


def provenance_problems(policy, live):
    """L：叙事基准（HEAD <sha>）必须与 meta.provenance 同代（fail-closed）。"""
    try:
        keys = map_policy.prose_provenance_keys(policy)
    except map_policy.PolicyMissing as e:
        return ["F 口径字段缺失（fail-closed）: %s" % e.dotted_key]
    problems = []
    prov = ((live.get("meta") or {}).get("provenance"))
    if not isinstance(prov, dict):
        return ["L meta.provenance 缺失（fail-closed；生成期须注入）"]
    for k in keys:
        if not isinstance(prov.get(k), str) or not prov.get(k, "").strip():
            problems.append("L meta.provenance.%s 缺失/为空（fail-closed）" % k)
    head = prov.get("head_short") or ""
    if head and not re.fullmatch(r"[0-9a-f]{7,40}", head):
        problems.append("L meta.provenance.head_short 格式非法: %r" % head)
    for owner, note in _prose_units(live):
        for m in _SHA.finditer(note or ""):
            if m.group(1) != head:
                problems.append("L 叙事基准与 provenance 不同代@%s: note=%s provenance=%s"
                                % (owner, m.group(1), head or "<缺失>"))
    return problems


def projection_problems(policy, proj):
    """R1：受控叙事投影快照 ↔ policy 同代（CI 无 live map 时的叙事面判据）。"""
    if not isinstance(proj, dict):
        return ["P 投影快照不是 JSON 对象（fail-closed）"]
    problems = []
    try:
        want_proj = map_policy.notes_projection(policy)
        want_arch = map_policy.notes_sha256(policy)
        want_mods = map_policy.notes_sha256_by_module(policy)
        keys = map_policy.prose_provenance_keys(policy)
    except map_policy.PolicyMissing as e:
        return ["F 摘要字段缺失（fail-closed）: %s" % e.dotted_key]
    for key in ("notes_projection", "notes_sha256", "notes_sha256_by_module", "provenance"):
        if key not in proj:
            problems.append("P 投影快照缺 %s（fail-closed）" % key)
    if proj.get("notes_projection") != want_proj:
        problems.append("P 投影 notes_projection != policy（口径漂移）")
    if proj.get("notes_sha256") != want_arch:
        problems.append("P 投影 notes_sha256 != policy（顶层摘要漂移）")
    got_mods = proj.get("notes_sha256_by_module")
    if not isinstance(got_mods, dict):
        problems.append("P 投影 notes_sha256_by_module 缺失/非对象（fail-closed）")
    elif got_mods != want_mods:
        only_p = sorted(set(want_mods) - set(got_mods))
        only_s = sorted(set(got_mods) - set(want_mods))
        drift = sorted(k for k in set(want_mods) & set(got_mods) if want_mods[k] != got_mods[k])
        problems.append("P 投影逐格摘要与 policy 不同代: policy-only=%s snapshot-only=%s drift=%s"
                        % (only_p, only_s, drift))
    prov = proj.get("provenance")
    if not isinstance(prov, dict):
        problems.append("P 投影 provenance 非对象（fail-closed）")
    else:
        for k in keys:
            if not isinstance(prov.get(k), str) or not prov.get(k, "").strip():
                problems.append("P 投影 provenance.%s 缺失/为空（fail-closed）" % k)
        head = prov.get("head_short") or ""
        if head and not re.fullmatch(r"[0-9a-f]{7,40}", head):
            problems.append("P 投影 provenance.head_short 格式非法: %r" % head)
    bands = proj.get("churn_bands")
    if not isinstance(bands, dict) or not bands:
        problems.append("P 投影 churn_bands 缺失/为空（fail-closed）")
    elif set(bands) != set(map_policy.module_ids(policy)):
        problems.append("P 投影 churn_bands 键集 != policy.modules")
    elif any(b not in ("high", "medium", "low") for b in bands.values()):
        problems.append("P 投影 churn_bands 含非法档位（仅 high/medium/low）")
    return problems


def single_source_problems(root):
    """INV-5：**构建/迁移路径**（run/** 与受管 CLI 副本）不得硬编码 review_note 字面量。

    范围界定（诚实声明）：`run/**` 是「构建期整体覆盖已纠正 note」的回归向量（Δ4 的 `ARCH_REVIEW_NOTE`
    正住在这里），故纳入断言。`.easyvibe/map/_tools/spec_*.py` 是 **authoring 面**（模块规格生成器，
    逐格 note 的原始宿主、gitignored 且不被地图管线执行）——把它们判红会让闸门在真实仓库上永久为红，
    属「口径过宽」，故**不在**本断言范围内；它们的叙事同样受 I14 摘要闸门（prose 一旦落进 live map 即被冻结比对）约束。
    """
    problems = []
    targets = [os.path.join(root, "run")]
    copy = os.path.join(root, ".easyvibe", "map", "easyvibe_map_cli.py")
    if os.path.isfile(copy):
        targets.append(copy)
    for t in targets:
        if os.path.isfile(t):
            paths = [t]
        elif os.path.isdir(t):
            paths = [os.path.join(dp, fn) for dp, _dn, fns in os.walk(t) for fn in fns if fn.endswith(".py")]
        else:
            continue
        for p in paths:
            try:
                text = open(p, encoding="utf-8", errors="replace").read()
            except OSError:
                continue
            for i, line in enumerate(text.splitlines(), 1):
                if _ASSIGN.search(line):
                    problems.append("INV-5 review_note 字面量赋值: %s:%d" % (os.path.relpath(p, root), i))
    return problems


def run_check(map_path):
    policy = map_policy.load_policy(REPO)
    problems = ["F0 %s" % p for p in map_policy.validate_policy(policy)]
    if not os.path.isfile(map_path):
        problems.append("live map 不存在: %s" % map_path)
        live = None
    else:
        live = json.load(open(map_path, encoding="utf-8"))
    if live is not None:
        problems += digest_problems(policy, live)
        problems += retired_prose_problems(policy, live)
        problems += quantity_problems(policy, live)
        problems += provenance_problems(policy, live)
    problems += single_source_problems(REPO)
    report = {"ok": not problems, "map": os.path.relpath(map_path, REPO),
              "modules": len((live or {}).get("modules", [])),
              "arch_digest": map_policy.prose_arch_digest(live) if live else None,
              "problems": problems}
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["ok"] else 1


def run_projection(policy, proj_path):
    """R1：CI 入口——只比对受版本控制的投影快照与 policy（不读 live map）。"""
    problems = ["F0 %s" % p for p in map_policy.validate_policy(policy)]
    if not os.path.isfile(proj_path):
        problems.append("P 投影快照不存在: %s（fail-closed）" % proj_path)
        proj = None
    else:
        try:
            proj = json.load(open(proj_path, encoding="utf-8"))
        except ValueError as e:
            problems.append("P 投影快照不可解析: %s" % e)
            proj = None
    if proj is not None:
        problems += projection_problems(policy, proj)
    report = {"ok": not problems, "mode": "projection",
              "projection": os.path.relpath(proj_path, REPO),
              "modules": len((proj or {}).get("notes_sha256_by_module", {})),
              "problems": problems}
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["ok"] else 1


_RULES = {"mode": "digits-forbidden",
          "families": {"churn_touches": r"churn\s*(?:近\s*)?\d+\s*[天月][^。；\n]{0,12}?\d+\s*次",
                       "touches_phrase": r"\d+\s*次文件触点",
                       "global_rank": r"全库第\s*\d+",
                       "rank_fraction": r"第\s*\d+/\d+",
                       "product_files": r"\d+\s*个产品文件",
                       "file_loc": r"\d+\s*行"},
          "whitelist": ["quoted-citation"], "exempt_marker": "<!--volatile-exempt-->"}


def _synth():
    live = {"meta": {"provenance": {"head_short": "abcdef0", "generated_at": "2026-10-08T11:23:07+08:00"}},
            "health": {"review_note": "顶层叙事 A"},
            "modules": [{"id": "m1", "notes": "格一注记", "health": {"review_note": "格一叙事"}},
                        {"id": "m2", "notes": "格二注记", "health": {"review_note": "格二叙事"}}]}
    policy = {"gates": {
        "retired_concerns": ["c-x-1"],
        "notes_sha256": map_policy.prose_arch_digest(live),
        "notes_sha256_by_module": {m["id"]: map_policy.prose_module_digest(m["id"], live)
                                   for m in live["modules"]},
        "prose_quantity_rules": _RULES,
        "prose_provenance_keys": ["head_short", "generated_at"],
    }}
    return policy, live


def selfcheck():
    import tempfile
    results = []

    def add(name, passed, detail=""):
        results.append((name, bool(passed), detail))

    policy, live = _synth()
    add("n1 正例（摘要同代 + 无退役 id）→ 必绿",
        not digest_problems(policy, live) and not retired_prose_problems(policy, live))

    drift = json.loads(json.dumps(live))
    drift["health"]["review_note"] = "顶层叙事 B"
    probs = digest_problems(policy, drift)
    add("n2 arch note 改一位 → 必红", bool(probs), "; ".join(probs[:1]))

    drift2 = json.loads(json.dumps(live))
    drift2["modules"][1]["health"]["review_note"] = "格二叙事改"
    probs = digest_problems(policy, drift2)
    add("n3 某格 note 改一位 → 必红且定位到格",
        bool(probs) and any("@m2" in p for p in probs), "; ".join(probs[:1]))

    miss = {"gates": dict(policy["gates"])}
    miss["gates"].pop("notes_sha256")
    add("n4 policy 缺 notes_sha256 → 必红（fail-closed）", bool(digest_problems(miss, live)))

    extra = json.loads(json.dumps(policy))
    extra["gates"]["notes_sha256_by_module"]["m3"] = "0" * 64
    probs = digest_problems(extra, live)
    add("n5 by_module 多一格 → 必红（格集不等）", any("格集不等" in p for p in probs),
        "; ".join(probs[:1]))

    messy = json.loads(json.dumps(live))
    messy["health"]["review_note"] = "顶层叙事 A\r\n   \r\n"
    add("n6 规范化等价（CRLF/尾随空格/空行）→ 必绿",
        map_policy.prose_arch_digest(messy) == policy["gates"]["notes_sha256"])

    add("n7 真实 run/** 无 review_note 字面量 → 必绿", not single_source_problems(REPO),
        "; ".join(single_source_problems(REPO)[:1]))
    with tempfile.TemporaryDirectory() as td:
        os.makedirs(os.path.join(td, "run"))
        with open(os.path.join(td, "run", "x.py"), "w", encoding="utf-8") as fh:
            fh.write('def f(ah):\n    out = dict(ah)\n    out[\'review_note\'] = "硬编码旧叙事"\n')
        add("n7b 受控外硬编码 review_note 字面量 → 必红", bool(single_source_problems(td)))

    open_live = json.loads(json.dumps(live))
    open_live["modules"][0]["health"]["review_note"] = "本次巡检 c-x-1 仍未闭环"
    probs = retired_prose_problems(policy, open_live)
    add("n8 退役 concern 复现为「未闭环」→ 必红", bool(probs), "; ".join(probs[:1]))

    close_live = json.loads(json.dumps(live))
    close_live["modules"][0]["health"]["review_note"] = "本轮摘除 c-x-1（已闭环）"
    add("n9 正当表述「本轮摘除 c-x-1」→ 必绿", not retired_prose_problems(policy, close_live))

    other_live = json.loads(json.dumps(live))
    other_live["modules"][0]["health"]["review_note"] = "c-zzz-9 仍未闭环"
    add("n10 非退役 id 的「未闭环」→ 必绿（不越界）",
        not retired_prose_problems(policy, other_live))

    quote_live = json.loads(json.dumps(live))
    quote_live["modules"][0]["health"]["review_note"] = "上轮断言「c-x-1 未闭环」与代码矛盾，本轮订正"
    add("n11 引述式「『c-x-1 未闭环』」的订正句 → 必绿（不误伤）",
        not retired_prose_problems(policy, quote_live))

    # ---- R3 扩面：notes 入摘要闸门 ----
    notes_drift = json.loads(json.dumps(live))
    notes_drift["modules"][1]["notes"] = "格二注记改一位"
    probs = digest_problems(policy, notes_drift)
    add("n14 某格 notes 改一位 → 必红且定位到格",
        bool(probs) and any("@m2" in p for p in probs), "; ".join(probs[:1]))

    notes_miss = json.loads(json.dumps(live))
    notes_miss["modules"][0].pop("notes")
    probs = digest_problems(policy, notes_miss)
    add("n15 某格 notes 缺失 → 必红（缺失以 None 参与摘要，fail-closed）",
        bool(probs) and any("@m1" in p for p in probs), "; ".join(probs[:1]))

    swap = json.loads(json.dumps(live))
    swap["modules"][0]["health"]["review_note"], swap["modules"][0]["notes"] = \
        live["modules"][0]["notes"], live["modules"][0]["health"]["review_note"]
    probs = digest_problems(policy, swap)
    add("n16 review_note 与 notes 互换文本 → 必红（防跨字段串扰）", bool(probs), "; ".join(probs[:1]))

    # ---- R2 计数规则（K）----
    k_live = json.loads(json.dumps(live))
    k_live["modules"][0]["health"]["review_note"] = "churn 近 3 月 255 次文件触点（全库第 2）"
    add("k1 note 写「近 3 月 255 次 / 全库第 2」→ 必红",
        len(quantity_problems(policy, k_live)) >= 1, "; ".join(quantity_problems(policy, k_live)[:2]))

    k_quote = json.loads(json.dumps(live))
    k_quote["modules"][0]["health"]["review_note"] = "上轮曾写「近 3 月 255 次」，本轮按档位订正"
    add("k2 引述式「曾写 255 次」→ 必绿（引述白名单，不误伤）",
        not quantity_problems(policy, k_quote), "; ".join(quantity_problems(policy, k_quote)[:1]))

    k_band = json.loads(json.dumps(live))
    k_band["modules"][0]["health"]["review_note"] = "churn 档位 high（口径见 gates.churn_basis）"
    add("k4 离散档位「churn 档位 high」→ 必绿（档位不受限）",
        not quantity_problems(policy, k_band), "; ".join(quantity_problems(policy, k_band)[:1]))

    no_rules = json.loads(json.dumps(policy))
    no_rules["gates"].pop("prose_quantity_rules")
    add("k3 缺 gates.prose_quantity_rules → 必红（fail-closed）", bool(quantity_problems(no_rules, live)))

    k_loc = json.loads(json.dumps(live))
    k_loc["modules"][0]["notes"] = "主件 490 行、产品文件 423 个产品文件"
    add("k5 notes 里写行数与产品文件数 → 必红（notes 同受 K 约束）",
        len(quantity_problems(policy, k_loc)) >= 1)

    # ---- R6 叙事基准（L）----
    l_ok = json.loads(json.dumps(live))
    l_ok["modules"][0]["notes"] = "本模块条目在 HEAD abcdef0（2026-10-08T11:23:07+08:00）逐条复核"
    add("l0 基准 sha == provenance → 必绿", not provenance_problems(policy, l_ok))

    l_bad = json.loads(json.dumps(live))
    l_bad["modules"][0]["notes"] = "本模块条目在 HEAD 4d09e7e 逐条复核"
    add("l1 基准 sha 与 provenance 不同代 → 必红", bool(provenance_problems(policy, l_bad)))

    l_miss = json.loads(json.dumps(live))
    l_miss.pop("meta")
    add("l2 meta.provenance 缺失 → 必红（fail-closed）", bool(provenance_problems(policy, l_miss)))

    # ---- R1 投影快照 ↔ policy 同代 ----
    proj = {"notes_projection": {"fields": ["health.review_note", "notes"],
                                 "normalize": "eol+rstrip+collapse-blank+strip",
                                 "algo": "sha256+canonical-json"},
            "notes_sha256": policy["gates"]["notes_sha256"],
            "notes_sha256_by_module": policy["gates"]["notes_sha256_by_module"],
            "provenance": live["meta"]["provenance"],
            "churn_bands": {"m1": "high", "m2": "low"}}
    pol2 = json.loads(json.dumps(policy))
    pol2["gates"]["notes_projection"] = proj["notes_projection"]
    pol2["modules"] = [{"id": "m1", "emit_order": 0}, {"id": "m2", "emit_order": 1}]
    add("p1 投影与 policy 同代 → 必绿", not projection_problems(pol2, proj),
        "; ".join(projection_problems(pol2, proj)[:1]))

    proj_drift = json.loads(json.dumps(proj))
    proj_drift["notes_sha256_by_module"]["m1"] = "0" * 64
    add("n13 投影逐格摘要与 policy 不同代 → 必红",
        any("不同代" in p for p in projection_problems(pol2, proj_drift)))

    proj_miss = json.loads(json.dumps(proj))
    proj_miss.pop("notes_sha256_by_module")
    add("n12 投影缺 notes_sha256_by_module → 必红（fail-closed）",
        any("fail-closed" in p for p in projection_problems(pol2, proj_miss)))

    proj_noband = json.loads(json.dumps(proj))
    proj_noband["churn_bands"] = {"m1": "high"}
    add("p2 投影 churn_bands 键集 != policy.modules → 必红",
        any("churn_bands" in p for p in projection_problems(pol2, proj_noband)))

    ok = True
    for name, passed, detail in results:
        ok = ok and passed
        print("  %s %s  %s" % ("PASS" if passed else "FAIL", name, detail))
    print("n1–n16 / k1–k5 / l0–l2 / p1–p2 %s" % ("全 PASS" if ok else "有 FAIL"))
    return ok


def main():
    ap = argparse.ArgumentParser(description="叙事（prose）同代判据（c-arch-17/18 / R2+R3+R4+R6+R1）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--map", default=LIVE, help="默认 live map；fixture 无 prose，故不做默认载体")
    ap.add_argument("--projection", default=None,
                    help="只比对受版本控制的叙事投影快照与 policy（CI 入口，无 live 依赖）")
    args = ap.parse_args()
    if args.selfcheck:
        return 0 if selfcheck() else 1
    if args.projection:
        return run_projection(map_policy.load_policy(REPO), os.path.abspath(args.projection))
    return run_check(os.path.abspath(args.map))


if __name__ == "__main__":
    sys.exit(main())
