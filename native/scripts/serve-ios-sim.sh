#!/usr/bin/env bash
# Start Metro for an iOS Simulator run, pointed at the e2e stack.
#
# Purpose: one entry point so a device/simulator run never hand-sets the API
# URL or the Metro port. Everything comes from the stack's own env file
# (e2e/.e2e-stack.env) written by `scripts/e2e-stack.sh up` — the same rule
# serve-capture-web.sh follows for the web captures. (An inline
# `EXPO_PUBLIC_API_URL=http://localhost:$E2E_API_PORT` silently inlines an
# EMPTY port in some launchers, and the app then fails every request with a
# generic error — this file exists so that cannot happen.)
#
# Usage:
#   scripts/e2e-stack.sh up
#   bash native/scripts/serve-ios-sim.sh          # Metro on 8081
#   bash native/scripts/serve-ios-sim.sh 8082     # or a specific port

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
env_file="$root/e2e/.e2e-stack.env"

if [[ ! -f "$env_file" ]]; then
  echo "[serve-ios-sim] missing $env_file — run scripts/e2e-stack.sh up first" >&2
  exit 1
fi

# shellcheck disable=SC1090
source "$env_file"

metro_port="${1:-8081}"

export EXPO_PUBLIC_API_URL="http://localhost:${E2E_API_PORT}"
export EXPO_PUBLIC_WS_URL="ws://localhost:${E2E_WS_PORT}"
export CI=1

if [[ -z "${E2E_API_PORT:-}" ]]; then
  echo "[serve-ios-sim] E2E_API_PORT is empty in $env_file" >&2
  exit 1
fi

echo "[serve-ios-sim] api=${EXPO_PUBLIC_API_URL} ws=${EXPO_PUBLIC_WS_URL} metro=${metro_port}"
cd "$root/native"
exec npx expo start --port "${metro_port}"
