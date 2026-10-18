#!/usr/bin/env bash
# Build the web app for the E2E suite (production standalone bundle).
# NEXT_PUBLIC_API_URL is inlined at build time; the standalone server does
# not copy .next/static or public itself, so this script does.
set -euo pipefail
# The e2e stack script (scripts/e2e-stack.sh) writes the per-run env to
# e2e/.e2e-stack.env; a bundle built afterward must inline THE SAME values
# (NEXT_PUBLIC_API_URL / NEXT_PUBLIC_WS_PORT) or drift creeps back in.
if [ -f e2e/.e2e-stack.env ]; then
  echo "[e2e-build-web] sourcing e2e/.e2e-stack.env (stack run present)"
  set -a; . e2e/.e2e-stack.env; set +a
  export NEXT_PUBLIC_WS_PORT="${E2E_WS_PORT:-}"
fi
# The stack env carries E2E_API_URL; the bundle inlines it so the web
# build can never drift to a stale default (p37 doctrine: the env file
# wins, callers never hand-set ports).
export NEXT_PUBLIC_API_URL="${NEXT_PUBLIC_API_URL:-${E2E_API_URL:-http://localhost:8080}}"
export NEXT_PUBLIC_WS_PORT="${NEXT_PUBLIC_WS_PORT:-8081}"

# No kit pre-build: @marula-core/ui resolves from src and is transpiled by the
# web build (2026-10-19).
pnpm --filter @marula-core/web build
rm -rf apps/web/.next/standalone/.next/static apps/web/.next/standalone/public
cp -r apps/web/.next/static apps/web/.next/standalone/.next/static
cp -r apps/web/public apps/web/.next/standalone/public

# Guard: a standalone tree missing its static assets serves HTML that points at
# /_next/static files that are not there — every chunk 400s. Fail here instead
# of letting the suite fail with an unreadable asset error. Same posture as the
# guard in scripts/build-desktop.sh (which must leave this bundle consistent
# too — it rebuilds the same .next tree).
[ -n "$(find apps/web/.next/standalone/.next/static -name '*.js' -print -quit 2>/dev/null)" ] \
  || { echo "[e2e-build-web] error: standalone .next/static has no JS after the build" >&2; exit 1; }
[ -n "$(find apps/web/.next/standalone/.next/static -name '*.css' -print -quit 2>/dev/null)" ] \
  || { echo "[e2e-build-web] error: standalone .next/static has no CSS after the build" >&2; exit 1; }

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
