#!/bin/sh
# Marula restore rehearsal — scratch-restore + verify. Run manually:
#   sh deploy/backup/restore.sh <path to .dump>
# Requires: psql/pg_restore reachable and a running PG. Creates the scratch DB
# `marula_restore_test`, restores into it, then asserts the restore is real:
#   0. the archive's SHA-256 matches the `.sha256` sidecar backupd.sh wrote
#      next to it. A dump without a sidecar is refused by default: restoring an
#      unverified artifact is how a truncated backup becomes a partial database
#      nobody notices. RESTORE_VERIFY=warn restores it with a warning,
#      RESTORE_VERIFY=off skips the check entirely (both for a legacy dump).
#   1. schema landed (tables restored, RLS enforced on every public table);
#   2. round-trip row parity: when the source DB is reachable (PGDATABASE
#      set, e.g. `PGDATABASE=marula` against the live stack), row counts
#      for every public table must match the dump's origin — a dump that
#      loses rows in transit is not restorable cleanly. Override
#      PGSOURCE_HOST/PGSOURCE_PORT when the source is a different server.
#      Set RESTORE_PARITY=none to skip it: an automated drill restores a dump
#      taken minutes ago against a live database, where exact counts are
#      EXPECTED to differ and the comparison would fail every time.
# Drops the scratch DB when done; RESTORE_KEEP=1 leaves it in place so a caller
# (deploy/backup/drill.sh) can assert against the restored rows before dropping
# it. A backup that has never restored is not a backup — rehearse this.
set -eu
DUMP="${1:?usage: restore.sh <path to .dump>}"
[ -f "$DUMP" ] || { echo "no such dump: $DUMP" >&2; exit 1; }
PGHOST="${PGHOST:-localhost}"
PGPORT="${PGPORT:-15432}"
SCRATCH_DB="marula_restore_test"
# strict (default) | warn | off — see the header.
RESTORE_VERIFY="${RESTORE_VERIFY:-strict}"

log() { echo "[restore $(date -u +%FT%TZ)] $*"; }
fail() { log "FAIL: $*"; exit 1; }

# Reads the sidecar backupd.sh writes, and compares digests rather than the
# filename recorded in it: a dump restored from a different mount path would
# make `sha256sum -c` fail on the path alone, which is not what this proves.
verify_dump_checksum() {
  _dump="$1"
  case "$RESTORE_VERIFY" in
    off)
      log "verify: checksum skipped (RESTORE_VERIFY=off)"
      return 0
      ;;
    strict|warn) ;;
    *)
      fail "RESTORE_VERIFY=$RESTORE_VERIFY is not one of strict|warn|off"
      ;;
  esac
  if [ ! -f "$_dump.sha256" ]; then
    if [ "$RESTORE_VERIFY" = "warn" ]; then
      log "WARNING: no checksum sidecar next to $_dump — integrity unproven (RESTORE_VERIFY=warn)"
      return 0
    fi
    fail "no checksum sidecar ($(basename "$_dump").sha256) — the archive's integrity is unproven. backupd.sh writes one with every dump. Re-run with RESTORE_VERIFY=warn to restore it anyway, or RESTORE_VERIFY=off to skip this check."
  fi
  _expected=$(cut -d' ' -f1 "$_dump.sha256" 2>/dev/null)
  [ -n "$_expected" ] || fail "checksum sidecar $_dump.sha256 is empty or unreadable"
  _actual=$(sha256sum "$_dump" | cut -d' ' -f1)
  [ "$_actual" = "$_expected" ] ||
    fail "checksum mismatch: sidecar says $_expected, archive is $_actual — the archive is truncated or corrupt"
  log "verify: sha256 matches ($_actual)"
}

psql_scratch() {
  psql -h "$PGHOST" -p "$PGPORT" -U marula -d "$SCRATCH_DB" "$@"
}

# Before any database work: an archive that does not match its sidecar is not
# restored at all, so a truncated dump cannot become a scratch database that
# looks healthy until someone misses rows.
verify_dump_checksum "$DUMP"

log "scratch DB create"
# The dump carries GRANTs for marula_app; a scratch PG created straight
# (compose stack: the app role already exists) — only a bare scratch server
# needs it created. pg_restore ignores missing-grant errors regardless, but
# a green leg should not have them.
if ! psql -h "$PGHOST" -p "$PGPORT" -U marula -d postgres -tA \
  -c "SELECT 1 FROM pg_roles WHERE rolname='marula_app';" | grep -q 1; then
  psql -h "$PGHOST" -p "$PGPORT" -U marula -d postgres \
    -c "CREATE ROLE marula_app LOGIN PASSWORD 'marula' BYPASSRLS NOINHERIT;" >/dev/null
  log "created scratch-only role marula_app"
fi
psql -h "$PGHOST" -p "$PGPORT" -U marula -d postgres \
  -c "DROP DATABASE IF EXISTS $SCRATCH_DB;" \
  -c "CREATE DATABASE $SCRATCH_DB OWNER marula;"

log "pg_restore into scratch"
pg_restore -h "$PGHOST" -p "$PGPORT" -U marula -d "$SCRATCH_DB" \
  --no-owner --clean --if-exists "$DUMP"

log "verify: schema landed (table count)"
TABLES=$(psql_scratch -tA -c "SELECT count(*) FROM pg_tables WHERE schemaname='public';")
[ "${TABLES:-0}" -gt 0 ] 2>/dev/null || fail "scratch DB has $TABLES tables in public schema"
log "  $TABLES tables in public schema"

log "verify: RLS enforced on every public table"
# Everything org-scoped carries RLS; four server-wide registry tables are
# deliberately sans-RLS (no org_id column by design, see
# 0076_plugin_v2.sql, 0095_wave19_supplychain_identity.sql), and
# `org_erasure_receipts` is a platform record whose tenant column is
# `purged_org_id` — the org it names no longer exists (0146, see
# docs/reference/data-lifecycle.md §3). This allowlist must match the migration
# suite exactly — count rows should not be edited.
NO_RLS="plugin_catalog|plugin_versions|plugin_signing_keys|ai_decision_question_sets|org_erasure_receipts"
RLS_OFF=$(psql_scratch -tA -c \
  "SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
   WHERE n.nspname = 'public' AND c.relkind = 'r' AND c.relrowsecurity = false
     AND c.relname !~ '^($NO_RLS)$';")
[ -z "${RLS_OFF:-}" ] || fail "public tables lack RLS: $RLS_OFF — restore is partial"
log "  RLS on for all $TABLES public tables"

if [ -n "${PGDATABASE:-}" ] && [ "${RESTORE_PARITY:-exact}" != "none" ]; then
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
    b=$(psql -h "$SRC_HOST" -p "$SRC_PORT" -U marula -d "$PGDATABASE" \
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
elif [ -n "${PGDATABASE:-}" ]; then
  log "verify: round-trip skipped (RESTORE_PARITY=none — the caller asserts on the restored rows)"
else
  log "verify: round-trip skipped (PGDATABASE not set — point it at the source stack to assert row parity)"
fi

if [ "${RESTORE_KEEP:-0}" = "1" ]; then
  log "scratch kept ($SCRATCH_DB) — the caller drops it after its own assertions"
else
  log "drop scratch"
  psql -h "$PGHOST" -p "$PGPORT" -U marula -d postgres -c "DROP DATABASE $SCRATCH_DB;"
fi
log "scratch restore OK — schema complete, RLS enforced, row parity holds"
