#!/usr/bin/env bash
# Build the double-clickable Meridian desktop test app (ADR 0005).
#
# One command = update the app:
#   1. pnpm web build (Next standalone bundle)
#   2. stage the standalone server into desktop/resources/web
#   3. cargo build via tauri (custom profile "desktop", SHARED target dir
#      .kilo/target-shared via .cargo/config.toml — never a worktree-local
#      CARGO_TARGET_DIR, DX6), then assemble Meridian.app by hand
#      (the Tauri npm CLI has no --profile flag and its bundler globs skip
#      hidden dirs, so the script owns the bundle layout)
#
# Incremental: web-only changes skip the Rust rebuild almost entirely;
# Rust changes are incremental on the shared cache. The data dir
# (~/Library/Application Support/Meridian) is untouched by rebuilds.
set -euo pipefail
cd "$(dirname "$0")/.."

START_TS=$(date +%s)
WEB_DIR=apps/web

echo "==> [1/4] building the Next standalone bundle"
pnpm --filter @meridian/web build
STANDALONE="$WEB_DIR/.next/standalone"
[ -f "$STANDALONE/server.js" ] || { echo "error: $STANDALONE/server.js missing" >&2; exit 1; }

echo "==> [2/4] staging the standalone server into desktop/resources/web"
# The standalone tree's node_modules entries are pnpm symlinks whose
# relative targets only resolve inside the repo (the pnpm virtual store
# lives at the repo root, not under apps/web). Copy the tree with
# symlinks preserved, then rewrite every symlink to an ABSOLUTE path of
# its resolved source — the bundle stays tiny and works as long as the
# repo checkout exists (acceptable for a local test app; documented in
# docs/adr/0005).
rm -rf desktop/resources/web
mkdir -p desktop/resources/web
cp -R "$STANDALONE/." desktop/resources/web/
find desktop/resources/web -type l | while read -r link; do
  rel="${link#desktop/resources/web/}"
  orig="$STANDALONE/$rel"
  tgt=$(readlink "$orig")
  case "$tgt" in
    ../../../node_modules/*)
      # Next authored the link for a standalone-local .pnpm store, but a
      # pnpm workspace hoists the store to the repo root — remap.
      abs="$PWD/node_modules/${tgt#../../../node_modules/}" ;;
    *)
      abs=$(cd "$(dirname "$orig")" && python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$tgt") ;;
  esac
  ln -sfn "$abs" "$link"
done
rm -rf desktop/resources/web/.next/static
cp -R "$WEB_DIR/.next/static" desktop/resources/web/.next/static
[ -d "$WEB_DIR/public" ] && cp -RL "$WEB_DIR/public" desktop/resources/web/public || true

# Tauri still requires frontendDist to exist at build time even though the
# webview is served by the bundled Next server (see ADR 0005 deviation).
mkdir -p apps/web/out
printf 'The desktop app serves the bundled Next server, not these assets.\n' > apps/web/out/index.html

echo "==> [3/4] cargo build (profile: desktop, shared target dir)"
(cd desktop && pnpm exec tauri build --no-bundle -- --profile desktop)

TARGET_DIR=$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
BIN="$TARGET_DIR/desktop/meridian-desktop"
[ -f "$BIN" ] || { echo "expected binary at $BIN" >&2; exit 1; }

echo "==> [4/4] assembling Meridian.app"
# The double-clickable app lands in the repo-local dist/ (gitignored —
# the bundle stays local; easy to find next to the checkout). Override
# with MERIDIAN_DESKTOP_APP_DIR=/some/dir ./scripts/build-desktop.sh.
APP_DIR="${MERIDIAN_DESKTOP_APP_DIR:-dist}"
APP="$APP_DIR/Meridian.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/resources"
cp "$BIN" "$APP/Contents/MacOS/meridian-desktop"
cp -R desktop/resources/web "$APP/Contents/Resources/resources/web"
cp desktop/icons/icon.icns "$APP/Contents/Resources/icon.icns"
cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Meridian</string>
  <key>CFBundleDisplayName</key><string>Meridian</string>
  <key>CFBundleIdentifier</key><string>com.meridian.desktop</string>
  <key>CFBundleExecutable</key><string>meridian-desktop</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>0.1.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSMinimumSystemVersion</key><string>10.15</string>
</dict>
</plist>
PLIST
# Ad-hoc sign so Gatekeeper lets the locally built app run.
codesign --force --deep -s - "$APP" >/dev/null

ELAPSED=$(( $(date +%s) - START_TS ))
SIZE=$(du -sh "$APP" | cut -f1)
echo
echo "Built $APP"
echo "Size: $SIZE · wall time: ${ELAPSED}s"
echo "Data dir (persists across rebuilds): ~/Library/Application Support/Meridian"