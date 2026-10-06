#!/usr/bin/env python3
"""壳边界守卫（R2）：桌面壳对前端产物只有「托管」语义，不得有「符号依赖」。

扫描面（全部受版本控制）：
  easyvibe-desktop/src-tauri/src/**/*.rs
  easyvibe-desktop/src-tauri/build.rs
  easyvibe-desktop/src-tauri/tauri.conf.json

禁止命中：前端模块 id / 前端路径别名 / 前端目录或 crate 名。
允许（白名单）：资源目录路径字面量 "resources/dist"——构建期产物托管，非符号引用。

模式：
  --check（默认） 对真实仓库扫描；任一命中 → exit 1
  --selfcheck     反例自证（S1 必红 / S2 去注释不误报 / S3 必红 / S4 真仓库必绿 / S5 新模块名必红 /
                  S6 renderer-api 必红 / S7 settings-ui 必红）

本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""
import argparse
import os
import shutil
import sys
import tempfile

SCAN_FILES = [
    "easyvibe-desktop/src-tauri/build.rs",
    "easyvibe-desktop/src-tauri/tauri.conf.json",
]
SCAN_DIRS = ["easyvibe-desktop/src-tauri/src"]

FORBIDDEN = [
    "console-ui", "chat-ui", "task-ui", "map-canvas", "renderer-core",
    "renderer-runtime", "renderer-api", "renderer-shared", "ui-kit", "settings-ui",
    "easyvibe-renderer", "easyvibe_renderer",
    "@/components", "@/runtime", "@/api", "@/shared",
]
WHITELIST = ["resources/dist"]


def iter_scan_files(root):
    for rel in SCAN_FILES:
        p = os.path.join(root, rel)
        if os.path.isfile(p):
            yield rel, p
    for d in SCAN_DIRS:
        base = os.path.join(root, d)
        if not os.path.isdir(base):
            continue
        for dirpath, _dirnames, filenames in os.walk(base):
            for fn in sorted(filenames):
                p = os.path.join(dirpath, fn)
                yield os.path.relpath(p, root).replace(os.sep, "/"), p


def strip_comments(text):
    """去 Rust/JSON 注释，保留字符串字面量内容（避免把字符串内的 // 当注释）。"""
    out = []
    i, n = 0, len(text)
    state = "code"  # code | line | block | str | chr
    while i < n:
        c = text[i]
        nxt = text[i + 1] if i + 1 < n else ""
        if state == "code":
            if c == "/" and nxt == "/":
                state = "line"; i += 2; continue
            if c == "/" and nxt == "*":
                state = "block"; i += 2; continue
            if c == '"':
                state = "str"; out.append(c); i += 1; continue
            if c == "'":
                state = "chr"; out.append(c); i += 1; continue
            out.append(c); i += 1; continue
        if state == "line":
            if c == "\n":
                state = "code"; out.append(c)
            i += 1; continue
        if state == "block":
            if c == "*" and nxt == "/":
                state = "code"; i += 2; continue
            if c == "\n":
                out.append(c)
            i += 1; continue
        if state in ("str", "chr"):
            quote = '"' if state == "str" else "'"
            if c == "\\":
                out.append(c)
                if nxt:
                    out.append(nxt)
                i += 2; continue
            if c == quote:
                state = "code"
            out.append(c); i += 1; continue
    return "".join(out)


def scan(root):
    """返回 [(rel, lineno, token, line)]。"""
    hits = []
    for rel, path in iter_scan_files(root):
        try:
            with open(path, encoding="utf-8") as fh:
                raw = fh.read()
        except (UnicodeDecodeError, OSError):
            continue
        for i, line in enumerate(strip_comments(raw).splitlines(), 1):
            stripped = line
            for w in WHITELIST:
                stripped = stripped.replace(w, " ")
            for t in FORBIDDEN:
                if t in stripped:
                    hits.append((rel, i, t, line.strip()))
    return hits


def _fixture(root, tmp):
    """最小 fixture：复刻扫描面布局。"""
    os.makedirs(os.path.join(tmp, "easyvibe-desktop/src-tauri/src"), exist_ok=True)
    with open(os.path.join(tmp, "easyvibe-desktop/src-tauri/src/lib.rs"), "w", encoding="utf-8") as fh:
        fh.write('fn run() { let d = res.join("resources/dist"); }\n')
    with open(os.path.join(tmp, "easyvibe-desktop/src-tauri/build.rs"), "w", encoding="utf-8") as fh:
        fh.write("fn main() {}\n")
    with open(os.path.join(tmp, "easyvibe-desktop/src-tauri/tauri.conf.json"), "w", encoding="utf-8") as fh:
        fh.write('{ "build": { "frontendDist": "../dist" }, "bundle": { "resources": [ "resources/dist/" ] } }\n')


