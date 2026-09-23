# Runbook: backup and restore

> **Audience:** site operator / MSP admin — you host Meridian yourself and
> own its deployment.
>
> This is the first authored page of the `deploy/runbooks/` series. The
> scaffold and future runbook index live at `deploy/runbooks/README.md`.

Everything in Meridian lives in the Postgres database, and once S3
object storage is enabled also the object-storage (Garage/S3) bucket. Back both up or
back neither up alone: document attachments (content-addressed uploads) and
versioned documents reference object storage, while everything else —
tickets, time, invoices, FX rates, the provenance and audit ledges, AI
decision answers, customization rows — is rows in Postgres.

## Backup

Three tiers — pick by how much you can lose. The compose `backup` profile
(runner at `deploy/backup/`) automates every tier below:

| Tier | Tool | You get | Cost |
|---|---|---|---|
| 0 | manual `pg_dump` (below) | last-night-to-now restore point | minutes lost per backup |
| 1 | **restic** snapshots (`deploy/backup/restic.env.example`) | deduped, encrypted snapshots of the object store + the nightly dumps — `--keep-daily 7 --keep-weekly 4` | scheduled container, one passphrase |
| 2 | **WAL-G** (`deploy/backup/wal-g.env.example`) | **point-in-time recovery** — restore to any second, not any night; WAL shipped continuously to the same S3-compatible bucket | `archive_command` + retention discipline |

```sh
docker compose -f deploy/docker-compose.yml exec postgres \
  pg_dump -U meridian -d meridian -Fc -f /tmp/meridian.dump
docker cp meridian-postgres-1:/tmp/meridian.dump meridian-$(date +%F).dump
```

Notes the runbook enforces:

- Dump as the table owner (`meridian`), not `meridian_app` — the
  RLS-constrained app role must never be able to read across tenants, which
  also means it cannot produce a complete dump.
- Dump with a client that matches the server's major version. The backup
  runner runs inside the postgres image on purpose; a diverged local
  `pg_dump` refuses the archive outright.
- If you run `OBJECT_STORE=s3` with Garage, the restic leg snapshots the
  garage data/meta volumes in the same window so attachments and rows stay
  as consistent as two separate stores allow — this is **not** atomic: a
  few seconds of drift between the WAL cutoff and the object snapshot is
  possible. Repair path: the orphan object-hash scan (attachments pointed at
  objects that restored rows reference but the snapshot missed) is
  documented in the storage schema — re-upload those from the pre-snapshot
  source of truth.
- Take a dump before every upgrade step (see [upgrade](../how-to/upgrade.md)
  step 3). The upgrade preflight runs against a database copy — supply it
  with a fresh one.

### WAL-G specifics (PITR)

1. Copy `deploy/backup/wal-g.env.example` → `wal-g.env` (compose: same for
   restic), fill in the Garage/S3 coordinates, and build with WAL-G:
   `docker compose --profile backup build --build-arg INSTALL_WAL_G=1 backup`.
2. Enable WAL archiving by starting postgres with
   `-c archive_command='wal-g wal-push %p'` — compose-override file
   documented in `deploy/backup/compose.walg.example.yml`.
3. `wal-g backup-push` runs on the scheduled nightly pass. Retention via
   `WALG_RETAIN_FULL` (the runner applies `wal-g delete retain FULL <n>`
   after each push).
4. **Alert on archive_command silence.** A broken archive pipe fails
   silently and quietly degrades you from "any second" back to "last night."
   If WAL shipping shows no progress for >24h (check the archiver's
   `pg_stat_archiver` view), page yourself.

## Restore

1. Stop the services (`docker compose … stop core-api worker realtime`).
2. For the last-night tier: plain `pg_restore` (below). For PITR: follow
   [wal-g restore](https://github.com/wal-g/wal-g/blob/master/README.md) —
   stop Postgres, `wal-g backup-fetch`, replay WAL to the recovery target,
   then start. Stop at a point **after** the damage, not before it
   (`recovery_target_time` defaults to "latest consistent state").
3. Rehearse before you need it:

   ```sh
   sh deploy/backup/restore.sh /tmp/meridian.dump
   ```

   It restores into a scratch DB and asserts the restore is real: the schema
   lands (with RLS enabled on every org-scoped table), and when the source
   stack is reachable the row counts of every public table match, which catches
   a dump that loses rows in transit. Set `PGDATABASE=meridian` (plus
   `PGSOURCE_HOST`/`PGSOURCE_PORT` against a different server) to turn the
   parity check on. Without it the leg stops after the schema checks. It
   drops the scratch DB when done — a backup that has never restored is not
   a backup.
4. Restore the garage data/meta volumes or bucket contents if you use object storage.
5. Start the services. Do **not** re-run migrations between restore and
   start unless the restored dump predates a schema change you have since
   applied — migrations are forward-only and append-only, history is never
   edited, so any deployed database replays them identically.

## Verify a restore

Boot a scratch stack pointed at the restored volume and log in. Confirm: an
open ticket renders in the workbench, today's time entries exist, and
`/audit` shows the append-only ledger intact. The upgrade-fixture CI
restores are production-like snapshots — treat your restore rehearsal the
same way, not as a theoretical step.
