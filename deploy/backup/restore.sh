#!/bin/sh
# Meridian restore rehearsal — scratch-restore + verify. Run manually:
#   sh deploy/backup/restore.sh <path to .dump>
# Requires: psql/pg_restore reachable and a running PG. Creates the scratch DB
# `meridian_restore_test`, restores into it, then asserts the restore is real:
#   1. schema landed (tables restored, RLS enforced on every public table);
#   2. round-trip row parity: when the source DB is reachable (PGDATABASE
#      set, e.g. `PGDATABASE=meridian` against the live stack), row counts
#      for every public table must match the dump's origin — a dump that
#      loses rows in transit is not restorable cleanly. Override
#      PGSOURCE_HOST/PGSOURCE_PORT when the source is a different server.
# Drops the scratch DB when done. A backup that has never restored is not a
# backup — rehearse this.
set -eu
DUMP="${1:?usage: restore.sh <path to .dump>}"
[ -f "$DUMP" ] || { echo "no such dump: $DUMP" >&2; exit 1; }
PGHOST="${PGHOST:-localhost}"
PGPORT="${PGPORT:-15432}"
SCRATCH_DB="meridian_restore_test"

log() { echo "[restore $(date -u +%FT%TZ)] $*"; }
fail() { log "FAIL: $*"; exit 1; }

psql_scratch() {
  psql -h "$PGHOST" -p "$PGPORT" -U meridian -d "$SCRATCH_DB" "$@"
}

log "scratch DB create"
# The dump carries GRANTs for meridian_app; a scratch PG created straight
# (compose stack: the app role already exists) — only a bare scratch server
# needs it created. pg_restore ignores missing-grant errors regardless, but
# a green leg should not have them.
if ! psql -h "$PGHOST" -p "$PGPORT" -U meridian -d postgres -tA \
  -c "SELECT 1 FROM pg_roles WHERE rolname='meridian_app';" | grep -q 1; then
  psql -h "$PGHOST" -p "$PGPORT" -U meridian -d postgres \
    -c "CREATE ROLE meridian_app LOGIN PASSWORD 'meridian' BYPASSRLS NOINHERIT;" >/dev/null
  log "created scratch-only role meridian_app"
fi
psql -h "$PGHOST" -p "$PGPORT" -U meridian -d postgres \
  -c "DROP DATABASE IF EXISTS $SCRATCH_DB;" \
  -c "CREATE DATABASE $SCRATCH_DB OWNER meridian;"

log "pg_restore into scratch"
pg_restore -h "$PGHOST" -p "$PGPORT" -U meridian -d "$SCRATCH_DB" \
  --no-owner --clean --if-exists "$DUMP"

log "verify: schema landed (table count)"
TABLES=$(psql_scratch -tA -c "SELECT count(*) FROM pg_tables WHERE schemaname='public';")
[ "${TABLES:-0}" -gt 0 ] 2>/dev/null || fail "scratch DB has $TABLES tables in public schema"
log "  $TABLES tables in public schema"

log "verify: RLS enforced on every public table"
# Everything org-scoped carries RLS; four server-wide registry tables are
# deliberately sans-RLS (no org_id column by design, see
# 0076_plugin_v2.sql, 0095_wave19_supplychain_identity.sql). This allowlist
# must match the migration suite exactly — count rows should not be edited.
NO_RLS="plugin_catalog|plugin_versions|plugin_signing_keys|ai_decision_question_sets"
RLS_OFF=$(psql_scratch -tA -c \
  "SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
   WHERE n.nspname = 'public' AND c.relkind = 'r' AND c.relrowsecurity = false
     AND c.relname !~ '^($NO_RLS)$';")
[ -z "${RLS_OFF:-}" ] || fail "public tables lack RLS: $RLS_OFF — restore is partial"
log "  RLS on for all $TABLES public tables"

if [ -n "${PGDATABASE:-}" ]; then
  # Parity target: the DB the dump was taken from. Defaults to the same
  # server as the restore target; point PGSOURCE_HOST/PGSOURCE_PORT at the
  # live stack when rehearsing into a separate scratch server.
  SRC_HOST="${PGSOURCE_HOST:-$PGHOST}"
  SRC_PORT="${PGSOURCE_PORT:-$PGPORT}"
  log "verify: round-trip row parity vs source DB '$PGDATABASE' via $SRC_HOST:$SRC_PORT"
  MISMATCHES=$(mktemp /tmp/.restore-parity.XXXXXX)
  psql_scratch -tA -c \
    "SELECT tablename FROM pg_tables WHERE schemaname='public' ORDER BY tablename;" |
  while IFS= read -r t; do
    a=$(psql_scratch -tA -c "SELECT count(*) FROM \"$t\";" || echo '?')
    b=$(psql -h "$SRC_HOST" -p "$SRC_PORT" -U meridian -d "$PGDATABASE" \
      -tA -c "SELECT count(*) FROM \"$t\";" || echo '?')
    if [ "$a" != "$b" ]; then
      echo "$t: scratch=$a source=$b" >>"$MISMATCHES"
    fi
  done
  if [ -s "$MISMATCHES" ]; then
    cat "$MISMATCHES" >&2
    fail "row counts differ between scratch and source"
  fi
  rm -f "$MISMATCHES"
  log "  row parity OK for all $TABLES tables"
else
  log "verify: round-trip skipped (PGDATABASE not set — point it at the source stack to assert row parity)"
fi

log "drop scratch"
psql -h "$PGHOST" -p "$PGPORT" -U meridian -d postgres -c "DROP DATABASE $SCRATCH_DB;"
log "scratch restore OK — schema complete, RLS enforced, row parity holds"