def selfcheck(real_root):
    results = []
    with tempfile.TemporaryDirectory(prefix="shell-boundary-") as tmp:
        _fixture(real_root, tmp)
        # S1 注入真符号引用 → 必红
        lib = os.path.join(tmp, "easyvibe-desktop/src-tauri/src/lib.rs")
        base = open(lib, encoding="utf-8").read()
        with open(lib, "w", encoding="utf-8") as fh:
            fh.write(base + "use easyvibe_renderer::x;\n")
        h1 = [x for x in scan(tmp) if x[0].endswith("lib.rs")]
        results.append(("S1 注入 crate 符号引用 → 必红", len(h1) > 0, "; ".join(x[2] for x in h1[:2])))
        # S2 注释里的模块名 → 去注释后不误报
        with open(lib, "w", encoding="utf-8") as fh:
            fh.write(base + "// import from 'console-ui' 内部 UI\n")
        h2 = [x for x in scan(tmp) if x[0].endswith("lib.rs")]
        results.append(("S2 注释含模块名 → 去注释不误报", len(h2) == 0, "; ".join(x[2] for x in h2[:2])))
        # S3 tauri.conf 注入模块名 → 必红
        conf = os.path.join(tmp, "easyvibe-desktop/src-tauri/tauri.conf.json")
        with open(conf, "w", encoding="utf-8") as fh:
            fh.write('{ "bundle": { "resources": [ "resources/dist/" ], "note": "console-ui" } }\n')
        h3 = [x for x in scan(tmp) if x[0].endswith("tauri.conf.json")]
        results.append(("S3 tauri.conf 注入模块名 → 必红", len(h3) > 0, "; ".join(x[2] for x in h3[:2])))
        # S5 注入新拆模块名 → 必红（R-F：重命名后不得绕过壳边界）
        with open(lib, "w", encoding="utf-8") as fh:
            fh.write(base + 'let _ = "renderer-runtime";\n')
        h5 = [x for x in scan(tmp) if x[0].endswith("lib.rs")]
        results.append(("S5 注入新模块名 → 必红", len(h5) > 0, "; ".join(x[2] for x in h5[:2])))
        # S6 注入 renderer-api（api 析出为独立模块）→ 必红
        with open(lib, "w", encoding="utf-8") as fh:
            fh.write(base + 'let _ = "renderer-api";\n')
        h6 = [x for x in scan(tmp) if x[0].endswith("lib.rs")]
        results.append(("S6 注入 renderer-api → 必红", len(h6) > 0, "; ".join(x[2] for x in h6[:2])))
        # S7 注入 settings-ui（settings 域析出为独立模块）→ 必红
        with open(lib, "w", encoding="utf-8") as fh:
            fh.write(base + 'let _ = "settings-ui";\n')
        h7 = [x for x in scan(tmp) if x[0].endswith("lib.rs")]
        results.append(("S7 注入 settings-ui → 必红", len(h7) > 0, "; ".join(x[2] for x in h7[:2])))
        with open(lib, "w", encoding="utf-8") as fh:
            fh.write(base)
        # S4 真仓库 → 必绿（仅白名单路径）
        h4 = scan(real_root)
        results.append(("S4 真仓库扫描 → 必绿", len(h4) == 0, "; ".join("%s:%d %s" % (a, b, c) for a, b, c, _ in h4[:3])))
    ok = True
    for name, passed, detail in results:
        ok = ok and passed
        print("  %s %s  %s" % ("PASS" if passed else "FAIL", name, detail))
    return ok


def main():
    ap = argparse.ArgumentParser(description="壳边界守卫（R2）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--root", default=os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    args = ap.parse_args()
    root = os.path.abspath(args.root)
    if args.selfcheck:
        print("▶ 壳边界守卫自检 S1–S7")
        return 0 if selfcheck(root) else 1
    hits = scan(root)
    for rel, lineno, token, line in hits:
        print("  HIT  %s:%d → %s" % (rel, lineno, token))
    print("  %s --check 命中 %d" % ("OK  " if not hits else "FAIL", len(hits)))
    return 0 if not hits else 1


if __name__ == "__main__":
    sys.exit(main())
