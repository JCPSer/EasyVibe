#!/usr/bin/env python3
"""受管资产的共享 IO 工具：名录装载、原子写、sha256。

被 sync_prompts.py / gen_map_cli.py / verify_assets.py 复用，避免重复实现（R4「一处实现」）。
本文件不得出现任何受管契约名 / env 名（由 verify_assets.py --forbid-literals 守卫）。
"""
import hashlib
import json
import os
import tempfile


def repo_root_from_script() -> str:
    """scripts/ 的父目录即仓库根（脚本只经相对自身定位，不依赖 cwd）。"""
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def default_assets_path(root: str) -> str:
    return os.path.join(root, "scripts", "assets.json")


def load_assets(assets_path: str) -> dict:
    with open(assets_path, encoding="utf-8") as fh:
        return json.load(fh)


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_bytes(path: str) -> bytes:
    with open(path, "rb") as fh:
        return fh.read()


def atomic_write_bytes(path: str, data: bytes) -> None:
    """同目录临时文件 + os.replace，LF 字节原样写（不做任何换行变换）。"""
    d = os.path.dirname(path)
    if d:
        os.makedirs(d, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=".tmp-", dir=d or ".")
    try:
        with os.fdopen(fd, "wb") as fh:
            fh.write(data)
            fh.flush()
            os.fsync(fh.fileno())
        # mkstemp 默认 0600；生成物按常规 0644（与既有 cp 行为一致；Windows 上等价于可写位）
        try:
            os.chmod(tmp, 0o644)
        except OSError:
            pass
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def write_json_atomic(path: str, obj) -> None:
    text = json.dumps(obj, ensure_ascii=False, indent=2) + "\n"
    atomic_write_bytes(path, text.encode("utf-8"))


def managed_names(assets: dict):
    return [a["name"] for a in assets.get("text_assets", [])]


def managed_envs(assets: dict):
    return [a["env"] for a in assets.get("text_assets", []) if a.get("env")]


def manifest_path(root: str) -> str:
    return os.path.join(root, "easyvibe-desktop", "src-tauri", "resources", "prompts", ".manifest")


def prompts_dir(root: str) -> str:
    return os.path.join(root, "easyvibe-desktop", "src-tauri", "resources", "prompts")
