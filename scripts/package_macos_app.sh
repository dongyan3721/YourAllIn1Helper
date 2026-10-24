#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export PATH="/Users/santa_antilles/.cargo/bin:$PATH"
export CARGO_HOME="/Users/santa_antilles/.cargo"
export RUSTUP_HOME="/Users/santa_antilles/.rustup"

APP_NAME="AllInOneToolbox"
APP_BUNDLE="$ROOT_DIR/dist/${APP_NAME}.app"
CONTENTS_DIR="$APP_BUNDLE/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

echo "[1/4] Building Rust Microkernel Daemon (release profile)..."
cargo build --release --offline -p all-in-one-host

echo "[2/4] Preparing macOS Application Bundle at: $APP_BUNDLE ..."
rm -rf "$APP_BUNDLE"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR/src-ui" "$RESOURCES_DIR/runtime_data"

cp "$ROOT_DIR/target/release/all-in-one-desktop" "$MACOS_DIR/all-in-one-desktop"
chmod +x "$MACOS_DIR/all-in-one-desktop"

cp "$ROOT_DIR/src-ui/index.html" "$RESOURCES_DIR/src-ui/index.html"
cp "$ROOT_DIR/src-ui/styles.css" "$RESOURCES_DIR/src-ui/styles.css"
cp "$ROOT_DIR/src-ui/app.js" "$RESOURCES_DIR/src-ui/app.js"

cat > "$CONTENTS_DIR/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>All-in-One Toolbox</string>
  <key>CFBundleDisplayName</key>
  <string>All-in-One Toolbox</string>
  <key>CFBundleIdentifier</key>
  <string>com.allinone.desktop</string>
  <key>CFBundleVersion</key>
  <string>0.1.0</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleExecutable</key>
  <string>${APP_NAME}</string>
  <key>LSMinimumSystemVersion</key>
  <string>13.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSAppTransportSecurity</key>
  <dict>
    <key>NSAllowsArbitraryLoads</key>
    <true/>
  </dict>
</dict>
</plist>
EOF

echo "[3/4] Compiling Native macOS Cocoa + WKWebView + Carbon + Quartz Host..."
/usr/bin/swiftc -O \
  "$ROOT_DIR/src-tauri/macos/NativeHost.swift" \
  -framework Cocoa \
  -framework WebKit \
  -framework Carbon \
  -framework ApplicationServices \
  -o "$MACOS_DIR/${APP_NAME}"
chmod +x "$MACOS_DIR/${APP_NAME}"

echo "[4/4] Signing & verifying packaged bundle..."
codesign --force --deep --sign - "$APP_BUNDLE"
ls -lh "$MACOS_DIR"
echo "=================================================================="
echo "✅ Packaged macOS Desktop App ready:"
echo "   $APP_BUNDLE"
echo "   Launch from Finder or terminal: open \"$APP_BUNDLE\""
echo "=================================================================="
