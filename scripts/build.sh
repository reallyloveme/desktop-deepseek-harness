#!/usr/bin/env bash
# 一键构建：准备运行时 -> 编译前端 -> Tauri bundle
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SKIP_PREPARE="${1:-}"

if [ "$SKIP_PREPARE" != "skip-prepare" ]; then
  bash "$ROOT/scripts/prepare_runtime.sh"
fi

cd "$ROOT"
echo "==> 编译前端"
npm run build

echo "==> Tauri bundle"
npm run tauri -- build

echo "==> 构建完成"
