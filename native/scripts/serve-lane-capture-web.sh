#!/usr/bin/env bash
# Lane gate helper: serve the native app as expo web on the stack's CORS port.
set -a
. e2e/.e2e-stack.env
set +a
cd "$(dirname "$0")/../.."
EXPO_PUBLIC_API_URL="$E2E_API_URL" \
EXPO_PUBLIC_WS_URL="$E2E_WS_URL" \
pnpm --filter @marula-core/native exec expo start --web --port "$E2E_WEB_PORT"
