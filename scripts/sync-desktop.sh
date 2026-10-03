#!/bin/bash
# EasyVibe 桌面壳打包前同步（beforeBuildCommand 调用）：
# 渲染器 dist + 提示词 + 后端 sidecar 二进制 → src-tauri/resources 与 binaries
# 用法: bash scripts/sync-desktop.sh   （在 easyvibe-desktop 下由 tauri 调用，亦可手动跑）
set -e
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DESKTOP="$ROOT/easyvibe-desktop"
TARGET="$(rustc -vV | sed -n 's/host: //p')"

echo "▶ 构建渲染器 dist"
(cd "$ROOT/easyvibe-renderer" && npm run build > /dev/null)

echo "▶ 同步静态资源与提示词"
rm -rf "$DESKTOP/src-tauri/resources/dist"
cp -R "$ROOT/easyvibe-renderer/dist" "$DESKTOP/src-tauri/resources/dist"
mkdir -p "$DESKTOP/src-tauri/resources/prompts"
for f in easyvibe-map-prompt-v2.2.md easyvibe-map-patrol-prompt.md easyvibe-map-schema-v1.json easyvibe-module-submap-prompt.md; do
  cp "$ROOT/$f" "$DESKTOP/src-tauri/resources/prompts/"
done

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
echo "✔ 同步完成（tauri build 即可打包 .app/.dmg/安装包）"
