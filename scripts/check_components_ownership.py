#!/usr/bin/env python3
"""组件归属守卫（c-arch-3）：把「components/ 根目录不得存在 *.tsx」等归属规则做成 CI 可见判据。

范式同 scripts/check_shell_boundary.py（纯 python3、秒级、--check / --selfcheck 双入口）。
本仓 CI 明示「不跑 vitest」（见 .github/workflows/asset-guard.yml 头部），故归属规则除本地
vitest（archGuard 断言组 7）外，必须有一条 CI 可见的等价判据——即本脚本。

判据：
  C1 根目录 *.tsx 集合为空（归属锚点）
  C2 根目录不存在任何文件（只允许目录；防 .ts/.css/.json 换皮逃逸）
  C3 四落点文件到位（pages 9 页 / canvas 7 件 / shell 3 件 / overlays 4 件）
  C4 旧路径零残留（扫描渲染器源码，排除夹具快照）
  C5 地图源 parts 已同步（live `.easyvibe/` 被 gitignore，缺失则 SKIP）

模式：
  --check（默认） 对真实仓库扫描；任一不满足 → exit 1（fail-closed）
  --selfcheck     负例自证（S1 根级组件必红 / S2 纯目录不误报 / S3 缺迁移件必红 / S4 真仓库必绿）

本文件不得出现任何受管契约名 / env 名（由 scripts/verify_assets.py --forbid-literals 自守卫）。
"""
import argparse
import os
import sys

RENDERER = os.path.join("easyvibe-renderer", "src")
COMPONENTS = os.path.join(RENDERER, "components")
PARTS = os.path.join(".easyvibe", "map", "parts")

PAGES = [
    "ChangesPage.tsx", "DepsPage.tsx", "DriftPage.tsx", "GitPage.tsx", "HealthPage.tsx",
    "ModulesPage.tsx", "PlaceholderPage.tsx", "RunsPage.tsx", "UsagePage.tsx",
]
CANVAS = [
    "BandNode.tsx", "ModuleNode.tsx", "SubmoduleNode.tsx", "ExpandedModuleNode.tsx",
    "DetailPanel.tsx", "IssuesList.tsx", "TaskFormPanel.tsx",
]
SHELL = ["AppShell.tsx", "WindowControls.tsx", "ThemeToggle.tsx"]
OVERLAYS = ["OnboardingChecklist.tsx", "SessionBubble.tsx", "ViewsPanel.tsx", "WelcomePage.tsx"]

MIGRATED = [p[:-4] for p in PAGES + CANVAS + SHELL + OVERLAYS]
# 旧落点说明符形态（别名 / 全路径）；夹具快照豁免。
OLD_SPEC = "@/components/%s'"
EXEMPT = "fixtures"


def root_entries(components_dir):
    if not os.path.isdir(components_dir):
        return []
    return sorted(os.listdir(components_dir))


def root_files(components_dir):
    return [f for f in root_entries(components_dir)
            if os.path.isfile(os.path.join(components_dir, f))]


def check_layout(components_dir, renderer_dir):
    """C1–C3：返回 problems。"""
    problems = []
    tsx = [f for f in root_files(components_dir) if f.endswith(".tsx")]
    if tsx:
        problems.append("C1 components/ 根目录仍存在 *.tsx: %s" % tsx)
    files = root_files(components_dir)
    if files:
        problems.append("C2 components/ 根目录仍存在文件（应只有目录）: %s" % files)

    required = {
        "pages": (os.path.join(renderer_dir, "pages"), PAGES),
        "canvas": (os.path.join(components_dir, "canvas"), CANVAS),
        "shell": (os.path.join(components_dir, "shell"), SHELL),
        "overlays": (os.path.join(components_dir, "overlays"), OVERLAYS),
    }
    for label, (base, names) in sorted(required.items()):
        for n in names:
            if not os.path.isfile(os.path.join(base, n)):
                problems.append("C3 落点缺失 %s/%s" % (label, n))
    return problems


def scan_old_paths(renderer_dir):
    """C4：返回 [(rel, lineno, token)]；夹具路径豁免。"""
    hits = []
    for dirpath, dirnames, filenames in os.walk(renderer_dir):
        if EXEMPT in "/" + dirpath.replace(os.sep, "/"):
            dirnames[:] = []
            continue
        for fn in sorted(filenames):
            if not fn.endswith((".ts", ".tsx")):
                continue
            path = os.path.join(dirpath, fn)
            rel = os.path.relpath(path, os.path.dirname(renderer_dir)).replace(os.sep, "/")
            if EXEMPT in rel:
                continue
            try:
                with open(path, encoding="utf-8") as fh:
                    lines = fh.readlines()
            except (UnicodeDecodeError, OSError):
                continue
            for i, line in enumerate(lines, 1):
                for name in MIGRATED:
                    if (OLD_SPEC % name) in line:
                        hits.append((rel, i, name))
    return hits


