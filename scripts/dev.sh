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
# R2 清债：后端被 kill -9 时 spawn 的 agent 会变孤儿（占写互斥槽位/白烧 LLM）。
# 按 EasyVibe 特征命令行精确清理（-p --bare 组合是本产品 spawn 的签名，不影响交互式 claude）
pkill -f "claude -p --bare --dangerously-skip-permissions" 2>/dev/null || true
sleep 1

# 受管契约 env 从唯一名录（scripts/assets.json）派生，不再手写（避免清单漂移）
eval "$(python3 "$ROOT/scripts/verify_assets.py" --print-env)"

echo "▶ 启动 EasyVibe 后端 (127.0.0.1:7101, 仓库: $REPO)"
cd easyvibe-backend
EASYVIBE_REPO="$REPO" cargo run &

sleep 3
echo "▶ 启动渲染器 (http://localhost:7100)"
cd $ROOT/easyvibe-renderer
npm run dev -- --port 7100
