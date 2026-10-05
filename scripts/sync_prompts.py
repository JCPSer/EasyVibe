#!/usr/bin/env python3
"""按名录（scripts/assets.json）把仓库根契约文本同步到桌面壳 resources/prompts 并写 .manifest。

规则：
- 源 = 仓库根的同名文件（唯一事实源，入 git）；目标 = 桌面壳打包产物（不入 git）。
- 逐字节拷贝，不加生成标记头（生成物 == 源，sha256 必等）。
- 源缺失 = fail（不留旧副本，避免"静默用陈旧契约"）。
- 幂等：同一源连跑两次产出同字节 + 同 manifest（除 generated_at）。
"""
import argparse
import datetime
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import asset_io  # noqa: E402


def utc_now() -> str:
    return datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat()


def emit_assets(root, data: bytes):
    """回灌 hooks：R1 并集回灌 / 守卫自证用；当前无副作用。"""


def sync(root: str, assets_path: str, timestamp: str | None = None):
    root = os.path.abspath(root)
    assets = asset_io.load_assets(assets_path)
    prompts = asset_io.prompts_dir(root)
    entries = []
    for a in assets.get("text_assets", []):
        if not a.get("distribute", True):
            continue
        name = a["name"]
        src = os.path.join(root, name)
        if not os.path.isfile(src):
            raise SystemExit("[sync_prompts] 源缺失，拒绝生成陈旧副本: %s" % name)
        data = asset_io.read_bytes(src)
        asset_io.atomic_write_bytes(os.path.join(prompts, name), data)
        entries.append({
            "file": name,
            "source": name,
            "sha256": asset_io.sha256_bytes(data),
            "generated_at": timestamp or utc_now(),
        })
    manifest = {"version": 1, "generator": "scripts/sync_prompts.py", "entries": entries}
    asset_io.write_json_atomic(asset_io.manifest_path(root), manifest)
    return entries


def main() -> int:
    ap = argparse.ArgumentParser(description="同步受管契约文本到桌面壳 resources/prompts")
    ap.add_argument("--root", default=asset_io.repo_root_from_script())
    ap.add_argument("--assets", default=None)
    ap.add_argument("--timestamp", default=None, help="固定 generated_at（自检可复现用）")
    args = ap.parse_args()
    assets_path = args.assets or asset_io.default_assets_path(args.root)
    entries = sync(args.root, assets_path, args.timestamp)
    print("[sync_prompts] 已同步 %d 份 + manifest" % len(entries))
    return 0


if __name__ == "__main__":
    sys.exit(main())
