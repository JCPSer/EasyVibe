#!/bin/bash
# EasyVibe 桌面壳打包前同步（tauri beforeBuildCommand 调用）：
# 渲染器 dist + 受管契约文本（按 scripts/assets.json 名录）+ 后端 sidecar 二进制 → resources 与 binaries
# 用法: bash scripts/sync-desktop.sh   （在 easyvibe-desktop 下由 tauri 调用，亦可手动跑）
#
# R5′ 双入口之一（默认入口，自愈）：无旧 manifest 时**不 fail**（视为首次构建 →
# 全新 checkout 的 tag 打包不被阻断）；生成后跑 verify_assets.py --check（源↔副本 sha256），
# 真实漂移才 fail。严格门禁（无 manifest = fail）在独立入口 --verify-manifest / --prebuild，
# 不在 beforeBuildCommand 内（否则全新 runner 无 manifest 会禁掉打包）。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DESKTOP="$ROOT/easyvibe-desktop"
TARGET="$(rustc -vV | sed -n 's/host: //p')"

echo "▶ 构建渲染器 dist"
(cd "$ROOT/easyvibe-renderer" && npm run build > /dev/null)

echo "▶ 同步静态资源"
rm -rf "$DESKTOP/src-tauri/resources/dist"
cp -R "$ROOT/easyvibe-renderer/dist" "$DESKTOP/src-tauri/resources/dist"

echo "▶ 同步受管契约文本（按名录，含 manifest）"
python3 "$ROOT/scripts/sync_prompts.py" --root "$ROOT"

echo "▶ 构建后端 sidecar（release，$TARGET）"
(cd "$ROOT/easyvibe-backend" && cargo build -p easyvibe-app --release > /dev/null)

echo "▶ 复制 sidecar 二进制"
mkdir -p "$DESKTOP/src-tauri/binaries"
# Windows 目标必须带 .exe 后缀（tauri externalBin 约定），否则壳找不到 sidecar
EXT=""
case "$TARGET" in
  *windows*) EXT=".exe" ;;
esac
cp "$ROOT/easyvibe-backend/target/release/easyvibe-backend$EXT" \
   "$DESKTOP/src-tauri/binaries/easyvibe-backend-$TARGET$EXT"

echo "▶ 产物一致性校验（源↔副本 sha256；漂移即 fail-closed）"
python3 "$ROOT/scripts/verify_assets.py" --check
echo "✔ 同步完成（tauri build 即可打包 .app/.dmg/安装包）"
