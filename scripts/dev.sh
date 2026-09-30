#!/bin/bash
# EasyVibe 开发环境一键启动：后端(7101) + 渲染器(7100)
# 用法: ./scripts/dev.sh [仓库路径，默认 hover-client]
set -e
cd "$(dirname "$0")/.."
ROOT=$PWD
REPO=${1:-/Users/liyuhang/Documents/git_projects/language-band/hover-client}

# 清理占端口的旧进程（曾导致新后端绑定失败、页面连到旧仓库）
lsof -ti :7101 | xargs kill -9 2>/dev/null || true
lsof -ti :7100 | xargs kill -9 2>/dev/null || true
sleep 1

echo "▶ 启动 EasyVibe 后端 (127.0.0.1:7101, 仓库: $REPO)"
cd easyvibe-backend
EASYVIBE_REPO="$REPO" \
EASYVIBE_PROMPT_PATH=$ROOT/easyvibe-map-prompt-v2.2.md \
EASYVIBE_PATROL_PROMPT_PATH=$ROOT/easyvibe-map-patrol-prompt.md \
EASYVIBE_SCHEMA_PATH=$ROOT/easyvibe-map-schema-v1.json \
cargo run &

sleep 3
echo "▶ 启动渲染器 (http://localhost:7100)"
cd $ROOT/easyvibe-renderer
npm run dev -- --port 7100
