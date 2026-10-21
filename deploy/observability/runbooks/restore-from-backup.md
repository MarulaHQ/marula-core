# Backup is stale: restore or re-establish

**Pages:** `MarulaBackupStale`
**Reached from:** `MarulaBackupUnobserved` (opt-in rule)

## What the alert means

The last backup pass that finished without an error is more than 26 hours old,
against a nightly runner. `deploy/backup/backupd.sh` writes its stamp only after a
pass in which no layer reported an error, so a stamp that has not moved means
either last night failed somewhere or the runner is not running.

The alert is about recovery point. Every hour in this state is an hour of
committed data with no copy, so the first action is to get a copy, not to explain
the old one.

## First three checks

1. Is the runner alive, and did it fail or never start:

   ```sh
   docker compose --profile backup ps run backup
   docker compose logs --tail=100 backup
   ```

   The log names the layer that failed. `wal-g backup-push` failing is object
   storage or credentials. A failing `pg_dump` is usually the database refusing
   connections or the volume being full.

2. Does the stamp exist and what does it say, which separates "the pass failed"
   from "the pass succeeded and the stamp is unwritable":

   ```sh
   docker compose run --rm backup sh -c \
     'ls -l "${BACKUP_STAMP_FILE:-${PGDUMPS_DIR:-/pgdumps}/.last_success_epoch}" && \
      date -u -r "$(cat "${BACKUP_STAMP_FILE:-${PGDUMPS_DIR:-/pgdumps}/.last_success_epoch}")"'
   ```

   A fresh stamp with a stale metric means the worker side is wrong: check
   `MARULA_BACKUP_STAMP_PATH` and that the volume is mounted into the worker.

3. Are the artifacts there, independent of the stamp:

   ```sh
   docker compose run --rm backup ls -lh "${PGDUMPS_DIR:-/pgdumps}" | tail -5
   ```

   With WAL-G configured the newest full backup is in `WALG_S3_PREFIX` instead,
   and `wal-g backup-list` is the command that answers this.

## Escape hatch

Take a copy now, before diagnosing. A logical dump is slower to restore than a
WAL-G backup and it is still a point in time that survives whatever happens
next:

```sh
docker compose exec -T postgres pg_dump -U marula -Fc marula > marula-emergency-$(date -u +%Y%m%dT%H%MZ).dump
```

Then rehearse it. A backup that has never restored is not a backup, and the
repository ships the rehearsal:

```sh
sh deploy/backup/restore.sh ./marula-emergency-20260101T0000Z.dump
```

That script creates a scratch database, restores into it, and asserts schema,
RLS and row parity before dropping it. A green run is the only evidence that the
artifact is worth keeping.

If you are here because a restore is actually needed, restore into the live
volume only after the rehearsal passed against the same artifact. Restoring a
dump that fails the rehearsal over production data turns one incident into two.

## Afterwards

Record the failing layer, the length of the gap, and the artifact that was
rehearsed. If the gap came from the runner not being scheduled at all, that is a
deployment defect and the stamp is the last thing that would have told you.
