#!/usr/bin/env bash
# 一条命令本地启动：构建前端 + 启动后端（后端托管前端静态资源）
set -euo pipefail
cd "$(dirname "$0")"

export PATH="$HOME/.cargo/bin:$PATH"

if [ ! -d frontend/node_modules ]; then
  echo "==> 安装前端依赖"
  (cd frontend && npm install)
fi

echo "==> 构建前端"
(cd frontend && npm run build)

echo "==> 启动服务"
exec cargo run
