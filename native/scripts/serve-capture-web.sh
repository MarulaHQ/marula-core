#!/usr/bin/env bash
# Serve the native app on the e2e stack's web port, for pack captures.
#
# Purpose: one entry point so capture runs never hand-set ports or the API
# URL — everything comes from the stack's own env file (e2e/.e2e-stack.env),
# which `scripts/e2e-stack.sh up` wrote. Mirrors the documented invocation in
# native/scripts/capture-web.mjs.
#
# Usage (from the repo root):
#   scripts/e2e-stack.sh up
#   bash native/scripts/serve-capture-web.sh
#   node native/scripts/capture-pack.mjs

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
env_file="$root/e2e/.e2e-stack.env"

if [[ ! -f "$env_file" ]]; then
  echo "[serve-capture-web] missing $env_file — run scripts/e2e-stack.sh up first" >&2
  exit 1
fi

# shellcheck disable=SC1090
source "$env_file"

export EXPO_PUBLIC_API_URL="http://localhost:${E2E_API_PORT}"
export CI=1

echo "[serve-capture-web] api=${EXPO_PUBLIC_API_URL} web=${E2E_WEB_PORT}"
cd "$root"
exec pnpm --filter @marula-core/native exec expo start --web --port "${E2E_WEB_PORT}"
