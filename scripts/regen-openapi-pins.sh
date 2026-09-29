#!/usr/bin/env bash
# OpenAPI pin regen ritual, automated (DX12; report §3.4).
#
# Boots the demo stack through scripts/e2e-stack.sh (or reuses a healthy
# running API), curls /openapi.json, writes BOTH committed pins
# (packages/sdk/openapi.json + fixtures/openapi/v1/openapi.json) COMPACT
# single-line and byte-identical, regenerates the SDK types, then runs the
# format gate scripts/check-openapi-pins.sh.
#
# Usage: scripts/regen-openapi-pins.sh
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

API_URL="$(sed -n 's/^E2E_API_URL=//p' e2e/.e2e-stack.env 2>/dev/null | head -1 || true)"
HEALTHY=0
RUN_MODE=0
if [ -n "$API_URL" ] && curl -fsS -o /dev/null --max-time 3 "$(echo "$API_URL")api/v1/health" 2>/dev/null; then
  # ONLY a run the stack script registered is reused — a stray legacy
  # core-api on 8080 could be serving an older binary + route set and
  # reproducibly mis-pin the spec.
  HEALTHY=1
  echo "[regen-pins] reusing registered stack run at $API_URL"
fi
if [ "$HEALTHY" = "0" ]; then
  echo "[regen-pins] no registered healthy API — booting one via scripts/e2e-stack.sh"
  RUN_MODE=1
  bash scripts/e2e-stack.sh
  API_URL="$(sed -n 's/^E2E_API_URL=//p' e2e/.e2e-stack.env | head -1 || true)"
fi
API_URL="$(echo "$API_URL" | sed 's|/$||')"
cleanup() {
  if [ "$RUN_MODE" = "1" ]; then
    RUN_ID="$(grep -m1 '^MARULA_RUN_ID=' e2e/.e2e-stack.env 2>/dev/null | cut -d= -f2 | tr -d '"' || true)"
    bash scripts/e2e-stack.sh down "$RUN_ID" || true
  fi
}
trap cleanup EXIT

echo "[regen-pins] fetching /openapi.json from $API_URL"
curl -fsS "$API_URL/openapi.json" -o /tmp/meridian-openapi-pretty.json

# Write both pins as the SAME compact bytes: python minifies once (single-
# line, sorted-key-free — preserves server key order), then the identical
# buffer lands on both paths per DX12.
# Write both pins as the SAME compact bytes: node minifies once (exactly
# the JSON.stringify the SDK generate step uses, so byte-identity survives
# the in-place minify), and the identical buffer lands on both paths.
node -e '
const fs = require("fs");
const compact = JSON.stringify(JSON.parse(fs.readFileSync("/tmp/meridian-openapi-pretty.json", "utf8")));
for (const pin of ["packages/sdk/openapi.json", "fixtures/openapi/v1/openapi.json"]) {
  fs.writeFileSync(pin, compact);
}
console.log("[regen-pins] wrote both pins compact (" + compact.length + " bytes)");
'

echo "[regen-pins] regenerate SDK (@meridian/sdk generate)"
pnpm --filter @meridian/sdk generate

echo "[regen-pins] run the format gate"
bash scripts/check-openapi-pins.sh
echo "[regen-pins] OK — both pins compact, byte-identical, SDK regenerated"
