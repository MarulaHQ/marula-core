#!/usr/bin/env sh
# OpenAPI pin format gate (DX12): the committed snapshots
# packages/sdk/openapi.json and fixtures/openapi/v1/openapi.json are
# COMPACT single-line minified JSON, and they are a byte-identical pair.
# The API serves pretty-printed JSON, so a regen ritual that skips
# `pnpm --filter @meridian/sdk generate` (which minifies in place)
# produces a ~43k-line pretty pin — this gate fails that before merge.
set -eu

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

for pin in packages/sdk/openapi.json fixtures/openapi/v1/openapi.json; do
  [ -f "$pin" ] || fail "$pin missing"
  lines=$(wc -l < "$pin" | tr -d ' ')
  if [ "$lines" -gt 2 ]; then
    fail "$pin is pretty-printed ($lines lines) — committed format is compact minified JSON. Re-pin with: pnpm --filter @meridian/sdk generate"
  fi
done

if ! cmp -s packages/sdk/openapi.json fixtures/openapi/v1/openapi.json; then
  fail "packages/sdk/openapi.json and fixtures/openapi/v1/openapi.json must be a byte-identical pair"
fi

echo "OpenAPI pin format: clean (compact, byte-identical pair)."
