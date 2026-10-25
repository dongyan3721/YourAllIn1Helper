#!/usr/bin/env bash
# macOS 原生 .app 打包脚本 (Tauri 2.x + Vue 3 工程化前端)
# 自动编译现代前端、构建 Rust 微内核宿主并生成标准的 macOS Application Bundle

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

# 构建前确认固定签名可用，避免编译完才发现无证书并退回临时身份。
python3 "$ROOT_DIR/scripts/macos_signing.py" check

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
mkdir -p "$ROOT_DIR/dist" "$ROOT_DIR/.runtime_data"

# 从正式配置定位唯一产物，避免 find/head 误选目录里遗留的旧名字 App。
TAURI_BUNDLE_DIR="$ROOT_DIR/target/release/bundle/macos"
APP_NAME="$(python3 - "$ROOT_DIR/src-tauri/tauri.conf.json" <<'APP_NAME_PY'
import json
import sys
from pathlib import Path
print(json.loads(Path(sys.argv[1]).read_text())["productName"] + ".app")
APP_NAME_PY
)"
APP_SRC="$TAURI_BUNDLE_DIR/$APP_NAME"
if [ ! -d "$APP_SRC" ]; then
  echo "找不到本次构建产物：$APP_SRC" >&2
  exit 1
fi
# 先在隔离暂存目录完成签名与验证，再替换交付包；失败时保留旧 App。
STAGING_DIR="$(mktemp -d "$ROOT_DIR/.runtime_data/package.XXXXXX")"
trap 'rm -rf "$STAGING_DIR"' EXIT
cp -R "$APP_SRC" "$STAGING_DIR/$APP_NAME"
python3 "$ROOT_DIR/scripts/macos_signing.py" sign "$STAGING_DIR/$APP_NAME"
rm -rf "$ROOT_DIR/dist/$APP_NAME"
mv "$STAGING_DIR/$APP_NAME" "$ROOT_DIR/dist/$APP_NAME"
mv "$STAGING_DIR/macos-build.json" "$ROOT_DIR/dist/macos-build.json"
echo "✅ 打包成功！应用已放置在："
echo "   $ROOT_DIR/dist/$APP_NAME"
echo "=========================================================="
