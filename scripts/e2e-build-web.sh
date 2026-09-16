#!/usr/bin/env bash
# Build the web app for the E2E suite (production standalone bundle).
# NEXT_PUBLIC_API_URL is inlined at build time; the standalone server does
# not copy .next/static or public itself, so this script does.
set -euo pipefail
export NEXT_PUBLIC_API_URL="${NEXT_PUBLIC_API_URL:-http://localhost:8080}"

pnpm --filter @meridian/ui build
pnpm --filter @meridian/web build
rm -rf apps/web/.next/standalone/.next/static apps/web/.next/standalone/public
cp -r apps/web/.next/static apps/web/.next/standalone/.next/static
cp -r apps/web/public apps/web/.next/standalone/public

# The standalone tree's node_modules entries are pnpm symlinks whose relative
# targets only resolve inside the repo (pnpm hoists the virtual store to the
# repo root, not under apps/web). Rewrite every symlink to an ABSOLUTE path —
# same fix as scripts/build-desktop.sh (ADR 0005).
find apps/web/.next/standalone -type l | while read -r link; do
  tgt=$(readlink "$link")
  case "$tgt" in
    ../../../node_modules/*)
      abs="$PWD/node_modules/${tgt#../../../node_modules/}" ;;
    *)
      abs=$(cd "$(dirname "$link")" && python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$tgt") ;;
  esac
  ln -sfn "$abs" "$link"
done
echo "[e2e-build-web] standalone bundle ready (NEXT_PUBLIC_API_URL=$NEXT_PUBLIC_API_URL)"
