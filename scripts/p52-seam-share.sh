#!/usr/bin/env bash
# P12 seam share — price the internal search seam against a bulk import.
#
# Runs two ignored measurement tests and joins their numbers:
#
#   * worker  — `seam_share_over_a_bulk_seed`: seeds N tickets, then prices the
#     production indexer shape (one flush per XREAD window), the one-batch
#     floor, and the per-row shape the P12 row describes, against a counting
#     stub of core-api's `/internal/search/*` routes.
#   * core-api — `import_wall_clock_over_a_bulk_batch`: stages N rows and times
#     the worker replay of the batch, which is the import path's wall clock.
#
# The ratio answers the row: what share of an imported row's wall clock the
# seam costs today, and what the per-row shape would cost.
#
# MARULA_TEST_DATABASE_URL must point at a DISPOSABLE database: the worker test
# drops and recreates `public`. Never aim this at a stack you want to keep.
#
# Usage: MARULA_TEST_DATABASE_URL=postgres://… scripts/p52-seam-share.sh [rows]
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if [[ -z "${MARULA_TEST_DATABASE_URL:-}" ]]; then
  echo "p52-seam-share: set MARULA_TEST_DATABASE_URL to a disposable database (the worker test drops schema public)" >&2
  exit 2
fi

rows="${1:-${SEAM_ROWS:-2000}}"
export SEAM_ROWS="$rows"

evidence="design/qa-p52-internal-seam-batching/measurements"
mkdir -p "$evidence"
seam_log="$(mktemp)"
import_log="$(mktemp)"
trap 'rm -f "$seam_log" "$import_log"' EXIT

echo "[p52] seam shape (worker, $rows rows) …"
cargo test -p worker --test search_indexer_flow seam_share_over_a_bulk_seed \
  -- --ignored --nocapture --test-threads=1 >"$seam_log" 2>&1 || {
  tail -40 "$seam_log" >&2
  echo "p52-seam-share: worker measurement failed — log above" >&2
  exit 1
}

echo "[p52] import wall clock (core-api, $rows rows) …"
cargo test -p core-api --test import_async_flow import_wall_clock_over_a_bulk_batch \
  -- --ignored --nocapture --test-threads=1 >"$import_log" 2>&1 || {
  tail -40 "$import_log" >&2
  echo "p52-seam-share: import measurement failed — log above" >&2
  exit 1
}

seam_json="$(grep -m1 '^SEAM_SHARE ' "$seam_log" | sed 's/^SEAM_SHARE //' || true)"
import_json="$(grep -m1 '^IMPORT_WALL_CLOCK ' "$import_log" | sed 's/^IMPORT_WALL_CLOCK //' || true)"
if [[ -z "$seam_json" || -z "$import_json" ]]; then
  echo "p52-seam-share: a measurement printed no result line" >&2
  echo "--- worker log ---" >&2
  tail -20 "$seam_log" >&2
  echo "--- core-api log ---" >&2
  tail -20 "$import_log" >&2
  exit 1
fi

SEAM_JSON="$seam_json" IMPORT_JSON="$import_json" EVIDENCE_DIR="$evidence" python3 - <<'PY'
import json
import os

seam = json.loads(os.environ["SEAM_JSON"])
wall = json.loads(os.environ["IMPORT_JSON"])

current_share = 100.0 * seam["production_ms_per_row"] / wall["import_ms_per_row"]
per_row_share = 100.0 * seam["per_row_flush_ms_per_row"] / wall["import_ms_per_row"]

report = {
    "fixture_rows": seam["rows"],
    "import": wall,
    "seam": seam,
    "share": {
        "current_seam_percent_of_import_row": round(current_share, 3),
        "per_row_seam_percent_of_import_row": round(per_row_share, 3),
        "current_requests_per_row": seam["production_calls_per_row"],
        "per_row_requests_per_row": (
            seam["per_row_flush_calls"] / seam["rows"] if seam["rows"] else None
        ),
    },
}

print()
print(f"rows                      {report['fixture_rows']}")
print(f"import path               {wall['import_ms']} ms total, "
      f"{wall['import_ms_per_row']} ms/row, {wall['rows_per_second']} rows/s")
print(f"seam today                {seam['production_ms_per_row']} ms/row, "
      f"{seam['production_calls_per_row']} POSTs/row ({seam['production_calls']} calls)")
print(f"seam, one batch           {seam['batched_apply_ms']} ms for {seam['rows']} rows "
      f"({seam['batched_apply_calls']} POST)")
print(f"seam, per-row shape       {seam['per_row_flush_ms_per_row']} ms/row, "
      f"{seam['per_row_flush_calls']} POSTs ({seam['per_row_flush_calls']} calls)")
print(f"transport, per-row only   {seam['per_row_apply_ms_per_call']} ms/POST")
print()
print(f"seam share of an import row today      {report['share']['current_seam_percent_of_import_row']}%")
print(f"seam share if charged per row          {report['share']['per_row_seam_percent_of_import_row']}%")
print()

path = os.path.join(os.environ["EVIDENCE_DIR"], "seam-share.json")
with open(path, "w", encoding="utf-8") as handle:
    json.dump(report, handle, indent=2)
    handle.write("\n")
print(f"wrote {path}")
PY
