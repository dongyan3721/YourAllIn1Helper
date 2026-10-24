#!/usr/bin/env bash
# macOS 原生 .app 打包脚本 (Tauri 2.x + Vue 3 工程化前端)
# 自动编译现代前端、构建 Rust 微内核宿主并生成标准的 macOS Application Bundle

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

echo "=========================================================="
echo "[1/3] 构建现代工程化前端 (Vite + Vue 3 + TypeScript)..."
echo "=========================================================="
pnpm --dir "$ROOT_DIR/src-ui" run build:fast

echo "=========================================================="
echo "[2/3] 执行 Tauri 2.x 原生桌面打包 (生成 .app 包)..."
echo "=========================================================="
"$ROOT_DIR/src-ui/node_modules/.bin/tauri" build --bundles app --no-sign

echo "=========================================================="
echo "[3/3] 整理产物至 dist/ 目录便于直接分发与性能测试..."
echo "=========================================================="
mkdir -p "$ROOT_DIR/dist"

# 查找生成的 .app 目录并同步至 dist/
TAURI_BUNDLE_DIR="$ROOT_DIR/target/release/bundle/macos"
if [ -d "$TAURI_BUNDLE_DIR" ]; then
  APP_SRC="$(find "$TAURI_BUNDLE_DIR" -name "*.app" -maxdepth 1 | head -n 1)"
  if [ -n "$APP_SRC" ]; then
    APP_NAME="$(basename "$APP_SRC")"
    rm -rf "$ROOT_DIR/dist/$APP_NAME"
    cp -R "$APP_SRC" "$ROOT_DIR/dist/$APP_NAME"
    echo "✅ 打包成功！应用已放置在："
    echo "   $ROOT_DIR/dist/$APP_NAME"
    echo ""
    echo "可通过 Finder 打开，或直接在终端运行："
    echo "   open \"$ROOT_DIR/dist/$APP_NAME\""
  fi
fi
echo "=========================================================="
