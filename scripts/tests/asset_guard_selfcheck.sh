#!/bin/bash
# 守卫自检 T1–T4（秒级、纯 python3、不触发 npm/cargo）：
#   T1 无 manifest 走默认生成入口成功（= tag 打包路径不被「缺 manifest」阻断）
#   T2 副本漂移必红（--check 非零）
#   T3 无 manifest 严格门禁必红（--verify-manifest 非零）
#   T4 同步后严格门禁必绿
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
exec python3 "$ROOT/scripts/verify_assets.py" --selfcheck
