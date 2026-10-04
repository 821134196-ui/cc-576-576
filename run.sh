#!/usr/bin/env bash
# 一条命令在本地启动「共享办公访客网络临时码系统」。
# 用法：./run.sh                          构建前端并启动后端
#       BIND_ADDR=0.0.0.0:9000 ./run.sh   自定义监听地址
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
export BIND_ADDR="${BIND_ADDR:-127.0.0.1:8787}"

echo "==> [1/3] 检查工具链"
command -v cargo >/dev/null || { echo "未找到 cargo，请先安装 Rust: https://rustup.rs"; exit 1; }
command -v node  >/dev/null || { echo "未找到 node（需要 Node 18+）"; exit 1; }

echo "==> [2/3] 构建 Svelte 前端"
cd "$ROOT/frontend"
[ -d node_modules ] || npm install
npm run build

echo "==> [3/3] 启动 Axum 服务（首次启动自动迁移 SQLite）"
cd "$ROOT/backend"
export STATIC_DIR="$ROOT/frontend/dist"
export GUESTNET_DB="${GUESTNET_DB:-$ROOT/backend/guestnet.db}"

echo "------------------------------------------------------------"
echo " 前台 / 网管控制台:  http://$BIND_ADDR"
echo " 基础数据检查:       curl http://$BIND_ADDR/api/meta"
echo " 按 Ctrl-C 停止"
echo "------------------------------------------------------------"
exec cargo run
