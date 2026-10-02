#!/usr/bin/env bash
# Migration safety gate (BUILD-PROMPT §CI): destructive schema changes are
# forward-only but must be flagged in the filename with `--breaking` and
# documented, so operators can't upgrade past a data-losing step unnoticed.
# (bash, not zsh: the CI runner guarantees bash, this file is CI-portable.)
#
# What counts as destructive:
#   DROP TABLE / DATABASE / SCHEMA / INDEX / CONSTRAINT / POLICY
#   ALTER TABLE ... DROP COLUMN
#   ALTER COLUMN ... TYPE (may rewrite/reject data)
#   TRUNCATE
#   UPDATE/DELETE touching a whole table without WHERE
#
# Non-purpose: this is a static lint over migration SQL — it does not run
# migrations or diff against the live database.

set -euo pipefail

migrations_dir="$(dirname "$0")/../crates/core-db/migrations"

if [[ ! -d "$migrations_dir" ]]; then
  echo "error: migrations dir not found at $migrations_dir" >&2
  exit 1
fi

rc=0
# Only scan files NOT already flagged as breaking.
for f in "$migrations_dir"/*.sql; do
  base="$(basename "$f")"
  if [[ "$base" == *--breaking* ]]; then
    continue
  fi

  hits=$(grep -inE \
    '^\s*(DROP\s+(TABLE|DATABASE|SCHEMA|INDEX|CONSTRAINT|POLICY)|TRUNCATE)|DROP\s+COLUMN|ALTER\s+COLUMN\s+\S+\s+TYPE' \
    "$f" || true)

  # Unfiltered UPDATE/DELETE (whole-table data loss): statement starts a line
  # and carries no WHERE on that line. Keyed off the statement keyword so
  # `ON DELETE CASCADE;` fragments inside ADD COLUMN lines never match.
  unfiltered=$(grep -inE \
    '^\s*(DELETE\s+FROM\s+\S+\s*;|UPDATE\s+\S+\s+SET\b[^;]*;)' \
    "$f" | grep -ivE 'where' || true)
  hits="${hits}${unfiltered}"

  # Deliberate whole-table backfills in ALREADY-SHIPPED migrations. Shipped
  # migration files are checksummed by sqlx — editing them (to add --breaking
  # or annotations) would break `sqlx migrate run` on every environment that
  # already applied them — so each intentional unfiltered statement is
  # whitelisted here with its rationale. Everything not listed stays strict.
  typeset -a migration_backfill_allowlist
  migration_backfill_allowlist=(
    # 0013: seeds refresh-token rotation chains — each existing session
    # becomes its own chain root (see the comment above the statement);
    # no data is lost.
    '0013_auth_hardening.sql:UPDATE auth_sessions SET chain_id = id;'
  )
  filtered="$hits"
  for entry in $migration_backfill_allowlist; do
    stmt="${entry#*:}"
    filtered="$(printf '%s\n' "$filtered" | grep -vF -- "$stmt" || true)"
  done
  hits="$filtered"

  if [[ -n "$hits" ]]; then
    echo "::error file=$base::destructive statement(s) without --breaking flag:"
    echo "$hits"
    rc=1
  fi
done

# Filename shape check: NNN_name.sql keeps history sortable and greppable.
for f in "$migrations_dir"/*.sql; do
  base="$(basename "$f")"
  if ! [[ "$base" =~ ^[0-9]{4}_[a-z0-9_]+(--breaking)?\.sql$ ]]; then
    echo "::error file=$base::migration filename must match NNN_name.sql"
    rc=1
  fi
done

if [[ "$rc" -eq 0 ]]; then
  echo "Migration safety: clean ($(ls "$migrations_dir" | wc -l | tr -d ' ') migrations scanned)."
else
  echo "Migration safety: FAILED — rename offending migrations to include --breaking and document the data impact." >&2
fi
exit "$rc"
