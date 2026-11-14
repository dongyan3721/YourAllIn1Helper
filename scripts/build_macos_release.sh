#!/usr/bin/env bash
# macOS release build: code-sign the .app first, then create and minisign the updater tarball.
# The updater archive must contain the final signed bundle so TCC grants survive replacement.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

SIGNING_KEY="${TAURI_SIGNING_PRIVATE_KEY_PATH:-$HOME/Library/Application Support/YourAllIn1Helper/release-keys/updater.key}"
NOTES_FILE="${1:-$ROOT_DIR/docs/releases/0.1.3-beta.md}"

if [ ! -f "$SIGNING_KEY" ]; then
  echo "未找到 updater 私钥：$SIGNING_KEY" >&2
  echo "可通过 TAURI_SIGNING_PRIVATE_KEY_PATH 指定；私钥不得进入仓库。" >&2
  exit 1
fi
if [ ! -f "$NOTES_FILE" ]; then
  echo "未找到发布说明：$NOTES_FILE" >&2
  exit 1
fi

python3 "$ROOT_DIR/scripts/macos_signing.py" check

echo "[1/5] 安装并检查前端..."
pnpm --dir "$ROOT_DIR/src-ui" install --frozen-lockfile
pnpm --dir "$ROOT_DIR/src-ui" exec vue-tsc --noEmit

echo "[2/5] 构建未签名 .app..."
"$ROOT_DIR/src-ui/node_modules/.bin/tauri" build --bundles app --ci --no-sign \
  --config '{"bundle":{"createUpdaterArtifacts":false}}'

APP_DIR="$ROOT_DIR/target/release/bundle/macos"
APP_NAME="$(python3 - "$ROOT_DIR/src-tauri/tauri.conf.json" <<'PY'
import json
import sys
from pathlib import Path
print(json.loads(Path(sys.argv[1]).read_text())["productName"] + ".app")
PY
)"
APP_SRC="$APP_DIR/$APP_NAME"
if [ ! -d "$APP_SRC" ]; then
  echo "找不到构建产物：$APP_SRC" >&2
  exit 1
fi

echo "[3/5] 固定证书签名并验证..."
python3 "$ROOT_DIR/scripts/macos_signing.py" sign "$APP_SRC"

VERSION="$(python3 - "$ROOT_DIR/src-tauri/tauri.conf.json" <<'PY'
import json
import sys
from pathlib import Path
print(json.loads(Path(sys.argv[1]).read_text())["version"])
PY
)"
case "$(uname -m)" in
  arm64) ARCH="aarch64" ;;
  x86_64) ARCH="x86_64" ;;
  *) echo "不支持的构建架构：$(uname -m)" >&2; exit 1 ;;
esac

ARTIFACT_NAME="All-in-One-Toolbox_${VERSION}_macos_${ARCH}.app.tar.gz"
echo "[4/5] 打包并签名 updater 产物：$ARTIFACT_NAME"
tar -czf "$APP_DIR/$ARTIFACT_NAME" -C "$APP_DIR" "$APP_NAME"
"$ROOT_DIR/src-ui/node_modules/.bin/tauri" signer sign \
  --private-key-path "$SIGNING_KEY" \
  --app-version "$VERSION" \
  "$APP_DIR/$ARTIFACT_NAME"

echo "[5/5] 更新 feed 与 dist 发布目录..."
python3 - "$APP_DIR/$ARTIFACT_NAME" "$NOTES_FILE" "$VERSION" "$ARCH" "$ROOT_DIR" <<'PY'
import datetime as dt
import hashlib
import json
import shutil
import sys
from pathlib import Path

artifact = Path(sys.argv[1])
notes_file = Path(sys.argv[2])
version = sys.argv[3]
arch = sys.argv[4]
root = Path(sys.argv[5])
repo = "dongyan3721/YourAllIn1Helper"
tag = "v" + version
target = f"darwin-{arch}"
signature = Path(str(artifact) + ".sig")
if not artifact.is_file() or not signature.is_file():
    raise SystemExit("app.tar.gz and its .sig must both exist")

feed_path = root / "updates" / "macos-beta.json"
platforms = {}
if feed_path.is_file():
    existing = json.loads(feed_path.read_text(encoding="utf-8"))
    # 同一版本可跨架构合并；换版本后旧架构附件必须重新构建，不能沿用。
    if existing.get("version") == version:
        platforms = existing.get("platforms", {})

platforms[target] = {
    "url": f"https://github.com/{repo}/releases/download/{tag}/{artifact.name}",
    "signature": signature.read_text().strip(),
}
manifest = {
    "version": version,
    "notes": notes_file.read_text(encoding="utf-8"),
    "pub_date": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
    "platforms": platforms,
}
serialized = json.dumps(manifest, ensure_ascii=False, indent=2) + "\n"
feed_path.write_text(serialized, encoding="utf-8")

output = root / "dist" / tag
output.mkdir(parents=True, exist_ok=True)
for src in (artifact, signature):
    shutil.copy2(src, output / src.name)
build_manifest = artifact.parent / "macos-build.json"
if build_manifest.is_file():
    shutil.copy2(build_manifest, output / "macos-build.json")
(output / "macos-beta.json").write_text(serialized, encoding="utf-8")
sums = "\n".join(
    f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}"
    for path in sorted(output.iterdir())
    if path.name != "SHA256SUMS.txt"
) + "\n"
(output / "SHA256SUMS.txt").write_text(sums, encoding="utf-8")
print(output)
PY

echo "✅ macOS release 构建完成："
echo "   App:        $APP_SRC"
echo "   Updater:    $APP_DIR/$ARTIFACT_NAME"
echo "   Feed:       $ROOT_DIR/updates/macos-beta.json"
