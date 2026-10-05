#!/usr/bin/env python3
"""按名录（scripts/assets.json）从唯一源生成地图 CLI 副本（确定、幂等、逐字节）。

源 = run/ 下的 CLI（入 git、可 review）；生成物 = 自托管运行目录副本（不入 git）。
源缺失 = fail。生成物是源的纯函数：连跑两次 sha256 不变。
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import asset_io  # noqa: E402


def generate(root: str, assets_path: str):
    root = os.path.abspath(root)
    assets = asset_io.load_assets(assets_path)
    out = []
    for a in assets.get("cli_assets", []):
        src = os.path.join(root, a["source"])
        if not os.path.isfile(src):
            raise SystemExit("[gen_map_cli] 源缺失，拒绝生成陈旧副本: %s" % a["source"])
        data = asset_io.read_bytes(src)
        dst = os.path.join(root, a["generated"])
        asset_io.atomic_write_bytes(dst, data)
        out.append((a["generated"], asset_io.sha256_bytes(data)))
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description="从唯一源生成地图 CLI 副本")
    ap.add_argument("--root", default=asset_io.repo_root_from_script())
    ap.add_argument("--assets", default=None)
    args = ap.parse_args()
    assets_path = args.assets or asset_io.default_assets_path(args.root)
    for path, digest in generate(args.root, assets_path):
        print("[gen_map_cli] %s  sha256=%s" % (path, digest[:12]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
