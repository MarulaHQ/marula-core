#!/bin/sh
# Marula restore drill — an automated rehearsal of the newest backup.
#
# Purpose: `restore.sh` is a rehearsal an operator runs by hand, and everything
# in the reliability story depends on it being run. A drill nobody schedules is a
# rehearsal nobody runs, so this script is the scheduler: it takes the newest
# local dump (or the newest WAL-G base backup), restores it into the scratch
# database through `restore.sh`, asserts the restored database actually carries
# the product's data, and records the run — outcome, artifact and duration — in
# `ops.backup_drill_runs`.
#
# Why the duration is recorded: the written RTO in `docs/reference/rpo-rto.md` is
# a model with stated assumptions. This is the measurement that replaces it for
# this deployment, and the number an operator should quote instead of the model.
#
# What this proves, and what it does not:
#   * logical dump — the dump is restorable (schema, RLS, row data), and the
#     whole path from pg_dump to a queryable database works with the backups this
#     host actually holds.
#   * WAL-G base backup — that the base backup is RETRIEVABLE (`backup-fetch`
#     succeeds and the payload carries a PG_VERSION). It does not start a server
#     on it: that needs a dedicated Postgres instance, which is a different
#     piece of infrastructure than this container. The gap is stated in the
#     docs rather than papered over.
#
# Non-purpose: no WAL replay to a point in time (a recovery-target restore is an
# incident action, not a rehearsal), no object-store restore (restic needs the
# passphrase and the target directory; the doc says what to check instead), and
# no drop of the live database — the drill only ever writes to
# `marula_restore_test`.
set -eu

PGHOST="${PGHOST:-postgres}"
PGPORT="${PGPORT:-5432}"
PG_USER="${PG_USER:-marula}"
BACKUP_DB="${PGDATABASE:-marula}"
PGDUMPS_DIR="${PGDUMPS_DIR:-/pgdumps}"
# Same scratch database restore.sh creates; the two constants have to agree for
# the assertions below to read the database that was actually restored.
SCRATCH_DB="marula_restore_test"
BACKUP_SCRIPTS_DIR="${BACKUP_SCRIPTS_DIR:-/opt/backup}"

log() { echo "[drill $(date -u +%FT%TZ)] $*"; }
fail() {
  log "FAIL: $*"
  record failed "$DRILL_SOURCE" "${DRILL_ARTIFACT:-none}" "" "{}" "$*"
  exit 1
}

psql_live() {
  psql -h "$PGHOST" -p "$PGPORT" -U "$PG_USER" -d "$BACKUP_DB" -tA -v ON_ERROR_STOP=1 "$@"
}

psql_scratch() {
  psql -h "$PGHOST" -p "$PGPORT" -U "$PG_USER" -d "$SCRATCH_DB" -tA -v ON_ERROR_STOP=1 "$@"
}

# Writes one ledger row. Never fatal: a drill that restored correctly has done
# its job even if the ledger insert fails, and failing the drill over it would
# train an operator to ignore the alarm. The failure is logged instead.
record() {
  _outcome="$1"
  _source="$2"
  _artifact="$3"
  _duration_ms="$4"
  _rows="$5"
  _detail="${6:-}"
  # No quote can reach the statement: these values come from filenames and from
  # the sentences above, and the ledger row is not worth a syntax error.
  _artifact=$(printf '%s' "$_artifact" | tr -d "'")
  _detail=$(printf '%s' "$_detail" | tr -d "'")
  _rows=$(printf '%s' "$_rows" | tr -d "'")
  _tables=""
  if [ "$_outcome" = "passed" ]; then
    _tables=$(psql_scratch -c "SELECT count(*) FROM pg_tables WHERE schemaname='public';" 2>/dev/null || echo "")
  fi
  if ! psql_live -c "
      INSERT INTO ops.backup_drill_runs
        (started_at, finished_at, outcome, source, artifact, duration_ms,
         schema_tables, restored_rows, detail)
      VALUES ($DRILL_STARTED_AT, now(), '$_outcome', '$_source', '$_artifact',
              nullif('$_duration_ms','')::bigint, nullif('$_tables','')::integer,
              '$_rows'::jsonb, nullif('$_detail',''))" >/dev/null 2>&1; then
    log "WARNING: could not write the drill ledger row (ops.backup_drill_runs)"
  fi
}

DRILL_STARTED_AT="'$(date -u +%FT%TZ)'"
started_epoch=$(date +%s)
DRILL_SOURCE="logical_dump"
DRILL_ARTIFACT=""

# 1. Pick the artifact. An explicit path wins (so an operator can rehearse one
#    specific dump); otherwise the newest local dump; otherwise the WAL-G base.
if [ -n "${DRILL_DUMP:-}" ]; then
  DUMP="$DRILL_DUMP"
