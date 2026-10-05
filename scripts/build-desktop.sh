#!/bin/bash
# 桌面端打包统一入口：codesign shim（禁用时间戳）+ tauri build
set -e
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$ROOT/scripts/codesign-shim:$PATH"
cd "$ROOT/easyvibe-desktop"
exec npx tauri build "$@"
