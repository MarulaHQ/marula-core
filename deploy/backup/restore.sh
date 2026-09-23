#!/bin/sh
# Meridian restore rehearsal — scratch-restore + verify. Run manually:
#   sh deploy/backup/restore.sh <path to .dump>
# Requires: psql/pg_restore reachable and a running PG. Creates the scratch DB
# `meridian_restore_test`, restores into it, checks table count, drops it.
# A backup that has never restored is not a backup — rehearse this.
set -eu
DUMP="${1:?usage: restore.sh <path to .dump>}"
PGHOST="${PGHOST:-localhost}"
PGPORT="${PGPORT:-15432}"

log() { echo "[restore $(date -u +%FT%TZ)] $*"; }

log "scratch DB create"
psql -h "$PGHOST" -p "$PGPORT" -U meridian -d postgres \
  -c "DROP DATABASE IF EXISTS meridian_restore_test;" \
  -c "CREATE DATABASE meridian_restore_test OWNER meridian;"

log "pg_restore into scratch"
pg_restore -h "$PGHOST" -p "$PGPORT" -U meridian -d meridian_restore_test \
  --no-owner --if-exists "$DUMP"

log "verify: table count"
psql -h "$PGHOST" -p "$PGPORT" -U meridian -d meridian_restore_test \
  -tA -c "SELECT count(*) FROM pg_tables WHERE schemaname='public';"

log "drop scratch"
psql -h "$PGHOST" -p "$PGPORT" -U meridian -d postgres -c "DROP DATABASE meridian_restore_test;"
log "scratch restore OK"