elif [ -e "$PGDUMPS_DIR" ] && [ -n "$(ls -1t "$PGDUMPS_DIR"/*.dump 2>/dev/null | head -1)" ]; then
  DUMP=$(ls -1t "$PGDUMPS_DIR"/*.dump 2>/dev/null | head -1)
else
  DUMP=""
fi

if [ -n "$DUMP" ]; then
  DRILL_ARTIFACT=$(basename "$DUMP")
  log "rehearsing the logical dump $DUMP"
  # Parity is off: the dump is minutes (or hours) old and the live database has
  # moved on, so exact counts would fail every time. What a drill can honestly
  # assert is that the restored database carries the product's data at all —
  # which is what the checks below do, on the restored rows themselves.
  if ! RESTORE_KEEP=1 RESTORE_PARITY=none PGHOST="$PGHOST" PGPORT="$PGPORT" \
    sh "$BACKUP_SCRIPTS_DIR/restore.sh" "$DUMP"; then
    fail "restore.sh rejected $DRILL_ARTIFACT"
  fi

  # 2. Assert the restored database is a Marula database with data in it.
  #    Schema completeness is restore.sh's job (it counts tables and checks RLS
  #    on every one); these are the data assertions it cannot make without
  #    comparing against a live source.
  migrations=$(psql_scratch -c "SELECT count(*) FROM _sqlx_migrations;" 2>/dev/null || echo 0)
  [ "${migrations:-0}" -gt 0 ] 2>/dev/null || fail "restored database has no migration history"

  orgs=$(psql_scratch -c "SELECT count(*) FROM orgs;" 2>/dev/null || echo 0)
  users=$(psql_scratch -c "SELECT count(*) FROM users;" 2>/dev/null || echo 0)
  tickets=$(psql_scratch -c "SELECT count(*) FROM tickets;" 2>/dev/null || echo 0)
  audit=$(psql_scratch -c "SELECT count(*) FROM audit_events;" 2>/dev/null || echo 0)
  [ "${orgs:-0}" -gt 0 ] 2>/dev/null || fail "restored database has no orgs — the dump is empty"
  [ "${users:-0}" -gt 0 ] 2>/dev/null || fail "restored database has no users — the dump is partial"
  log "  migrations=$migrations orgs=$orgs users=$users tickets=$tickets audit_events=$audit"

  duration_ms=$(( ($(date +%s) - started_epoch) * 1000 ))
  rows="{\"orgs\": ${orgs:-0}, \"users\": ${users:-0}, \"tickets\": ${tickets:-0}, \"audit_events\": ${audit:-0}}"
  record passed "$DRILL_SOURCE" "$DRILL_ARTIFACT" "$duration_ms" "$rows" ""
  psql -h "$PGHOST" -p "$PGPORT" -U "$PG_USER" -d postgres \
    -c "DROP DATABASE IF EXISTS $SCRATCH_DB;" >/dev/null
  log "PASS: $DRILL_ARTIFACT restores into a queryable database with data in it (${duration_ms} ms)"
  exit 0
fi

# 3. No local dump: a WAL-G deployment keeps its base backups in object storage,
#    so the drill proves the base backup can be FETCHED. It cannot prove Postgres
#    starts on it — that needs a second server — and the docs say so.
if [ -n "${WALG_S3_PREFIX:-}" ] && command -v wal-g >/dev/null 2>&1; then
  DRILL_SOURCE="walg_base"
  latest=$(wal-g backup-list 2>/dev/null | tail -1 | awk '{print $1}')
  [ -n "${latest:-}" ] || fail "wal-g reports no base backups"
  DRILL_ARTIFACT="$latest"
  target="/tmp/wal-drill-restore"
  rm -rf "$target"
  log "fetching WAL-G base backup $latest"
  if ! wal-g backup-fetch "$latest" "$target"; then
    fail "wal-g could not fetch base backup $latest"
  fi
  [ -f "$target/PG_VERSION" ] || fail "fetched base backup has no PG_VERSION — payload is incomplete"
  size=$(du -sk "$target" | awk '{print $1}')
  duration_ms=$(( ($(date +%s) - started_epoch) * 1000 ))
  record passed "$DRILL_SOURCE" "$DRILL_ARTIFACT" "$duration_ms" "{}" "payload ${size} KiB"
  rm -rf "$target"
  log "PASS: base backup $latest fetches and carries a Postgres data directory (${duration_ms} ms)"
  exit 0
fi

fail "no backup artifact to drill (no *.dump in $PGDUMPS_DIR, no WALG_S3_PREFIX)"