def check_parts(parts_dir):
    """C5：地图源 parts 中不得再出现单段 components/<Name>.tsx 条目；缺失目录 → SKIP。"""
    if not os.path.isdir(parts_dir):
        return [], True
    bad = []
    for fn in sorted(os.listdir(parts_dir)):
        if not fn.endswith(".json"):
            continue
        path = os.path.join(parts_dir, fn)
        try:
            with open(path, encoding="utf-8") as fh:
                text = fh.read()
        except (UnicodeDecodeError, OSError):
            continue
        for name in MIGRATED:
            needle = "easyvibe-renderer/src/components/%s.tsx" % name
            if needle in text:
                bad.append((fn, name))
    return bad, False


def run(root):
    components_dir = os.path.join(root, COMPONENTS)
    renderer_dir = os.path.join(root, RENDERER)
    problems = check_layout(components_dir, renderer_dir)
    for rel, lineno, token in scan_old_paths(renderer_dir):
        problems.append("C4 旧路径残留 %s:%d → components/%s" % (rel, lineno, token))
    bad, skipped = check_parts(os.path.join(root, PARTS))
    for fn, name in bad:
        problems.append("C5 地图源未同步 %s 仍含 components/%s.tsx" % (fn, name))
    return problems, skipped


def _fixture(tmp):
    """最小 fixture：四落点齐全 + 根目录空。"""
    base = os.path.join(tmp, RENDERER)
    for sub, names in (("pages", PAGES), ("components/canvas", CANVAS),
                       ("components/shell", SHELL), ("components/overlays", OVERLAYS)):
        os.makedirs(os.path.join(base, sub), exist_ok=True)
        for n in names:
            with open(os.path.join(base, sub, n), "w", encoding="utf-8") as fh:
                fh.write("export {}\n")
    for sub in ("chat", "settings", "taskworkflow", "gate", "ui", "__tests__"):
        os.makedirs(os.path.join(base, "components", sub), exist_ok=True)
    return base


def selfcheck(real_root):
    import shutil
    import tempfile

    results = []
    with tempfile.TemporaryDirectory(prefix="components-ownership-") as tmp:
        base = _fixture(tmp)
        # S1 根目录放一个组件 → C1/C2 必红
        with open(os.path.join(base, "components", "Foo.tsx"), "w", encoding="utf-8") as fh:
            fh.write("export {}\n")
        p1, _ = run(tmp)
        results.append(("S1 根级组件 → 必红", any(x.startswith("C1") for x in p1), "; ".join(p1[:1])))
        os.remove(os.path.join(base, "components", "Foo.tsx"))
        # S2 纯目录根 → C1/C2 不误报
        p2, _ = run(tmp)
        results.append(("S2 纯目录根 → 不误报", not any(x.startswith("C1") for x in p2), "; ".join(p2[:1])))
        # S3 缺迁移件 → C3 必红
        os.remove(os.path.join(base, "pages", "GitPage.tsx"))
        p3, _ = run(tmp)
        results.append(("S3 缺 pages/GitPage.tsx → 必红", any(x.startswith("C3") for x in p3), "; ".join(p3[:1])))
        # S4 旧路径残留 → C4 必红
        with open(os.path.join(base, "App.tsx"), "w", encoding="utf-8") as fh:
            fh.write("import { GitPage } from '@/components/GitPage'\n")
        p4, _ = run(tmp)
        results.append(("S4 旧说明符残留 → 必红", any(x.startswith("C4") for x in p4), "; ".join(p4[:1])))
        shutil.rmtree(os.path.join(base, "components", "canvas"), ignore_errors=True)
    # S5 真仓库 → 必绿
    problems, skipped = run(real_root)
    detail = "; ".join(problems[:2]) if problems else ("parts 已同步" if not skipped else "parts SKIP")
    results.append(("S5 真仓库扫描 → 必绿", not problems, detail))
    ok = True
    for name, passed, detail in results:
        ok = ok and passed
        print("  %s %s  %s" % ("PASS" if passed else "FAIL", name, detail))
    return ok


def main():
    ap = argparse.ArgumentParser(description="组件归属守卫（c-arch-3）")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--root", default=os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    args = ap.parse_args()
    root = os.path.abspath(args.root)
    if args.selfcheck:
        print("▶ 组件归属守卫自检 S1–S5")
        return 0 if selfcheck(root) else 1
    problems, skipped = run(root)
    for p in problems:
        print("  FAIL %s" % p)
    print("  %s --check 问题 %d%s" % ("OK  " if not problems else "FAIL",
                                      len(problems), "（parts SKIP）" if skipped else ""))
    return 0 if not problems else 1


if __name__ == "__main__":
    sys.exit(main())
