#!/usr/bin/env python3
"""REST/WS 直连守卫（c-arch-4 / R10）：presentation 业务文件不得绕过 src/api/** 直连 fetch。

范式同 scripts/check_components_ownership.py（纯 python3、秒级、--check / --selfcheck）。
本仓 CI 明示「不跑 vitest」（见 .github/workflows/asset-guard.yml 头部），故本地 archGuard
断言组 5 之外，必须有一条 CI 可见的等价判据——且用**原始文本**（不去注释）最严口径，
使「注释盲区吞代码」在 CI 语义下不可能让违规消失。

判据（RE 与 archGuard 断言组 5 同款）：
  `fetch(`（非成员调用） ｜ `'/api/…'` 字面量（含 `"`/`` ` ``） ｜ `new WebSocket`
扫描面：easyvibe-renderer/src/**   排除 src/api/**、__tests__/**、*.test.ts(x)、types/generated
分组：
  business  业务文件（presentation 四域 + pages/hooks/lib/App 等）——迁移完成 = 空表
  network   src/runtime/**（R5 裁定：renderer-core 受控网络层出口，独立登记、只降不升）

模式：
  --check [--baseline scripts/rest_baseline.json]
      两组各自 ① 键集双向全等 ② per-file 实际 ≤ 预算；任一不满足 → exit 1（fail-closed）
  --selfcheck   负例自证（S1 加一处 fetch 必红 / S2 删基线项必红 / S3 合法改注释不变 / S4 复原必绿）
  --map <map.json>  地图 presentation 口径边集复核（A8a/A8b/A8c；本地/归纳期）

本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""
import argparse
import json
import os
import re
import sys

RENDERER = os.path.join("easyvibe-renderer", "src")
DEFAULT_BASELINE = os.path.join("scripts", "rest_baseline.json")

RE_FETCH = re.compile(r"(?<![\w.])fetch\s*\(")
RE_APIPATH = re.compile(r"['\"`]/api/")
RE_WS = re.compile(r"new\s+WebSocket\b")

# presentation 五模块（地图口径 A8a/A8b 的 from 白名单；settings-ui 于 c-console-ui-3 析出）
PRESENTATION = {"console-ui", "settings-ui", "chat-ui", "task-ui", "map-canvas"}
APPLICATION = {"server-api", "task-engine", "event-bus"}


def is_excluded(rel):
    """rel 形如 src/components/... （相对 easyvibe-renderer）。"""
    if rel.startswith("src/api/"):
        return True
    if "__tests__/" in rel or rel.endswith((".test.ts", ".test.tsx")):
        return True
    if rel.startswith("src/types/generated"):
        return True
    return False


def count_hits(text):
    return len(RE_FETCH.findall(text)) + len(RE_APIPATH.findall(text)) + len(RE_WS.findall(text))


def scan(renderer_dir):
    """返回 (business, network)：rel -> 命中数（仅 >0 收录）。rel 相对 easyvibe-renderer。"""
    business, network = {}, {}
    if not os.path.isdir(renderer_dir):
        return business, network
    for dirpath, dirnames, filenames in os.walk(renderer_dir):
        dirnames.sort()
        for fn in sorted(filenames):
            if not fn.endswith((".ts", ".tsx")):
                continue
            path = os.path.join(dirpath, fn)
            rel = os.path.relpath(path, os.path.dirname(renderer_dir)).replace(os.sep, "/")
            if is_excluded(rel):
                continue
            try:
                with open(path, encoding="utf-8") as fh:
                    text = fh.read()
            except (UnicodeDecodeError, OSError):
                continue
            n = count_hits(text)
            if n <= 0:
                continue
            bucket = network if rel.startswith("src/runtime/") else business
            bucket[rel] = n
    return business, network


def check_group(label, actual, budget):
    problems = []
    if sorted(actual.keys()) != sorted(budget.keys()):
        only_actual = sorted(set(actual) - set(budget))
        only_budget = sorted(set(budget) - set(actual))
        problems.append("%s 键集不等：新增=%s 缺失=%s" % (label, only_actual, only_budget))
    for f, cap in sorted(budget.items()):
        got = actual.get(f, 0)
        if got > cap:
            problems.append("%s 只降不升：%s 实际 %d > 预算 %d" % (label, f, got, cap))
    return problems


def run(root, baseline_path):
    baseline_path = baseline_path if os.path.isabs(baseline_path) else os.path.join(root, baseline_path)
    with open(baseline_path, encoding="utf-8") as fh:
        baseline = json.load(fh)
    business, network = scan(os.path.join(root, RENDERER))
    problems = []
    problems += check_group("business", business, baseline.get("business", {}))
    problems += check_group("network", network, baseline.get("network", {}))
    disabled = [x for x in ("business", "network") if baseline.get(x) == [] or x not in baseline]
    return problems, business, network, disabled


def map_scope(map_path):
    """A8a/A8b/A8c：presentation 口径边集复核。返回 (problems, detail)。"""
    with open(map_path, encoding="utf-8") as fh:
        m = json.load(fh)
    edges = m.get("edges", [])
    api_edges = [e for e in edges if e.get("type") == "api"]
    a8a = [e for e in api_edges if e.get("from") in PRESENTATION]
    a8b = [e for e in api_edges if e.get("from") in PRESENTATION and e.get("to") in APPLICATION]
    detail = "A8a(presentation api 边=0): %d ｜ A8b(presentation→application api 边=0): %d ｜ A8c(全库 api 边登记): %s" % (
        len(a8a), len(a8b), sorted(e.get("id") for e in api_edges),
    )
    problems = []
    if a8a:
        problems.append("A8a 失败：presentation 存在 api 型直连边 %s" % [e.get("id") for e in a8a])
    if a8b:
        problems.append("A8b 失败：presentation→application api 边 %s" % [e.get("id") for e in a8b])
    return problems, detail


def _fixture(tmp):
    base = os.path.join(tmp, RENDERER)
    os.makedirs(os.path.join(base, "components", "chat"), exist_ok=True)
    os.makedirs(os.path.join(base, "runtime"), exist_ok=True)
    with open(os.path.join(base, "components", "chat", "QuickAsk.tsx"), "w", encoding="utf-8") as fh:
        fh.write("export const a = 1 // 见 ./api/*（注释里的干扰）\n")
    with open(os.path.join(base, "runtime", "ws.ts"), "w", encoding="utf-8") as fh:
        fh.write("export const ws = () => new WebSocket('/ws')\n")
    return base


def selfcheck(real_root):
    import tempfile

    results = []
    with tempfile.TemporaryDirectory(prefix="renderer-rest-") as tmp:
        base = _fixture(tmp)
        base_path = os.path.join(tmp, "baseline.json")
        qa = os.path.join(base, "components", "chat", "QuickAsk.tsx")

        def write_baseline(business, network):
            with open(base_path, "w", encoding="utf-8") as fh:
                json.dump({"business": business, "network": network}, fh)

        # S1 加一处 fetch → business 键集新增 → 必红
        write_baseline({}, {"src/runtime/ws.ts": 1})
        with open(qa, "a", encoding="utf-8") as fh:
            fh.write("export const b = () => fetch('/api/x')\n")
        p1, _, _, _ = run(tmp, base_path)
        results.append(("S1 新增直连 → 必红", any(x.startswith("business 键集不等") for x in p1), "; ".join(p1[:1])))
        # S2 基线登记某文件但磁盘已无 → 键集缺项 → 必红
        write_baseline({"src/components/chat/QuickAsk.tsx": 1}, {"src/runtime/ws.ts": 1})
        with open(qa, "w", encoding="utf-8") as fh:
            fh.write("export const a = 1\n")
        p2, _, _, _ = run(tmp, base_path)
        results.append(("S2 基线缺项 → 必红", any(x.startswith("business 键集不等") for x in p2), "; ".join(p2[:1])))
        # S3 合法改注释（含块注释起始干扰）→ 计数不变
        write_baseline({}, {"src/runtime/ws.ts": 1})
        with open(qa, "w", encoding="utf-8") as fh:
            fh.write("// 见 ./settings/*（2026-10-05）\nexport const a = 1\n{/* JSX 注释 */}\n")
        p3, _, _, _ = run(tmp, base_path)
        results.append(("S3 注释干扰 → 不误报", not p3, "; ".join(p3[:1])))
        # S4 复原 → 必绿
        p4, _, _, _ = run(tmp, base_path)
        results.append(("S4 合法基线 → 必绿", not p4, "; ".join(p4[:1])))
    # S5 真仓库 → 必绿
    problems, business, network, disabled = run(real_root, os.path.join(real_root, DEFAULT_BASELINE))
    detail = "business=%d network=%d" % (len(business), len(network))
    if disabled:
        detail += "（基线缺组：%s）" % ",".join(disabled)
    results.append(("S5 真仓库扫描 → 必绿", not problems, detail))
    ok = True
    for name, passed, d in results:
        ok = ok and passed
        print("  %s %s  %s" % ("PASS" if passed else "FAIL", name, d))
    return ok


def main():
    ap = argparse.ArgumentParser(description="REST/WS 直连守卫（c-arch-4 / R10）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--map", default=None)
    ap.add_argument("--baseline", default=DEFAULT_BASELINE)
    ap.add_argument("--root", default=os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    args = ap.parse_args()
    root = os.path.abspath(args.root)
    if args.selfcheck:
        print("▶ REST/WS 直连守卫自检 S1–S5")
        return 0 if selfcheck(root) else 1
    if args.map:
        problems, detail = map_scope(os.path.abspath(args.map))
        for p in problems:
            print("  FAIL %s" % p)
        print("  %s %s" % ("OK  " if not problems else "FAIL", detail))
        return 0 if not problems else 1
    problems, business, network, disabled = run(root, args.baseline)
    for p in problems:
        print("  FAIL %s" % p)
    summary = "business=%s network=%s" % (
        {k: v for k, v in sorted(business.items())}, {k: v for k, v in sorted(network.items())},
    )
    print("  %s --check 问题 %d ｜ %s" % ("OK  " if not problems else "FAIL", len(problems), summary))
    return 0 if not problems else 1


if __name__ == "__main__":
    sys.exit(main())
