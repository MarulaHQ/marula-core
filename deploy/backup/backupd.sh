#!/bin/sh
# Marula backup runner — nightly loop.
#   BACKUP_AT  cron-ish spec in "HH:MM" (default 03:00 UTC)
#   WAL env file present  -> WAL-G continuous backup (PITR-capable)
#   RESTIC env file present -> restic snapshot over restic-unset-specified sources
#   BACKUP_VERIFY=0 -> skip the archive-integrity read (default: verify)
#   DRILL_EVERY_DAYS=N -> rehearse the restore every N days (default 7; 0 = off)
#
# Every layer records its own outcome instead of continuing quietly, and a pass
# with any failed layer does not advance the stamp the worker's backup-age gauge
# reads — so a broken backup shows up as a growing age, not as a fresh success.
# The drill is the second half of the same claim: nightly backup is not evidence,
# a restored backup is.
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

BACKUP_VERIFY="${BACKUP_VERIFY:-1}"
DRILL_EVERY_DAYS="${DRILL_EVERY_DAYS:-7}"
BACKUP_SCRIPTS_DIR="${BACKUP_SCRIPTS_DIR:-/opt/backup}"
DRILL_STAMP_FILE="${DRILL_STAMP_FILE:-${PGDUMPS_DIR:-/pgdumps}/.last_drill_epoch}"

# Reads the archive back with pg_restore's table-of-contents walker.
#
# Why this and not "the file exists": pg_dump exits 0 on a truncated write more
# often than anyone expects (a full disk mid-stream, an OOM kill of the child
# with the parent still reporting wait status). `pg_restore --list` parses the
# whole archive header and every TOC entry, which is the cheapest read that
# proves the artifact is an archive at all. It is not a restore — that is the
# drill's job — and the doc says so.
verify_dump() {
  _dump="$1"
  if [ "$BACKUP_VERIFY" != "1" ]; then
    log "verify skipped (BACKUP_VERIFY=0)"
    return 0
  fi
  if pg_restore --list "$_dump" >/dev/null 2>&1; then
    log "verify: pg_restore reads the archive ($(du -h "$_dump" 2>/dev/null | cut -f1))"
    return 0
  fi
  log "ERROR: pg_dump produced an archive pg_restore cannot read: $_dump"
  return 1
}

# Rehearses the restore on a cadence, in this container, with no operator.
#
# A drill failure does NOT poison the backup stamp: the backup may be perfect and
# the drill may have failed for its own reason (no scratch space, a lock), and
# conflating the two would send an operator to the wrong problem. The drill keeps
# its own evidence in `ops.backup_drill_runs` (deploy/backup/drill.sh writes
# one row per run, with the duration the written RTO is checked against).
maybe_drill() {
  case "$DRILL_EVERY_DAYS" in
    ''|*[!0-9]*)
      log "ERROR: DRILL_EVERY_DAYS=$DRILL_EVERY_DAYS is not a number; drill skipped"
      return 0
      ;;
  esac
  if [ "$DRILL_EVERY_DAYS" -eq 0 ]; then
    log "drill disabled (DRILL_EVERY_DAYS=0)"
    return 0
  fi
  _now=$(date +%s)
  _last=0
  if [ -f "$DRILL_STAMP_FILE" ]; then
    _last=$(cat "$DRILL_STAMP_FILE" 2>/dev/null || echo 0)
    case "$_last" in ''|*[!0-9]*) _last=0 ;; esac
  fi
  if [ $((_now - _last)) -lt $((DRILL_EVERY_DAYS * 86400)) ]; then
    log "drill not due (last run $(( (_now - _last) / 3600 ))h ago)"
    return 0
  fi
  log "restore drill starting"
  if sh "$BACKUP_SCRIPTS_DIR/drill.sh"; then
    if date -u +%s > "$DRILL_STAMP_FILE" 2>/dev/null; then
      log "restore drill passed; stamp written"
    else
      log "WARNING: drill passed but $DRILL_STAMP_FILE could not be written"
    fi
  else
    log "ERROR: restore drill FAILED — this pass has not been proven restorable"
    log "       see ops.backup_drill_runs where outcome = 'failed'"
  fi
}

# Freshness stamp for the worker's backup-age gauge (the private build tracker R7a).
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
    _dump="${PGDUMPS_DIR:-/pgdumps}/marula_$(date -u +%Y%m%d).dump"
    # Dump to a staging name and rename on success: a half-written archive must
    # never sit at the name the drill picks as "the newest backup".
    if pg_dump -h "$BACKUP_HOST" -p "$BACKUP_PORT" -U "$BACKUP_USER" \
      -Fc "${BACKUP_DB:-marula}" -f "$_dump.partial"; then
      # `set -e` would abort the whole runner on a failed rename, and the loop
      # is what keeps the schedule; a failed layer is recorded instead.
      if mv "$_dump.partial" "$_dump"; then
        if ! verify_dump "$_dump"; then failed=1; fi
      else
        log "ERROR: could not rename $_dump.partial into place"
        failed=1
      fi
    else
      log "ERROR: pg_dump failed"
      rm -f "$_dump.partial"
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
    maybe_drill
  else
    log "backup pass finished with errors; stamp NOT advanced"
  fi
  log "backup pass done"
}

while true; do
  sleep "$(until_next_run)"
  run_once
done
