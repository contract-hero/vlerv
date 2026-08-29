#!/usr/bin/env bash
# Build Vlervtifacts.app for the iOS Simulator (arm64, debug).
#
# Output: src-tauri/gen/apple/build/arm64-sim/Vlervtifacts.app
# Bundle id: dev.vlerv.Vlervcode
#
# Two machine-specific facts this script handles, so the build is one command:
#
# 1. `xcode-select -p` points at /Library/Developer/CommandLineTools here, and
#    xcodebuild refuses to run from a Command Line Tools instance. Exporting
#    DEVELOPER_DIR in this shell is NOT enough: the Tauri CLI builds the child
#    environment for xcodebuild from an allow-list (cargo-mobile2) and drops
#    DEVELOPER_DIR. The fix is a shim named `xcodebuild`, first on PATH, that
#    sets DEVELOPER_DIR and execs the real /usr/bin/xcodebuild.
#
# 2. The CLI archives to build/src_tauri_iOS.xcarchive and then MOVES the .app
#    out of it. A left-over .app from an earlier run makes that move fail with
#    "failed to rename app ...: Directory not empty (os error 66)" AFTER
#    xcodebuild already reported BUILD SUCCEEDED. Delete the archive first.
#
# The `tauri ios xcode-script` Xcode build phase talks to an RPC server that
# only `tauri ios build`/`tauri ios dev` starts, so a bare `xcodebuild` run
# cannot replace this script.
set -euo pipefail

DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
export DEVELOPER_DIR

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APPLE_DIR="$REPO_ROOT/src-tauri/gen/apple"
APP_PATH="$APPLE_DIR/build/arm64-sim/Vlervtifacts.app"

if [ ! -d "$APPLE_DIR" ]; then
  echo "gen/apple is missing. Run: pnpm tauri ios init" >&2
  exit 1
fi

SHIM_DIR="$(mktemp -d)"
trap 'rm -rf "$SHIM_DIR"' EXIT
cat > "$SHIM_DIR/xcodebuild" <<EOF
#!/bin/sh
export DEVELOPER_DIR="$DEVELOPER_DIR"
exec /usr/bin/xcodebuild "\$@"
EOF
chmod +x "$SHIM_DIR/xcodebuild"
export PATH="$SHIM_DIR:$PATH"

rm -rf "$APPLE_DIR/build/src_tauri_iOS.xcarchive" "$APPLE_DIR/build/arm64-sim"

cd "$REPO_ROOT"
pnpm tauri ios build --debug --target aarch64-sim "$@"

echo
echo "app:       $APP_PATH"
echo "bundle id: dev.vlerv.Vlervcode"
echo
echo "install and run on the iPhone 17 Pro simulator:"
echo "  xcrun simctl boot AF4CB22E-8E9F-4E83-ADFC-0FFF70B657FE   # ok if booted"
echo "  xcrun simctl install booted '$APP_PATH'"
echo "  xcrun simctl launch booted dev.vlerv.Vlervcode"
