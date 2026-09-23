#!/usr/bin/env bash
# E2E demo stack (Wave 14 lane 8 `p14-e2e-ci`).
#
# Boots the exact stack the Playwright suite runs against: Postgres (service
# container in CI, Docker locally) → migrations → deterministic demo seed →
# core-api. The web app is started by Playwright's webServer on a production
# build (see e2e/playwright.config.ts).
#
# Usage: scripts/e2e-stack.sh            # DATABASE_URL must point at a fresh DB
#        E2E_KEEP=1 scripts/e2e-stack.sh # keep the local scratch container after boot
set -euo pipefail

export DATABASE_URL="${DATABASE_URL:-postgres://postgres:postgres@localhost:5432/meridian_test}"
export BIND_ADDR="${BIND_ADDR:-0.0.0.0:8080}"
export JWT_SECRET="${JWT_SECRET:-e2e-demo-secret-not-for-production}"
export WEB_BASE_URL="${WEB_BASE_URL:-http://localhost:3000}"
# Browser clients hit the API cross-origin from the Playwright web server.
export CORS_ORIGIN="${CORS_ORIGIN:-http://localhost:3000}"

# Local convenience: spin up a scratch Postgres when nothing is listening.
if ! command -v pg_isready >/dev/null 2>&1 || ! pg_isready -q -d "$DATABASE_URL"; then
  if [ -z "${E2E_PG_CONTAINER:-}" ] && command -v docker >/dev/null 2>&1; then
    echo "[e2e-stack] no DATABASE_URL reachable — starting scratch Postgres (pgvector:pg18)"
    docker rm -f meridian-e2e-pg >/dev/null 2>&1 || true
    docker run -d --name meridian-e2e-pg \
      -e POSTGRES_USER=postgres -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=meridian_test \
      -p 5432:5432 pgvector/pgvector:pg18 >/dev/null
    if [ -z "${E2E_KEEP:-}" ]; then trap 'docker rm -f meridian-e2e-pg >/dev/null 2>&1 || true' EXIT; fi
    for _ in $(seq 1 30); do pg_isready -q -d "$DATABASE_URL" && break; sleep 1; done
    pg_isready -q -d "$DATABASE_URL" || { echo "[e2e-stack] scratch Postgres never became ready"; exit 1; }
  else
    echo "[e2e-stack] DATABASE_URL not reachable and no docker to start one: $DATABASE_URL"
    exit 1
  fi
fi

echo "[e2e-stack] migrate schema"
cargo run -q -p meridian-cli -- migrate

echo "[e2e-stack] seed the deterministic demo org"
MERIDIAN_DEV_TOOLS=1 cargo run -q -p meridian-cli -- seed-demo --force

echo "[e2e-stack] boot core-api on ${BIND_ADDR} (release — E2E latency needs the optimized build)"
cargo build -q -p core-api --release
BIN="$(cargo metadata -q --no-deps --format-version 1 | python3 -c \
  'import json,sys; print(json.load(sys.stdin)["target_directory"])')/release/core-api"
"$BIN" &
API_PID=$!
# NOTE: no EXIT trap killing the API — in CI this script runs in its own
# step and the API must SURVIVE into the test steps on the same runner.
# E2E_FOREGROUND=1 keeps the script attached (local debugging); the plain
# invocation returns with core-api still running in the background.
disown "$API_PID" 2>/dev/null || true

API_HOST="$(echo "$BIND_ADDR" | sed 's/^[0-9]*://; s/^0\.0\.0\.0/localhost/')"
for _ in $(seq 1 60); do
  curl -fsS "http://${API_HOST}/api/v1/health" >/dev/null 2>&1 && break
  sleep 1
done
curl -fsS "http://${API_HOST}/api/v1/health" >/dev/null
echo "[e2e-stack] core-api healthy — demo env ready (owner@demo.local / demo-password-123)"

if [ "${E2E_FOREGROUND:-0}" = "1" ]; then
  wait "$API_PID"
else
  echo "[e2e-stack] core-api PID $API_PID left running (kill it when done)"
fi
