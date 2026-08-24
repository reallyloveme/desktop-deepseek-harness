#!/usr/bin/env bash
# 下载便携 Node 运行时并安装 @deepseek-ai/dsh 到 resources/
# 用法: ./scripts/prepare_runtime.sh [NodeVersion] [DshVersion]
# 注意: dsh 需要 Node >= 23（依赖 node:zlib 的 zstd 实验 API），请勿降低版本
set -euo pipefail

NODE_VERSION="${1:-v23.11.1}"
DSH_VERSION="${2:-0.1.1-rc.2}"   # dsh 为 developer preview，锁定精确版本
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RES_DIR="$ROOT/resources"
NODE_DIR="$RES_DIR/node"
DSH_DIR="$RES_DIR/dsh"

# 清除可能的 shim 注入（如 NODE_OPTIONS），避免 npm/pnpm 异常
unset NODE_OPTIONS

mkdir -p "$RES_DIR"

ARCH="$(uname -m)"
OS="$(uname -s)"
case "$OS" in
  Darwin) NODE_PLATFORM="darwin"; NODE_ARCH="$( [ "$ARCH" = "arm64" ] && echo arm64 || echo x64 )" ;;
  Linux)  NODE_PLATFORM="linux"; NODE_ARCH="$( [ "$ARCH" = "aarch64" ] && echo arm64 || echo x64 )" ;;
  *) echo "Unsupported OS: $OS" >&2; exit 1 ;;
esac

NODE_BIN="$NODE_DIR/bin/node"
VERSION_MARK="$NODE_DIR/.node-version"
NEED_NODE=true
if [ -x "$NODE_BIN" ] && [ -f "$VERSION_MARK" ]; then
  if [ "$(cat "$VERSION_MARK")" = "$NODE_VERSION" ]; then
    NEED_NODE=false
  fi
fi

if $NEED_NODE; then
  echo "==> 下载便携 Node $NODE_VERSION ($NODE_PLATFORM-$NODE_ARCH)"
  rm -rf "$NODE_DIR"
  TARBALL="node-$NODE_VERSION-$NODE_PLATFORM-$NODE_ARCH.tar.xz"
  curl -fsSL "https://nodejs.org/dist/$NODE_VERSION/$TARBALL" -o "$RES_DIR/$TARBALL"
  mkdir -p "$NODE_DIR"
  tar -xJf "$RES_DIR/$TARBALL" -C "$NODE_DIR" --strip-components=1
  rm -f "$RES_DIR/$TARBALL"
  printf '%s' "$NODE_VERSION" > "$VERSION_MARK"
else
  echo "==> 便携 Node 已存在 ($NODE_VERSION)，跳过下载"
fi

echo "==> 安装 @deepseek-ai/dsh@$DSH_VERSION 到 resources/dsh"
mkdir -p "$DSH_DIR"
PKG="@deepseek-ai/dsh"
[ -n "$DSH_VERSION" ] && PKG="@deepseek-ai/dsh@$DSH_VERSION"

# 优先使用 pnpm（hoisted 布局，规避部分文件系统/符号链接问题）；否则回退 npm
if command -v pnpm >/dev/null 2>&1; then
  echo "==> 使用 pnpm (hoisted) 安装"
  pnpm install --prefix "$DSH_DIR" --config.node-linker=hoisted --no-frozen-lockfile "$PKG"
else
  NPM_BIN="$NODE_DIR/bin/npm"
  echo "==> 使用 npm 安装"
  NODE_OPTIONS="--max-old-space-size=8192" "$NPM_BIN" install --prefix "$DSH_DIR" --no-fund --no-audit "$PKG"
fi

echo "==> 完成"
echo "  Node: $NODE_BIN"
echo "  dsh : $DSH_DIR/node_modules"
