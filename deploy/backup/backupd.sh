#!/bin/sh
# Meridian backup runner — nightly loop.
#   BACKUP_AT  cron-ish spec in "HH:MM" (default 03:00 UTC)
#   WAL env file present  -> WAL-G continuous backup (PITR-capable)
#   RESTIC env file present -> restic snapshot over restic-unset-specified sources
# Value: schedule is naive UTC — container restarts don't lose the schedule
# (the loop is `wait until next date`, not `sleep`).
set -ue
BACKUP_AT="${BACKUP_AT:-03:00}"
SCRIPTS_DIR="$(cd "$(dirname "$0")" && pwd)"
BACKUP_HOST="${PGHOST:-postgres}"
BACKUP_PORT="${PGPORT:-5432}"
BACKUP_USER="${PG_USER:-meridian}"
BACKUP_DB="${PGDATABASE:-meridian}"
PGDUMPS_DIR="${PGDUMPS_DIR:-/pgdumps}"

until_next_run() {
  now=$(date +%s)
  h=$(printf '%s' "$BACKUP_AT" | cut -d: -f1)
  m=$(printf '%s' "$BACKUP_AT" | cut -d: -f2)
  tod=$(date -d "$h:$m" +%s 2>/dev/null || date -j -f "%H:%M" "$BACKUP_AT" +%s 2>/dev/null)
  [ "$tod" -le "$now" ] && tod=$((tod + 86400))
  echo $((tod - now))
}

log() { echo "[backup $(date -u +%FT%TZ)] $*"; }

run_once() {
  log "backup pass starting"
  # PG dump layer: WAL-G when PITR is configured, else a nightly logical dump.
  if [ -n "${WALG_S3_PREFIX:-}" ]; then
    log "wal-g backup-push"
    if ! wal-g backup-push; then log "ERROR: wal-g backup-push failed"; fi
  else
    mkdir -p "${PGDUMPS_DIR:-/pgdumps}"
    log "pg_dump nightly"
    if ! pg_dump -h "$BACKUP_HOST" -p "$BACKUP_PORT" -U "$BACKUP_USER" \
      -Fc "${BACKUP_DB:-meridian}" \
      -f "${PGDUMPS_DIR:-/pgdumps}/meridian_$(date -u +%Y%m%d).dump"; then
      log "ERROR: pg_dump failed"
    fi
  fi
  # restic layer: snapshot the object store (LocalFsStore dir or garage data) + dumps
  if [ -n "${RESTIC_REPOSITORY:-}" ]; then
    log "restic backup of [$BACKUP_SOURCES]"
    if ! restic backup $BACKUP_SOURCES; then log "ERROR: restic backup failed"; fi
    if ! restic forget --keep-daily "${RESTIC_KEEP_DAILY:-7}" --keep-weekly "${RESTIC_KEEP_WEEKLY:-4}" --prune; then
      log "ERROR: restic forget/prune failed"
    fi
  fi
  log "backup pass done"
}

while true; do
  sleep "$(until_next_run)"
  run_once
done
