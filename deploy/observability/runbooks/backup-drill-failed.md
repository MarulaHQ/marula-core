# The restore drill failed, or has not run

**Reached from:** `MarulaBackupStale`, `MarulaBackupUnobserved`

## What this means

The drill is the only evidence that a backup restores. `deploy/backup/drill.sh` runs
every `DRILL_EVERY_DAYS` (seven by default) from the backup container, restores the
newest artifact into a scratch database through `restore.sh`, asserts that the schema,
the RLS policies and the row counts landed, and records one row in
`ops.backup_drill_runs` with the outcome and the wall-clock duration
([RPO and RTO](../../../docs/reference/rpo-rto.md), the drill section).

Nothing pages on it. The rule pack has no alert over that table
([RPO and RTO](../../../docs/reference/rpo-rto.md), honest gaps), so this page is reached
while somebody is already looking at backups, usually because `MarulaBackupStale` fired
and the first question was whether last night's artifact is usable.

Two readings matter and they are not the same. A failed drill means an artifact that
exists did not restore. No recent drill means the claim is unmeasured, whether or not
last night's pass succeeded.

## First three checks

1. Read the recent runs:

   ```sql
   SELECT started_at, outcome, source, artifact, duration_ms, detail
     FROM ops.backup_drill_runs ORDER BY started_at DESC LIMIT 10;
   ```

   `outcome = 'failed'` with a `detail` naming a lock or scratch space is an environment
   fault. A failure during the schema or RLS assertion is the artifact.

2. How stale is the proof:

   ```sql
   SELECT now() - max(started_at) AS since_last_drill,
          count(*) FILTER (WHERE outcome = 'failed') AS failures_last_30d
     FROM ops.backup_drill_runs
    WHERE started_at > now() - interval '30 days';
   ```

   A month with no row at all means the drill is not scheduled on this install, and the
   duration term in the RTO arithmetic is still an assumption.

3. Is the artifact itself readable, before blaming the drill:

   ```sh
   docker compose run --rm backup sh -c 'pg_restore --list "$(ls -t "${PGDUMPS_DIR:-/pgdumps}"/*.dump | head -1)" | head'
   ```

   A truncated archive fails here in milliseconds. With WAL-G configured, `wal-g
   backup-list` is the equivalent and `backup-fetch` is what proves the base backup is
   retrievable.

## Escape hatch

Run the drill by hand against the newest artifact rather than waiting for the schedule:

```sh
docker compose run --rm backup sh -c 'sh /usr/local/bin/drill.sh'
```

If it fails on the schema or RLS assertions, stop treating that artifact as a backup.
Take a fresh dump, rehearse it with `deploy/backup/restore.sh`, and only then let the
next nightly pass write a stamp.

## Afterwards

Record the outcome, the artifact, the duration and the failing step. The duration is the
restore term of the RTO estimate for this hardware, so a drill that finally lands on a
large dump is worth adding to the numbers in [RPO and RTO](../../../docs/reference/rpo-rto.md).

Two follow-ups belong on a row rather than in this page: an alert over
`ops.backup_drill_runs`, and a scheduled owner for the full PITR rehearsal, which no
machine performs today and which a `durable` claim quietly depends on.
