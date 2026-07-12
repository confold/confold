#!/usr/bin/env bash
# Build and register a debug .app bundle for deep-link testing on macOS.
#
# macOS only registers URI schemes from .app bundles with CFBundleURLTypes in Info.plist.
# In dev mode (pnpm tauri dev), the binary runs without a bundle, so confold:// doesn't work.
# This script builds a debug .app bundle (pnpm tauri build --debug) which includes the
# deep-link scheme registration in its Info.plist, then registers it with Launch Services.
#
# Usage: scripts/dev-register-deeplink-macos.sh
set -euo pipefail

APP="confold-app/src-tauri/target/debug/bundle/macos/Confold.app"

if [ ! -d "$APP" ]; then
  echo "Debug .app bundle not found at $APP"
  echo "Building it now (pnpm tauri build --debug)…"
  cd confold-app && pnpm tauri build --debug 2>&1 | tail -5
  cd ..
fi

if [ ! -d "$APP" ]; then
  echo "Error: build failed — $APP not found." >&2
  exit 1
fi

LSREG="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
"$LSREG" -f "$APP"
echo "✓ Registered confold:// → $APP"
echo ""
echo "To test deep-links:"
echo "  open \"$APP\""
echo '  open "confold://compare?origin=/path/a&destination=/path/b"'
echo ""
echo "  The .app bundle connects to the Vite dev server at localhost:1420"
echo "  if it's running (pnpm dev), giving you hot reload + deep links."
