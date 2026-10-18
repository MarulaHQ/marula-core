#!/usr/bin/env bash
# p47-fidelity-audit.sh — Wave 47 lane A13, `p47-vision-captures`.
#
# One run of the whole audit, in the order the evidence depends on it. Every leg
# writes under design/qa-p47-vision-captures/ and a leg that fails does not stop
# the others: an audit that stops at the first red reports one thing instead of
# all of them.
#
#   1. demo-surface captures + the multi-desktop responsive matrix
#   2. the pack-conformance captures (surface pack + workbench pack, both themes)
#   3. the structural pack diff   -> PACK-DIFF.json
#   4. the vision grids           -> <=8 panes per image, worst states first
#   5. the legibility audit       -> contrast, type floor, targets, clipping
#   6. the pack geometry gate     -> the pack HTML's own numbers
#   7. the axe accessibility matrix (31 surfaces x light + dark)
#
# Usage:
#   scripts/p47-fidelity-audit.sh              # assume the stack is up
#   scripts/p47-fidelity-audit.sh --boot       # boot the stack first
#   scripts/p47-fidelity-audit.sh --only 5     # one leg
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT" || exit 1

LANE="design/qa-p47-vision-captures"
LOGS="$LANE/gate-logs"
ENV_FILE="e2e/.e2e-stack.env"
PACK_DIFF_DIR="$LANE/pack-diff"

BOOT=0
ONLY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --boot) BOOT=1 ;;
    --only) ONLY="${2:-}"; [ -n "$ONLY" ] || { echo "--only needs a leg number" >&2; exit 2; } ;;
    help|-h|--help) sed -n '17,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown flag '$1'" >&2; exit 2 ;;
  esac
  shift
done

mkdir -p "$LOGS" "$PACK_DIFF_DIR"

want() { [ -z "$ONLY" ] || [ "$ONLY" = "$1" ]; }

if [ "$BOOT" = "1" ]; then
  echo "[audit] booting the stack through scripts/e2e-stack.sh"
  bash scripts/e2e-stack.sh up || { echo "[audit] boot failed" >&2; exit 1; }
fi
if [ ! -f "$ENV_FILE" ]; then
  echo "[audit] no $ENV_FILE — boot one: scripts/e2e-stack.sh up (or pass --boot)" >&2
  exit 1
fi

# The env file is the single source of truth for the run's ports. Without this
# export a bare playwright invocation sends global-setup to the wrong port.
set -a
# shellcheck disable=SC1090
. "$ENV_FILE"
set +a
export NEXT_PUBLIC_WS_PORT="${E2E_WS_PORT:-}"
RUN_ID="${MARULA_RUN_ID:-unknown}"
echo "[audit] run $RUN_ID — api ${E2E_API_URL:-?} web ${E2E_WEB_URL:-?}"

declare -a NAMES=() CODES=()
step() { # step <n> <name> <cmd...>
  local n="$1" name="$2"; shift 2
  if ! want "$n"; then return 0; fi
  local log
  log="$LOGS/$n-$(echo "$name" | tr ' /' '--').log"
  echo "[audit] ($n) $name"
  ( "$@" ) >"$log" 2>&1
  local rc=$?
  NAMES+=("$n $name"); CODES+=("$rc")
  if [ "$rc" = "0" ]; then echo "[audit]     ok (log: $log)"; else echo "[audit]     exit $rc — see $log"; fi
  return 0
}

step 1 "build-web"            bash scripts/e2e-build-web.sh
step 2 "demo-captures"        node e2e/p47-vision-captures.mjs
step 3 "pack-captures"        env P41_AUDIT_OUT="$PACK_DIFF_DIR" node e2e/p41-audit-captures.mjs --pack both
step 4 "pack-diff"            python3 scripts/p41-pack-diff.py --capture-dir "$PACK_DIFF_DIR" --limit 200
step 5 "vision-grids"         python3 scripts/p41-vision-grid.py --pack both --capture-dir "$PACK_DIFF_DIR"
step 6 "legibility"           node e2e/p47-legibility-audit.mjs
step 7 "pack-geometry"        python3 scripts/check-pack-geometry.py
step 8 "axe-matrix"           pnpm exec playwright test --config=e2e/playwright.config.ts e2e/flows/axe-audit.spec.ts

echo
echo "[audit] === summary (run $RUN_ID) ==="
fail=0
for i in "${!NAMES[@]}"; do
  code="${CODES[$i]}"
  if [ "$code" = "0" ]; then mark="ok  "; else mark="FAIL"; fail=1; fi
  printf '[audit]   %s %s (exit %s)\n' "$mark" "${NAMES[$i]}" "$code"
done
echo "[audit] artifacts under $LANE/ (captures/, $PACK_DIFF_DIR/, legibility/, PACK-DIFF.json, gate-logs/)"
exit "$fail"
