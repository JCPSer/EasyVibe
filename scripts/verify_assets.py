#!/usr/bin/env python3
"""产物副本一致性守卫（唯一实现，跨平台 python3）。

分层（方案 R4）：
  L1 字节层：入 git 的源 ↔ 不入 git 的副本（run/ ↔ 自托管副本；仓库根 ↔ 桌面壳 resources）
            副本不存在 = SKIP（CI 全新 checkout 场景，转 L2）
  L2 名录层：名录 ↔ 源码文本（--forbid-literals + 存在性），在全新 checkout 上也成立

模式：
  --check（默认）    L1 + L2；任一漂移 → exit 1
  --regen            生成（CLI 副本 + prompts + manifest）后再 --check；幂等
  --verify-manifest  严格门禁：manifest 必须存在且逐条 sha256 == 当前源；缺 → exit 1
  --prebuild         --verify-manifest 的语义别名
  --forbid-literals  只跑 L2 字面量扫描（受管名 + env 名 + 陈旧名；含注释）
  --selfcheck        T1–T4 自检（生成自愈 / 漂移必红 / 无 manifest 必红 / 同步后必绿）
  --print-env        按名录输出 shell export（dev.sh 派生 env 用）

本文件不得出现任何受管契约名 / env 名（由 --forbid-literals 自守卫）。
"""
import argparse
import json
import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import asset_io  # noqa: E402
import gen_map_cli  # noqa: E402
import sync_prompts  # noqa: E402

SCAN_DIRS = ["scripts", "easyvibe-desktop/src-tauri/src", "easyvibe-backend/crates"]
EXCLUDE_SKIP_PARTS = ("/target/", "/node_modules/")
# 排除面：名录本体（受管名/env 名的唯一合法落点）
EXCLUDE_FILES = ("scripts/assets.json",)


def _rel(root, path):
    return os.path.relpath(path, root).replace(os.sep, "/")


def scan_files(root):
    for d in SCAN_DIRS:
        base = os.path.join(root, d)
        if not os.path.isdir(base):
            continue
        for dirpath, dirnames, filenames in os.walk(base):
            rel_dir = "/" + _rel(root, dirpath) + "/"
            if any(p in rel_dir for p in EXCLUDE_SKIP_PARTS):
                dirnames[:] = []
                continue
            for fn in filenames:
                path = os.path.join(dirpath, fn)
                rel = _rel(root, path)
                if rel in EXCLUDE_FILES:
                    continue
                yield rel, path


def forbid_literals(root, assets):
    """返回 [(rel, lineno, target, line)]；交付态期望空。"""
    targets = sorted(set(asset_io.managed_names(assets))
                     | set(asset_io.managed_envs(assets))
                     | set(assets.get("stale_names", [])))
    hits = []
    for rel, path in scan_files(root):
        try:
            with open(path, encoding="utf-8") as fh:
                lines = fh.readlines()
        except (UnicodeDecodeError, OSError):
            continue
        for i, line in enumerate(lines, 1):
            for t in targets:
                if t in line:
                    hits.append((rel, i, t, line.rstrip("\n")))
    return hits


def check_l1(root, assets):
    """L1 字节层。返回 (errors, skips)。"""
    errors, skips = [], []
    for a in assets.get("cli_assets", []):
        src = os.path.join(root, a["source"])
        gen = os.path.join(root, a["generated"])
        if not os.path.exists(gen):
            skips.append("L1-A 副本不存在，跳过: %s" % a["generated"])
            continue
        if not os.path.exists(src):
            errors.append("L1-A 源缺失但副本存在: %s" % a["source"])
            continue
        if asset_io.sha256_bytes(asset_io.read_bytes(src)) != asset_io.sha256_bytes(asset_io.read_bytes(gen)):
            errors.append("L1-A 源↔副本漂移: %s vs %s" % (a["source"], a["generated"]))
    for a in assets.get("text_assets", []):
        if not a.get("distribute", True):
            continue
        src = os.path.join(root, a["name"])
        dst = os.path.join(asset_io.prompts_dir(root), a["name"])
        if not os.path.exists(dst):
            skips.append("L1-B 副本不存在，跳过: resources/prompts/%s" % a["name"])
            continue
        if not os.path.exists(src):
            errors.append("L1-B 源缺失但副本存在: %s" % a["name"])
            continue
        if asset_io.sha256_bytes(asset_io.read_bytes(src)) != asset_io.sha256_bytes(asset_io.read_bytes(dst)):
            errors.append("L1-B 源↔副本漂移: %s vs resources/prompts/%s" % (a["name"], a["name"]))
    return errors, skips


