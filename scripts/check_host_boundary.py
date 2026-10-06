#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""宿主能力边界守卫（c-arch-5，纯 python3 CI 载体）。

背景：全库唯一 direction_violation（renderer-runtime(order 2) → desktop-shell(order 0) 的宿主能力
门面 src/runtime/host.ts）被结构性消除为「端口 + app-entry 适配模块 + 同层引导入口」：
  · 端口  = renderer-runtime 的 src/runtime/host.ts（契约 + 注册表 + 默认 no-op，零 @tauri-apps）；
  · 适配器= app-entry 的 src/host-adapter/register.ts（全库唯一 @tauri-apps/* 落点）；
  · 引导  = src/host-adapter/boot.tsx（先注册后装载 App；index.html 入口指向它，同归 host-adapter）。
本仓 CI 明示「不跑 vitest」（见 .github/workflows/asset-guard.yml 头部），故把命题「不再复现」落成
纯 python3、秒级、fail-closed 的 CI 判据。

判据（--check）：
  B1 唯一落点：easyvibe-renderer/src/**（去注释、静态 from + 动态 import 双收、排除测试）
     的 @tauri-apps/* 说明符只许出现在 src/host-adapter/**，且适配模块确实承载之；
     runtime / presentation / presentation-support 一律零命中。
  B2 无逆边（正面结构断言）：地图中 order∈{1,2} 的模块出边不得指向 order==0 的模块；
     且全局 direction_violation == 0。
  B3 加载链：index.html 入口 = /src/host-adapter/boot.tsx；boot.tsx 静态 import 序 =
     先 './register' 后 '@/main'（无竞态）；适配器/引导文件在磁盘；除适配模块外任何源码
     不得 import '@/host-adapter/**'（防 presentation 反向依赖 app-entry）。
  B4 归属正确：地图 host-adapter.files 含 index.html 且覆盖适配模块代码；console-ui.files 不含
     index.html；map.modules 中 host-adapter 下标 < console-ui 下标（首匹配归属，防 index.html 被抢回）。

模式：
  --check [--map PATH]   默认读 .easyvibe/map/map.json；CI 传受版本控制 fixture。
  --selfcheck            合成输入 N0–N5（不读 live map／不改真仓库），逐条自证负例必红、正例必绿。

本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
退出码：--check 绿=0 / 红=1；--selfcheck 全 PASS=0 / 任一 FAIL=1。
"""
import argparse
import json
import os
import re
import sys
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_MAP = os.path.join(REPO, ".easyvibe", "map", "map.json")
EMIT_ORDER = os.path.join(REPO, ".easyvibe", "map", "emit_order.json")

RENDERER_SRC = "easyvibe-renderer/src"
ADAPTER_PREFIX = "easyvibe-renderer/src/host-adapter/"
REGISTER_FILE = "easyvibe-renderer/src/host-adapter/register.ts"
BOOT_FILE = "easyvibe-renderer/src/host-adapter/boot.tsx"
PORT_FILE = "easyvibe-renderer/src/runtime/host.ts"
INDEX_HTML = "easyvibe-renderer/index.html"
HOST_SPEC_RE = re.compile(r"^@tauri-apps/")

# 静态 `from '…'` / 动态 `import('…')` / 副作用裸 `import '…'` 三形态全收
SPEC_RE = re.compile(r"(?:from\s+|import\s*\(\s*|import\s+)(['\"])([^'\"]+)\1")


def strip_comments(text):
    """单遍状态机去 // 与 /* */ 注释；字符串/模板字面量原样保留（避免 '//' 误判）。"""
    out = []
    i, n = 0, len(text)
    state = "code"  # code | line | block | string
    quote = ""
    while i < n:
        c = text[i]
        d = text[i + 1] if i + 1 < n else ""
        if state == "code":
            if c == "/" and d == "/":
                state = "line"; i += 2; continue
            if c == "/" and d == "*":
                state = "block"; i += 2; continue
            if c in "'\"`":
                quote = c; state = "string"; out.append(c); i += 1; continue
            out.append(c); i += 1; continue
        if state == "line":
            if c == "\n":
                state = "code"; out.append(c)
            i += 1; continue
        if state == "block":
            if c == "*" and d == "/":
                state = "code"; i += 2; continue
            i += 1; continue
        # string
        if c == "\\":
            out.append(c)
            if d:
                out.append(d)
            i += 2; continue
        if c == quote:
            state = "code"
        out.append(c); i += 1
    return "".join(out)


def import_specs(text):
    return [m.group(2) for m in SPEC_RE.finditer(strip_comments(text))]


def _read(root, rel):
    with open(os.path.join(root, rel), encoding="utf-8", errors="ignore") as fh:
        return fh.read()


def _iter_ts(root, base):
    """遍历 base 下所有 .ts/.tsx，返回仓库相对 posix 路径列表。"""
    out = []
    absbase = os.path.join(root, base)
    for dirpath, dirnames, filenames in os.walk(absbase):
        dirnames.sort()
        for fn in sorted(filenames):
            if fn.endswith((".ts", ".tsx")):
                out.append(os.path.relpath(os.path.join(dirpath, fn), root).replace(os.sep, "/"))
    return out


def _is_test(rel):
    return "__tests__/" in rel or rel.endswith((".test.ts", ".test.tsx"))


# ------------------------------------------------------------------ B1
def check_b1(root):
    """唯一落点。返回 (problems, hits: rel -> [specs])。"""
    problems, hits = [], {}
    if not os.path.isdir(os.path.join(root, RENDERER_SRC)):
        return ["B1 扫描面缺失: %s" % RENDERER_SRC], hits
    for rel in _iter_ts(root, RENDERER_SRC):
        if _is_test(rel):
            continue
        spec = [s for s in import_specs(_read(root, rel)) if HOST_SPEC_RE.match(s)]
        if spec:
            hits[rel] = spec
    outside = sorted(r for r in hits if not r.startswith(ADAPTER_PREFIX))
    if outside:
        problems.append("B1 @tauri-apps/* 出现在适配模块之外（宿主实现只许落 %s）: %s"
                        % (ADAPTER_PREFIX, {r: hits[r] for r in outside}))
    inside = sorted(r for r in hits if r.startswith(ADAPTER_PREFIX))
    if not inside:
        problems.append("B1 适配模块未承载任何 @tauri-apps/*（唯一落点例外是空洞的）")
    if os.path.isfile(os.path.join(root, PORT_FILE)):
        port_out = [s for s in import_specs(_read(root, PORT_FILE)) if HOST_SPEC_RE.match(s)]
        if port_out:
            problems.append("B1 端口 %s 出现 @tauri-apps/*（端口必须零宿主依赖）: %s" % (PORT_FILE, port_out))
    else:
        problems.append("B1 端口文件缺失: %s" % PORT_FILE)
    return problems, hits


# ------------------------------------------------------------------ B2
def check_b2(map_obj):
    """无逆边 + DV==0。返回 (problems, detail)。"""
    problems = []
    layer_order = {l.get("id"): l.get("order") for l in map_obj.get("layers", [])}
    mod_order = {m.get("id"): layer_order.get(m.get("layer")) for m in map_obj.get("modules", [])}
    bad = []
    for e in map_obj.get("edges", []):
        fo, to = mod_order.get(e.get("from")), mod_order.get(e.get("to"))
        if fo in (1, 2) and to == 0:
            bad.append((e.get("id"), e.get("from"), e.get("to")))
    if bad:
        problems.append("B2 order(1/2) → app-entry(0) 逆边仍存在: %s" % bad)
    dv = [e.get("id") for e in map_obj.get("edges", []) if e.get("direction_violation")]
    if dv:
        problems.append("B2 direction_violation != 0: %s" % dv)
    return problems, {"reversed_edges": bad, "direction_violation": len(dv)}


# ------------------------------------------------------------------ B3
def check_b3(root):
    """加载链：入口/boot 静态序/无反向 import 适配模块。"""
    problems = []
    for rel in (REGISTER_FILE, BOOT_FILE):
        if not os.path.isfile(os.path.join(root, rel)):
            problems.append("B3 缺文件: %s" % rel)
    idx = os.path.join(root, INDEX_HTML)
    if not os.path.isfile(idx):
        problems.append("B3 缺文件: %s" % INDEX_HTML)
    else:
        html = _read(root, INDEX_HTML)
        if "/src/host-adapter/boot.tsx" not in html:
            problems.append("B3 index.html 入口未指向 /src/host-adapter/boot.tsx")
    if os.path.isfile(os.path.join(root, BOOT_FILE)):
        specs = import_specs(_read(root, BOOT_FILE))
        if specs != ["./register", "@/main"]:
            problems.append("B3 boot.tsx 静态 import 序应为 ['./register','@/main']，实际 %s" % specs)
    offenders = []
    for rel in _iter_ts(root, RENDERER_SRC):
        if rel.startswith(ADAPTER_PREFIX) or _is_test(rel):
            continue
        for s in import_specs(_read(root, rel)):
            if s.startswith("@/host-adapter") or "host-adapter/" in s:
                offenders.append("%s → %s" % (rel, s))
    if offenders:
        problems.append("B3 非适配模块反向 import 宿主适配器（新逆边）: %s" % offenders)
    return problems


# ------------------------------------------------------------------ B4
def check_b4(map_obj, emit_order=None):
    """归属：index.html 归 host-adapter；适配模块代码被覆盖；顺序先于 console-ui。"""
    problems = []
    mods = {m.get("id"): m for m in map_obj.get("modules", [])}
    ha, cu = mods.get("host-adapter"), mods.get("console-ui")
    if ha is None:
        problems.append("B4 地图缺 host-adapter 模块（app-entry 适配落点未登记）")
    else:
        files = [str(x) for x in ha.get("files", [])]
        if INDEX_HTML not in files:
            problems.append("B4 host-adapter.files 未含 %s" % INDEX_HTML)
        if not any(f.startswith(ADAPTER_PREFIX) or f.rstrip("/") + "/" == ADAPTER_PREFIX for f in files):
            problems.append("B4 host-adapter.files 未覆盖 %s（适配器代码不在其上）" % ADAPTER_PREFIX)
    if cu is None:
        problems.append("B4 地图缺 console-ui 模块")
    elif INDEX_HTML in [str(x) for x in cu.get("files", [])]:
        problems.append("B4 console-ui.files 仍含 %s（归属未移交）" % INDEX_HTML)
    # 首匹配归属顺序：map.modules 顺序 == emit_order.modules 顺序（finalize assemble 保序）
    order = [m.get("id") for m in map_obj.get("modules", [])]
    if "host-adapter" in order and "console-ui" in order and order.index("host-adapter") >= order.index("console-ui"):
        problems.append("B4 map.modules 中 host-adapter 下标 %d 未先于 console-ui 下标 %d（index.html 可能被抢回）"
                        % (order.index("host-adapter"), order.index("console-ui")))
    if emit_order is not None:
        emods = emit_order.get("modules") or []
        if "host-adapter" not in emods or "console-ui" not in emods:
            problems.append("B4 emit_order.modules 缺 host-adapter/console-ui")
        elif emods.index("host-adapter") >= emods.index("console-ui"):
            problems.append("B4 emit_order.modules 中 host-adapter 未先于 console-ui")
    return problems


# ------------------------------------------------------------------ --check
def run_check(root, map_path):
    problems = []
    try:
        with open(map_path, encoding="utf-8") as fh:
            map_obj = json.load(fh)
    except (OSError, ValueError) as e:
        print(json.dumps({"ok": False, "stage": "load-map", "map": map_path,
                          "problems": ["地图加载失败（fail-closed）: %s" % e]}, ensure_ascii=False, indent=2))
        return 1
    p1, hits = check_b1(root)
    p2, detail2 = check_b2(map_obj)
    p3 = check_b3(root)
    emit_obj = None
    if os.path.isfile(EMIT_ORDER):
        try:
            with open(EMIT_ORDER, encoding="utf-8") as fh:
                emit_obj = json.load(fh)
        except (OSError, ValueError):
            emit_obj = None
    p4 = check_b4(map_obj, emit_obj)
    problems = p1 + p2 + p3 + p4
    report = {
        "ok": not problems,
        "map": os.path.relpath(map_path, root) if os.path.isabs(map_path) else map_path,
        "tauri_landing_points": sorted(hits),
        "structure": detail2,
        "problems": problems,
    }
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if not problems else 1


# ------------------------------------------------------------------ --selfcheck
_CLEAN_HTML = ('<!doctype html>\n<html><body><div id="root"></div>\n'
               '<script type="module" src="/src/host-adapter/boot.tsx"></script></body></html>\n')


def _write(root, rel, text):
    path = os.path.join(root, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text)


def _fixture_tree(root):
    """最小合成树：复刻端口/适配器/引导/入口的磁盘布局（正例）。"""
    _write(root, INDEX_HTML, _CLEAN_HTML)
    _write(root, "easyvibe-renderer/src/main.tsx", "import './index.css'\nexport const app = 1\n")
    _write(root, "easyvibe-renderer/src/runtime/env.ts", "export function isTauriRuntime() { return false }\n")
    _write(root, PORT_FILE, "import { isTauriRuntime } from '@/runtime/env'\nexport { isTauriRuntime }\n")
    _write(root, REGISTER_FILE,
           "import { setHostCapabilities } from '@/runtime/host'\n"
           "const loadNotify = () => import('@tauri-apps/plugin-notification')\n"
           "export function registerHostCapabilities() { void loadNotify }\n")
    _write(root, BOOT_FILE, "import './register'\nimport '@/main'\n")


def _fixture_map(order=("host-adapter", "console-ui", "renderer-runtime"), reversed_edge=False):
    modules = [
        {"id": "host-adapter", "layer": "app-entry",
         "files": [ADAPTER_PREFIX + "**", INDEX_HTML], "dependencies": ["console-ui", "renderer-runtime"]},
        {"id": "console-ui", "layer": "presentation",
         "files": ["easyvibe-renderer/src/main.tsx"], "dependencies": ["renderer-runtime"]},
        {"id": "renderer-runtime", "layer": "presentation-support",
         "files": ["easyvibe-renderer/src/runtime/**"], "dependencies": []},
    ]
    by_id = {m["id"]: m for m in modules}
    modules = [by_id[i] for i in order]
    edges = [
        {"id": "e1", "from": "host-adapter", "to": "console-ui", "type": "import"},
        {"id": "e2", "from": "host-adapter", "to": "renderer-runtime", "type": "import"},
        {"id": "e3", "from": "console-ui", "to": "renderer-runtime", "type": "import"},
    ]
    if reversed_edge:
        edges.append({"id": "e4", "from": "renderer-runtime", "to": "host-adapter",
                      "type": "call", "direction_violation": True})
        by_id["renderer-runtime"]["dependencies"].append("host-adapter")
    return {"layers": [{"id": "app-entry", "order": 0}, {"id": "presentation", "order": 1},
                       {"id": "presentation-support", "order": 2}],
            "modules": modules, "edges": edges}


def selfcheck():
    results = []
    with tempfile.TemporaryDirectory(prefix="host-boundary-") as tmp:
        _fixture_tree(tmp)
        m0 = _fixture_map()
        p = check_b1(tmp)[0] + check_b2(m0)[0] + check_b3(tmp) + check_b4(m0)
        results.append(("N0 合成正例 → 必绿", not p, "; ".join(p[:1])))

        # N1 端口回归：把 @tauri-apps 注入 src/runtime/host.ts
        _write(tmp, PORT_FILE, "import { x } from '@tauri-apps/api/window'\nexport const y = x\n")
        p = check_b1(tmp)[0]
        results.append(("N1 端口注入 @tauri-apps → B1 必红", any(x.startswith("B1") for x in p), "; ".join(p[:1])))
        _write(tmp, PORT_FILE, "import { isTauriRuntime } from '@/runtime/env'\nexport { isTauriRuntime }\n")  # 复原

        # N2 presentation 反向 import 适配器（新逆边）
        _write(tmp, "easyvibe-renderer/src/main.tsx",
               "import '@/host-adapter/register'\nexport const app = 1\n")
        p = check_b3(tmp)
        results.append(("N2 console-ui import '@/host-adapter/register' → B3 必红",
                        any(x.startswith("B3") for x in p), "; ".join(p[:1])))
        _write(tmp, "easyvibe-renderer/src/main.tsx", "import './index.css'\nexport const app = 1\n")  # 复原

        # N3 index.html 归属回退 console-ui（顺序 + files 双退）
        m3 = _fixture_map(order=("console-ui", "host-adapter", "renderer-runtime"))
        for mm in m3["modules"]:
            if mm["id"] == "console-ui":
                mm["files"].append(INDEX_HTML)
        p = check_b4(m3)
        results.append(("N3 index.html 归回 console-ui → B4 必红", any(x.startswith("B4") for x in p), "; ".join(p[:1])))

        # N4 boot.tsx import 序被破坏
        _write(tmp, BOOT_FILE, "import '@/main'\nimport './register'\n")
        p = check_b3(tmp)
        results.append(("N4 boot.tsx 先装 App 后注册 → B3 必红", any(x.startswith("B3") for x in p), "; ".join(p[:1])))
        _write(tmp, BOOT_FILE, "import './register'\nimport '@/main'\n")  # 复原

        # N5 合成 order1/2 → order0 逆边
        m5 = _fixture_map(reversed_edge=True)
        p = check_b2(m5)[0]
        results.append(("N5 合成逆边 → B2 必红", any(x.startswith("B2") for x in p), "; ".join(p[:1])))

    ok = True
    for name, passed, detail in results:
        ok = ok and passed
        print("  %s %s  %s" % ("PASS" if passed else "FAIL", name, detail))
    print("N0–N5 %s" % ("全 PASS" if ok else "存在 FAIL"))
    return ok


def main():
    ap = argparse.ArgumentParser(description="宿主能力边界守卫（c-arch-5）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--map", default=DEFAULT_MAP)
    ap.add_argument("--root", default=REPO)
    args = ap.parse_args()
    root = os.path.abspath(args.root)
    if args.selfcheck:
        return 0 if selfcheck() else 1
    map_path = args.map if os.path.isabs(args.map) else os.path.join(root, args.map)
    return run_check(root, map_path)


if __name__ == "__main__":
    sys.exit(main())
