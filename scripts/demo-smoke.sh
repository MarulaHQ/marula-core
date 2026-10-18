#!/usr/bin/env bash
# demo-smoke.sh — the "the demo still works" net (Wave 47 lane A5,
# `p47-demo-smoke`).
#
# One command that proves the demo happy path on a freshly seeded org, against
# a real booted stack:
#
#   login → ticket create → note → timer start/stop → time entry →
#   invoice draft → portal view → BYOK surface
#
# What one run does, in order:
#   1. boot the stack through scripts/e2e-stack.sh (the ONE entry point — per-run
#      DB/ports/reseed; never hand-set NEXT_PUBLIC_API_URL or a port)
#   2. build the web bundle for the suite (scripts/e2e-build-web.sh)
#   3. run e2e/flows/demo-smoke.spec.ts through the shared Playwright config
#   4. run the live BYOK probe (scripts/demo-ai-probe.mjs) when a provider key
#      is supplied — without one the spec still proves the AI surface and its
#      admission posture, because the demo stack boots no provider runtime
#   5. tear down the run it booted (only that run id)
#
# Usage:
#   scripts/demo-smoke.sh                     # boot, build, run, tear down
#   scripts/demo-smoke.sh --keep              # leave the stack up afterwards
#   scripts/demo-smoke.sh --no-boot           # use the stack already up
#   scripts/demo-smoke.sh --project dark      # one Playwright project
#   scripts/demo-smoke.sh --ai-key "$KEY"     # also run the live BYOK probe
#
# The key can also arrive as DEMO_SMOKE_AI_KEY or PROBE_API_KEY. It is passed
# to the probe on argv only; this script never writes it anywhere.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

ENV_FILE="e2e/.e2e-stack.env"
# The spec as a path, not a bare word: Playwright's positional filter ignores
# a bare "demo-smoke" (it selects nothing and the run falls back to the whole
# suite — 172 tests, and every suite's own flake along with it). A path filters.
SPEC_PATH="e2e/flows/demo-smoke.spec.ts"
PROBE="scripts/demo-ai-probe.mjs"

BOOT=1
KEEP=0
PROJECT=""
AI_KEY="${DEMO_SMOKE_AI_KEY:-${PROBE_API_KEY:-}}"
RC=0

log()  { echo "[demo-smoke] $*"; }
# warn is reached only from teardown, and teardown only through the EXIT trap,
# so neither call site is visible to a static check.
# shellcheck disable=SC2329
warn() { echo "[demo-smoke] warning: $*" >&2; }
die()  { echo "[demo-smoke] FAIL: $*" >&2; exit 1; }

usage() {
  sed -n '21,26p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
  case "$1" in
    --keep) KEEP=1 ;;
    --no-boot) BOOT=0 ;;
    --ai-key) AI_KEY="${2:-}"; [ -n "$AI_KEY" ] || die "--ai-key needs a value" ;;
    --ai-key=*) AI_KEY="${1#--ai-key=}" ;;
    --project) PROJECT="${2:-}"; [ -n "$PROJECT" ] || die "--project needs a name" ;;
    --project=*) PROJECT="${1#--project=}" ;;
    help|-h|--help) usage; exit 0 ;;
    *) die "unknown flag '$1' (try: scripts/demo-smoke.sh help)" ;;
  esac
  shift
done

for tool in pnpm node; do
  command -v "$tool" >/dev/null 2>&1 || die "missing required tool: $tool"
done

RUN_ID=""

# reached only through the EXIT trap below
# shellcheck disable=SC2329
teardown() {
  local rc=$?
  if [ "$BOOT" = "1" ] && [ "$KEEP" = "0" ] && [ -n "$RUN_ID" ]; then
    log "tearing down $RUN_ID"
    bash scripts/e2e-stack.sh down "$RUN_ID" || warn "teardown of $RUN_ID reported a problem — check scripts/dev-status.sh"
  elif [ -n "$RUN_ID" ]; then
    log "stack left up: $RUN_ID (tear down with: scripts/e2e-stack.sh down $RUN_ID)"
  fi
  exit "$rc"
}
trap teardown EXIT

if [ "$BOOT" = "1" ]; then
  log "booting the demo stack (scripts/e2e-stack.sh up)"
  # `up` returns as soon as core-api answers its health check.
  bash scripts/e2e-stack.sh up
fi

[ -f "$ENV_FILE" ] \
  || die "no $ENV_FILE — nothing to run against. Boot one: scripts/e2e-stack.sh up (or drop --no-boot)."

# Playwright and the flake gate read these; the file is the single source of
# truth for the run's ports (p37 doctrine). Without this export a bare
# `playwright test` sends global-setup to the wrong port (Wave 47 lane A22/D).
RUN_ID="$(grep -m1 '^MARULA_RUN_ID=' "$ENV_FILE" | cut -d= -f2 | tr -d '"' || true)"
set -a
# shellcheck disable=SC1090
. "$ENV_FILE"
set +a
export NEXT_PUBLIC_WS_PORT="${E2E_WS_PORT:-}"
log "run $RUN_ID — api ${E2E_API_URL:-?} web ${E2E_WEB_URL:-?}"

log "building the web bundle for the suite"
bash scripts/e2e-build-web.sh

log "running the demo path (${SPEC_PATH}, ${PROJECT:-all themes})"
playwright_args=(--config=e2e/playwright.config.ts "$SPEC_PATH")
if [ -n "$PROJECT" ]; then
  playwright_args+=(--project "$PROJECT")
fi
pnpm exec playwright test "${playwright_args[@]}" || RC=1

if [ "$RC" = "0" ] && [ -n "$AI_KEY" ]; then
  log "running the live BYOK probe (scripts/demo-ai-probe.mjs)"
  # The key comes from the flag/env; the probe scrubs everything it prints.
  node "$PROBE" --api "${E2E_API_URL:-http://localhost:8080}" --api-key "$AI_KEY" || RC=1
elif [ "$RC" = "0" ]; then
  log "BYOK live call skipped (no key). Set DEMO_SMOKE_AI_KEY to run $PROBE."
fi

if [ "$RC" = "0" ]; then
  log "PASS — the demo path held on $RUN_ID"
else
  log "FAIL — the demo path is broken on $RUN_ID (evidence in e2e/test-results/)"
fi
exit "$RC"