def check_l2_names(root, assets):
    """L2 存在性：名录每个受管名必须在仓库根存在；名录自身无重名/无空。"""
    errors = []
    names = asset_io.managed_names(assets)
    envs = asset_io.managed_envs(assets)
    if len(set(names)) != len(names):
        errors.append("L2 名录 text_assets.name 有重复")
    if len(set(envs)) != len(envs):
        errors.append("L2 名录 text_assets.env 有重复")
    if len(names) != len(assets.get("text_assets", [])):
        errors.append("L2 名录条目缺 name")
    for n in names:
        if not os.path.isfile(os.path.join(root, n)):
            errors.append("L2 受管文件在仓库根不存在: %s" % n)
    return errors


def run_check(root, assets, verbose=True):
    errors, skips = check_l1(root, assets)
    errors += check_l2_names(root, assets)
    literals = forbid_literals(root, assets)
    for rel, lineno, target, line in literals:
        errors.append("L2 --forbid-literals 命中 %s:%d → %s" % (rel, lineno, target))
    if verbose:
        for s in skips:
            print("  SKIP " + s)
        if not errors:
            print("  OK   L1 字节层 + L2 名录层 + --forbid-literals 全部通过")
    return errors


def verify_manifest(root, assets):
    """严格门禁：manifest 必须存在；每条 sha256 == 当前源；无多余/缺失。"""
    errors = []
    mp = asset_io.manifest_path(root)
    if not os.path.exists(mp):
        return ["严格门禁：manifest 不存在（%s）" % _rel(root, mp)]
    try:
        manifest = json.load(open(mp, encoding="utf-8"))
    except Exception as exc:  # noqa: BLE001
        return ["严格门禁：manifest 不可解析（%s）" % exc]
    entries = manifest.get("entries", [])
    expected = [a for a in assets.get("text_assets", []) if a.get("distribute", True)]
    if len(entries) != len(expected):
        errors.append("严格门禁：manifest 条数 %d != 名录分发数 %d" % (len(entries), len(expected)))
    seen = set()
    for e in entries:
        name = e.get("file")
        seen.add(name)
        src = os.path.join(root, e.get("source", name))
        dst = os.path.join(asset_io.prompts_dir(root), name)
        if not os.path.isfile(src):
            errors.append("严格门禁：manifest.source 缺失 %s" % name)
            continue
        if not os.path.isfile(dst):
            errors.append("严格门禁：副本缺失 resources/prompts/%s" % name)
            continue
        if asset_io.sha256_bytes(asset_io.read_bytes(src)) != e.get("sha256"):
            errors.append("严格门禁：树非当前源生成（sha256 不符）%s" % name)
    for a in expected:
        if a["name"] not in seen:
            errors.append("严格门禁：manifest 缺条目 %s" % a["name"])
    return errors


def do_regen(root, assets_path):
    gen_map_cli.generate(root, assets_path)
    sync_prompts.sync(root, assets_path)
    return run_check(root, asset_io.load_assets(assets_path))


def print_env(root, assets):
    for a in assets.get("text_assets", []):
        if not a.get("env"):
            continue
        print('export %s="%s"' % (a["env"], os.path.join(root, a["name"])))
    return 0


# ------------------------------------------------------------------ selfcheck
def _fixture(real_root, assets_path, tmp):
    """构造"全新 checkout"最小 fixture：名录 + 5 份根文件，无 resources / 无 .easyvibe。"""
    os.makedirs(os.path.join(tmp, "scripts"), exist_ok=True)
    assets = asset_io.load_assets(assets_path)
    dst_assets = os.path.join(tmp, "scripts", "assets.json")
    shutil.copyfile(assets_path, dst_assets)
    for n in asset_io.managed_names(assets):
        shutil.copyfile(os.path.join(real_root, n), os.path.join(tmp, n))
    return dst_assets, assets


