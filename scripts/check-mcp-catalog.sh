#!/usr/bin/env bash
# The MCP catalog pin gate (row A29). Regenerates the derived artifacts in
# place and fails when the committed copies were stale, so a policy or
# snapshot change can never land without its regenerated catalog and cutover
# migration (the same discipline as scripts/check-openapi-pins.sh).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
cp packages/sdk/mcp-catalog.json "$tmp/catalog.json"
cp crates/core-db/migrations/0171_mcp_catalog_cutover.sql "$tmp/cutover.sql"

node scripts/gen-mcp-catalog.mjs >/dev/null

stale=0
cmp -s packages/sdk/mcp-catalog.json "$tmp/catalog.json" || stale=1
cmp -s crates/core-db/migrations/0171_mcp_catalog_cutover.sql "$tmp/cutover.sql" || stale=1

if [ "$stale" = "1" ]; then
  echo "[mcp-catalog] artifacts were stale — regenerated in place; review and commit the diff" >&2
  exit 1
fi
echo "[mcp-catalog] artifacts current"
