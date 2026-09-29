#!/bin/sh
# Marula backup runner — nightly loop.
#   BACKUP_AT  cron-ish spec in "HH:MM" (default 03:00 UTC)
#   WAL env file present  -> WAL-G continuous backup (PITR-capable)
#   RESTIC env file present -> restic snapshot over restic-unset-specified sources
# Value: schedule is naive UTC — container restarts don't lose the schedule
# (the loop is `wait until next date`, not `sleep`).
set -ue
BACKUP_AT="${BACKUP_AT:-03:00}"
BACKUP_HOST="${PGHOST:-postgres}"
BACKUP_PORT="${PGPORT:-5432}"
BACKUP_USER="${PG_USER:-marula}"
BACKUP_DB="${PGDATABASE:-marula}"
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
    # Retention: cull stale full backups when configured (env template ships 4).
    if [ -n "${WALG_RETAIN_FULL:-}" ]; then
      if ! wal-g delete retain FULL "$WALG_RETAIN_FULL"; then
        log "ERROR: wal-g delete retain FULL $WALG_RETAIN_FULL failed"
      fi
    fi
  else
    mkdir -p "${PGDUMPS_DIR:-/pgdumps}"
    log "pg_dump nightly"
    if ! pg_dump -h "$BACKUP_HOST" -p "$BACKUP_PORT" -U "$BACKUP_USER" \
      -Fc "${BACKUP_DB:-marula}" \
      -f "${PGDUMPS_DIR:-/pgdumps}/marula_$(date -u +%Y%m%d).dump"; then
      log "ERROR: pg_dump failed"
    fi
  fi
  # restic layer: snapshot the object store (LocalFsStore dir or garage data) + dumps
  if [ -n "${RESTIC_REPOSITORY:-}" ]; then
    log "restic backup of ${BACKUP_SOURCES:-}"
    # shellcheck disable=SC2086 # BACKUP_SOURCES is an intentional word list
    if ! restic backup ${BACKUP_SOURCES:-}; then log "ERROR: restic backup failed"; fi
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