def selfcheck(real_root, assets_path):
    results = []
    with tempfile.TemporaryDirectory(prefix="asset-selfcheck-") as tmp:
        fx_assets_path, assets = _fixture(real_root, assets_path, tmp)

        # T1 全新 checkout（无 manifest）走默认生成路径 → exit 0 且产出 5 份 + manifest
        try:
            entries = sync_prompts.sync(tmp, fx_assets_path, timestamp="1970-01-01T00:00:00+00:00")
            n_files = len([a for a in assets["text_assets"] if a.get("distribute", True)])
            have = all(os.path.isfile(os.path.join(asset_io.prompts_dir(tmp), a["name"]))
                       for a in assets["text_assets"] if a.get("distribute", True))
            t1_ok = len(entries) == n_files and have and os.path.exists(asset_io.manifest_path(tmp))
            results.append(("T1 无 manifest 默认入口自愈生成", t1_ok,
                            "entries=%d/%d have_all=%s" % (len(entries), n_files, have)))
        except SystemExit as exc:
            results.append(("T1 无 manifest 默认入口自愈生成", False, str(exc)))

        # T2 漂移必红：改副本 1 字节 → --check 非零
        first = [a for a in assets["text_assets"] if a.get("distribute", True)][0]["name"]
        victim = os.path.join(asset_io.prompts_dir(tmp), first)
        with open(victim, "ab") as fh:
            fh.write(b"x")
        errs_t2 = run_check(tmp, assets, verbose=False)
        results.append(("T2 副本漂移必红", len(errs_t2) > 0, "; ".join(errs_t2[:2])))

        # 复位（重新生成 → 消漂移）
        sync_prompts.sync(tmp, fx_assets_path, timestamp="1970-01-01T00:00:00+00:00")

        # T3 无 manifest 严格门禁必红
        os.unlink(asset_io.manifest_path(tmp))
        errs_t3 = verify_manifest(tmp, assets)
        results.append(("T3 无 manifest 严格门禁必红", len(errs_t3) > 0, "; ".join(errs_t3[:1])))

        # T4 同步后严格门禁必绿
        sync_prompts.sync(tmp, fx_assets_path, timestamp="1970-01-01T00:00:00+00:00")
        errs_t4 = verify_manifest(tmp, assets)
        results.append(("T4 同步后严格门禁必绿", len(errs_t4) == 0, "; ".join(errs_t4[:2])))

    ok = True
    for name, passed, detail in results:
        ok = ok and passed
        print("  %s %s  %s" % ("PASS" if passed else "FAIL", name, detail))
    return ok, results


def main() -> int:
    ap = argparse.ArgumentParser(description="产物副本一致性守卫")
    ap.add_argument("--root", default=asset_io.repo_root_from_script())
    ap.add_argument("--assets", default=None)
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--regen", action="store_true")
    ap.add_argument("--verify-manifest", action="store_true")
    ap.add_argument("--prebuild", action="store_true")
    ap.add_argument("--forbid-literals", action="store_true")
    ap.add_argument("--selfcheck", action="store_true")
    ap.add_argument("--print-env", action="store_true")
    args = ap.parse_args()

    root = os.path.abspath(args.root)
    assets_path = args.assets or asset_io.default_assets_path(root)
    assets = asset_io.load_assets(assets_path)

    if args.print_env:
        return print_env(root, assets)
    if args.selfcheck:
        print("▶ 守卫自检 T1–T4")
        ok, _ = selfcheck(root, assets_path)
        return 0 if ok else 1
    if args.regen:
        print("▶ 生成（CLI 副本 + prompts + manifest）")
        errors = do_regen(root, assets_path)
        for e in errors:
            print("  FAIL " + e)
        return 0 if not errors else 1
    if args.verify_manifest or args.prebuild:
        errors = verify_manifest(root, assets)
        for e in errors:
            print("  FAIL " + e)
        if not errors:
            print("  OK   严格门禁：manifest 存在且树由当前源生成")
        return 0 if not errors else 1
    if args.forbid_literals:
        hits = forbid_literals(root, assets)
        for rel, lineno, target, line in hits:
            print("  HIT  %s:%d → %s" % (rel, lineno, target))
        print("  %s --forbid-literals 命中 %d" % ("OK  " if not hits else "FAIL", len(hits)))
        return 0 if not hits else 1

    # 默认 = --check
    print("▶ 产物一致性检查（L1 字节层 + L2 名录层）")
    errors = run_check(root, assets)
    for e in errors:
        print("  FAIL " + e)
    return 0 if not errors else 1


if __name__ == "__main__":
    sys.exit(main())
