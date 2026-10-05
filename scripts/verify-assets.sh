#!/bin/bash
# 守卫入口 shim：逻辑唯一实现在 scripts/verify_assets.py（跨平台 python3 hashlib）。
# 用法: bash scripts/verify-assets.sh [--check|--regen|--verify-manifest|--prebuild|--forbid-literals|--selfcheck|--print-env]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec python3 "$ROOT/scripts/verify_assets.py" "$@"
