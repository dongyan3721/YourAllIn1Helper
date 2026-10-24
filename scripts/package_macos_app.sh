#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

if ! cargo tauri --version >/dev/null 2>&1; then
  echo "Tauri CLI is required. Install it with: cargo install tauri-cli --version '^2'"
  exit 1
fi

# Tauri builds the Rust host, bundles src-ui, and creates/signs the macOS .app.
# No Swift host or custom Cocoa window manager is used.
cargo tauri build
