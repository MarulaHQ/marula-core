#!/usr/bin/env bash
# Regenerate the Tauri desktop bundle icons from the meridian-ci brand assets.
#
# Source of truth: meridian-ci/source/*.svg (editable SVGs — see
# meridian-ci/README.md). The committed PNGs under meridian-ci/app-icon/ are
# the rendered artifacts of those SVGs (16–1024px + maskable variants); this
# script derives the platform containers the Tauri bundler consumes from them:
#
#   desktop/icons/icon.png   ← app-icon/icon-512.png (byte-identical copy)
#   desktop/icons/icon.ico   ← multi-size ICO (16–256) re-encoded from icon-512
#   desktop/icons/icon.icns  ← macOS iconset (16–1024) via iconutil
#
# Run after any meridian-ci source change: ./scripts/generate-desktop-icons.sh
# Requires: python3 with Pillow (pip3 install pillow) + macOS iconutil.
set -euo pipefail
cd "$(dirname "$0")/.."

SRC=meridian-ci/app-icon
OUT=desktop/icons
[ -f "$SRC/icon-512.png" ] || { echo "error: $SRC/icon-512.png missing" >&2; exit 1; }
command -v iconutil >/dev/null || { echo "error: iconutil not found (macOS only)" >&2; exit 1; }
python3 -c 'import PIL' 2>/dev/null || { echo "error: Pillow missing — pip3 install pillow" >&2; exit 1; }

mkdir -p "$OUT"

# 1. icon.png — the 512 render, byte-identical to the brand asset.
cp "$SRC/icon-512.png" "$OUT/icon.png"

# 2. icon.ico — multi-resolution Windows/tauri container, re-encoded from the
#    512 master (never upscale; the old 16px-only placeholder is retired).
python3 - "$SRC/icon-512.png" "$OUT/icon.ico" <<'PY'
import sys
from PIL import Image

master = Image.open(sys.argv[1]).convert("RGBA")
ico_sizes = [16, 24, 32, 48, 64, 128, 256]
frames = [master.resize((s, s), Image.LANCZOS) for s in ico_sizes]
frames[-1].save(sys.argv[2], format="ICO", sizes=[(s, s) for s in ico_sizes])
PY

# 3. icon.icns — macOS iconset from the 1024 master (iconutil requires the
#    exact 10-variant naming scheme).
ICONSET=$(mktemp -d)/icon.iconset
mkdir -p "$ICONSET"
python3 - "$SRC/icon-1024.png" "$ICONSET" <<'PY'
import sys, os
from PIL import Image

master = Image.open(sys.argv[1]).convert("RGBA")
variants = {
    "icon_16x16.png": 16,
    "icon_16x16@2x.png": 32,
    "icon_32x32.png": 32,
    "icon_32x32@2x.png": 64,
    "icon_128x128.png": 128,
    "icon_128x128@2x.png": 256,
    "icon_256x256.png": 256,
    "icon_256x256@2x.png": 512,
    "icon_512x512.png": 512,
    "icon_512x512@2x.png": 1024,
}
for name, size in variants.items():
    master.resize((size, size), Image.LANCZOS).save(os.path.join(sys.argv[2], name))
PY
iconutil -c icns "$ICONSET" -o "$OUT/icon.icns"

echo "desktop icons regenerated from $SRC (meridian-ci/source/*.svg):"
ls -l "$OUT"/icon.png "$OUT"/icon.ico "$OUT"/icon.icns
