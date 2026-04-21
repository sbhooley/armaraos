#!/usr/bin/env bash
# Re-run the Tauri-generated bundle_dmg.sh with the SAME layout as `cargo tauri build`:
#   cwd = target/<triple>/release/bundle/macos
#   --volicon must resolve to .../bundle/dmg/*.icns (use absolute path; relative ../dmg fails from wrong cwd).
#
# Usage (from anywhere):
#   ./scripts/run_bundle_dmg_like_tauri.sh [aarch64-apple-darwin|x86_64-apple-darwin]
#
# Requires a successful build that wrote:
#   target/<triple>/release/bundle/macos/ArmaraOS.app
#   target/<triple>/release/bundle/dmg/bundle_dmg.sh
#   target/<triple>/release/bundle/dmg/*.icns

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TRIPLE="${1:-aarch64-apple-darwin}"
MACOS="$ROOT/target/$TRIPLE/release/bundle/macos"
DMGDIR="$ROOT/target/$TRIPLE/release/bundle/dmg"
SCRIPT="$DMGDIR/bundle_dmg.sh"

VERSION="$(
  awk '/\[workspace.package\]/{p=1} p && /^version = /{gsub(/"/,"",$3); print $3; exit}' "$ROOT/Cargo.toml"
)"
if [[ -z "$VERSION" ]]; then
  echo "Could not read workspace version from Cargo.toml" >&2
  exit 1
fi
case "$TRIPLE" in
  aarch64-apple-darwin) ARCH_LABEL=aarch64 ;;
  x86_64-apple-darwin) ARCH_LABEL=x64 ;;
  *)
    echo "Unsupported triple: $TRIPLE" >&2
    exit 1
    ;;
esac
DMG_NAME="ArmaraOS_${VERSION}_${ARCH_LABEL}.dmg"

if [[ ! -d "$MACOS" || ! -d "$DMGDIR" ]]; then
  echo "Missing bundle output. Build from repo root:" >&2
  echo "  cd crates/openfang-desktop && cargo tauri build --target $TRIPLE" >&2
  exit 1
fi
if [[ ! -f "$SCRIPT" ]]; then
  echo "Missing $SCRIPT (Tauri writes this during the DMG step)." >&2
  exit 1
fi

ICON="$(ls -1 "$DMGDIR"/*.icns 2>/dev/null | head -1 || true)"
if [[ -z "$ICON" ]]; then
  echo "No .icns under $DMGDIR; cannot pass --volicon." >&2
  exit 1
fi

# Match openfang-desktop defaults (Tauri DmgConfig): see tauri-utils DmgConfig defaults.
cd "$MACOS"
# Avoid "hdiutil: convert failed - File exists" from a prior debug run.
rm -f "$DMG_NAME" rw.*."$DMG_NAME" 2>/dev/null || true
set -x
exec "$SCRIPT" \
  --volname ArmaraOS \
  --icon ArmaraOS.app 180 170 \
  --app-drop-link 480 170 \
  --window-size 660 400 \
  --hide-extension ArmaraOS.app \
  --volicon "$ICON" \
  "$DMG_NAME" ArmaraOS.app
