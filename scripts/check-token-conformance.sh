#!/usr/bin/env bash
# Token conformance — no colours outside the Marula token set.
#
# Core's theme is derived from the Marula brand (design spec §4.5): there is
# no second palette, so the rule is simply that no component may define a
# colour — it imports the token. This check is a RATCHET, the same shape the
# shared-chrome check uses: the app carries hexes from before the token
# migration, so the enforcing ledger below may only SHRINK. It fails when
#   - a ledgered file exceeds its recorded high-water count, or
#   - any file outside the ledger contains a raw colour at all.
# Growing the ledger or its counts is a review-visible act, never a silent one.
# When a file reaches zero it is removed from the ledger in the same PR.

set -euo pipefail
cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)"

# The token set itself, and the mirrors generated FROM it, define colours by
# definition — they are the source of truth, not a violation of it.
TOKEN_ARTIFACTS='^packages/ui/tokens/tokens\.css:|^packages/ui/src/tokens\.css:|^packages/ui/src/tokens-kit\.css:|^packages/ui-native/src/generated\.ts:|^native/src/ui/tokens\.ts:'
SCAN_DIRS=(apps packages website native)
GREP_INCLUDES=(--include='*.tsx' --include='*.ts' --include='*.css' --include='*.scss')
GREP_EXCLUDES=(--exclude-dir=node_modules --exclude-dir=.next --exclude-dir=dist
               --exclude-dir=.turbo --exclude='*.test.*' --exclude='*.spec.*')
PATTERN='#[0-9a-fA-F]{6}\b|#[0-9a-fA-F]{3}\b'

hits=$(grep -rnoE "$PATTERN" "${GREP_INCLUDES[@]}" "${GREP_EXCLUDES[@]}" \
       "${SCAN_DIRS[@]}" 2>/dev/null | grep -vE "$TOKEN_ARTIFACTS" || true)

failed=0

# The ledger: high-water <count> per file. Rationales above each row.
ledger_hits() {
  cat <<'EOF'
80|apps/web/app/portal/portal.css|Portal surface: status/brand hexes pending token extraction
10|apps/web/app/globals.css|Global web styles: base colours pending token extraction
8|apps/web/app/settings/tags/page.tsx|Tag-colour picker palette consumed inline; token set pending
4|apps/web/components/project-gantt.tsx|Gantt lane colours inline pending token extraction
2|apps/web/app/layout.tsx|theme-color meta tags; a value, not a var — tokenise at launch
2|website/app/layout.tsx|theme-color meta tags; a value, not a var — tokenise at launch
2|apps/web/app/ledger.css|Ledger accents pending token extraction
2|apps/web/components/auth/entry.css|Auth entry accents pending token extraction
1|website/app/agents/page.tsx|Marketing surface accent pending token extraction
1|apps/web/app/dev/page.tsx|Dev surface accent pending token extraction
1|apps/web/app/inventory/workbench/page.tsx|Workbench accent pending token extraction
1|apps/web/app/stock/locations/page.tsx|Stock accent pending token extraction
EOF
}

# 1. Ledgered files must not exceed their recorded counts.
while IFS='|' read -r max path rationale; do
  actual=$(grep -onE "$PATTERN" "$path" 2>/dev/null | wc -l | tr -d ' ')
  if [ "$actual" -gt "$max" ]; then
    echo "::error file=$path::token ledger grows: $actual raw colours > recorded $max"
    echo "                   $rationale"
    echo "                   The ledger may only SHRINK. Remove the colours or reduce the count consciously."
    failed=1
  elif [ "$actual" -lt "$max" ]; then
    echo "  shrank: $path now $actual (was $max) — reduce the ledger row in this PR"
  fi
done < <(ledger_hits)

# 2. Any file outside the ledger is a violation outright.
if [ -n "$hits" ]; then
  ledger_files=$(ledger_hits | cut -d'|' -f2 | sort)
  for path in $(comm -23 <(printf '%s\n' "$hits" | awk -F: '{print $1}' | sort -u) \
                         <(printf '%s\n' "$ledger_files")); do
    echo "::error file=$path::raw colour outside the Marula token set and absent from the shrink ledger — tokenise it (docs/theme.md), or add it to the ledger with a rationale"
    failed=1
  done
fi

if [ "$failed" -eq 0 ]; then
  total=$(printf '%s\n' "$hits" | grep -c . || true)
  echo "Clean — token ratchet holds: $total raw colours across the app, all inside the shrinking ledger."
fi
exit $failed
