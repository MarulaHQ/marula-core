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

# Freshness stamp for the worker's backup-age gauge (TASKLIST R7a).
#
# One line, UNIX epoch seconds — what `date +%s` prints — written ONLY after a
# pass in which no layer reported an error. It is the only breadcrumb this
# container can leave for a Rust process: there is no OTLP exporter in this
# image and no shared table for it, so the file is the interface. The worker
# reads it when MARULA_BACKUP_STAMP_PATH points here.
BACKUP_STAMP_FILE="${BACKUP_STAMP_FILE:-${PGDUMPS_DIR:-/pgdumps}/.last_success_epoch}"

write_stamp() {
  _dir=$(dirname "$BACKUP_STAMP_FILE")
  if mkdir -p "$_dir" 2>/dev/null && date -u +%s > "$BACKUP_STAMP_FILE"; then
    log "stamp written: $BACKUP_STAMP_FILE"
  else
    log "ERROR: could not write backup stamp $BACKUP_STAMP_FILE"
  fi
}

run_once() {
  log "backup pass starting"
  failed=0
  # PG dump layer: WAL-G when PITR is configured, else a nightly logical dump.
  if [ -n "${WALG_S3_PREFIX:-}" ]; then
    log "wal-g backup-push"
    if ! wal-g backup-push; then log "ERROR: wal-g backup-push failed"; failed=1; fi
    # Retention: cull stale full backups when configured (env template ships 4).
    if [ -n "${WALG_RETAIN_FULL:-}" ]; then
      if ! wal-g delete retain FULL "$WALG_RETAIN_FULL"; then
        log "ERROR: wal-g delete retain FULL $WALG_RETAIN_FULL failed"
        failed=1
      fi
    fi
  else
    mkdir -p "${PGDUMPS_DIR:-/pgdumps}"
    log "pg_dump nightly"
    if ! pg_dump -h "$BACKUP_HOST" -p "$BACKUP_PORT" -U "$BACKUP_USER" \
      -Fc "${BACKUP_DB:-marula}" \
      -f "${PGDUMPS_DIR:-/pgdumps}/marula_$(date -u +%Y%m%d).dump"; then
      log "ERROR: pg_dump failed"
      failed=1
    fi
  fi
  # restic layer: snapshot the object store (LocalFsStore dir or garage data) + dumps
  if [ -n "${RESTIC_REPOSITORY:-}" ]; then
    log "restic backup of ${BACKUP_SOURCES:-}"
    # shellcheck disable=SC2086 # BACKUP_SOURCES is an intentional word list
    if ! restic backup ${BACKUP_SOURCES:-}; then log "ERROR: restic backup failed"; failed=1; fi
    if ! restic forget --keep-daily "${RESTIC_KEEP_DAILY:-7}" --keep-weekly "${RESTIC_KEEP_WEEKLY:-4}" --prune; then
      log "ERROR: restic forget/prune failed"
      failed=1
    fi
  fi
  # A stamp is a claim that a restore is possible. A pass with any failed
  # layer makes no such claim, so the previous stamp keeps its age and the
  # gauge reports the real gap instead of a fresh one.
  if [ "$failed" -eq 0 ]; then
    write_stamp
  else
    log "backup pass finished with errors; stamp NOT advanced"
  fi
  log "backup pass done"
}

while true; do
  sleep "$(until_next_run)"
  run_once
done
