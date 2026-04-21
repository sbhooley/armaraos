#!/usr/bin/env bash
# Build a simple UDZO disk image from the bundled ArmaraOS.app (no Finder layout).
# Use when `cargo tauri build` fails at bundle_dmg.sh but the .app bundle succeeded.
#
# Usage:
#   ./scripts/macos_quick_dmg.sh [path/to/ArmaraOS.app] [output.dmg]
#
# Defaults (aarch64 native release):
#   target/aarch64-apple-darwin/release/bundle/macos/ArmaraOS.app
#   -> ./ArmaraOS_quick_<version>_aarch64.dmg in cwd

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_TRIPLE="${TARGET_TRIPLE:-aarch64-apple-darwin}"
APP_PATH="${1:-$ROOT/target/$TARGET_TRIPLE/release/bundle/macos/ArmaraOS.app}"
VOLNAME="${VOLNAME:-ArmaraOS}"

if [[ ! -d "$APP_PATH" ]]; then
  echo "App bundle not found: $APP_PATH" >&2
  echo "Build first: (cd crates/openfang-desktop && cargo tauri build --target $TARGET_TRIPLE)" >&2
  exit 1
fi

PLIST="$APP_PATH/Contents/Info.plist"
VERSION="0.0.0"
if [[ -f "$PLIST" ]]; then
  VERSION="$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$PLIST" 2>/dev/null || echo 0.0.0)"
fi

ARCH="$(uname -m)"
OUT="${2:-$(pwd)/ArmaraOS_quick_${VERSION}_${ARCH}.dmg}"
OUT="$(cd "$(dirname "$OUT")" && pwd)/$(basename "$OUT")"

TMP_DMG="$(mktemp -t armaraos_rwdmg.XXXXXX).dmg"
trap 'rm -f "$TMP_DMG"' EXIT

echo "Source: $APP_PATH"
echo "Output: $OUT"

hdiutil create -volname "$VOLNAME" -srcfolder "$APP_PATH" -format UDRW -ov "$TMP_DMG"
hdiutil convert "$TMP_DMG" -format UDZO -imagekey zlib-level=9 -ov -o "$OUT"
echo "Done: $OUT"
