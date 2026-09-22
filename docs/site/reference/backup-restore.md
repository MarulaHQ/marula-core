# Runbook: backup and restore

> **Audience:** site operator / MSP admin — you host Meridian yourself and
> own its deployment.
>
> This is the first authored page of the `deploy/runbooks/` series; the
> scaffold and future runbook index live at `deploy/runbooks/README.md`.

Everything in Meridian lives in the Postgres database, and once S3
object storage is enabled also the MinIO/S3 bucket. Back both up or
back neither up alone: document attachments (content-addressed uploads) and
versioned documents reference object storage, while everything else —
tickets, time, invoices, FX rates, the provenance and audit ledges, AI
decision answers, customization rows — is rows in Postgres.

## Backup

Schedule a full logical dump:

```sh
docker compose -f deploy/docker-compose.yml exec postgres \
  pg_dump -U meridian -d meridian -Fc -f /tmp/meridian.dump
docker cp meridian-postgres-1:/tmp/meridian.dump meridian-$(date +%F).dump
```

Notes the runbook enforces:

- Dump as the table owner (`meridian`), not `meridian_app` — the
  RLS-constrained app role must never be able to read across tenants, which
  also means it cannot produce a complete dump.
- If you run `OBJECT_STORE=s3` with MinIO, snapshot its data volume (or use
  `mc mirror`) in the same window so attachments and rows stay consistent.
- Take a dump before every upgrade step (see [upgrade](../how-to/upgrade.md)
  step 3). The upgrade preflight runs against a database copy — supply it
  with a fresh one.

## Restore

1. Stop the services (`docker compose … stop core-api worker realtime`).
2. Recreate the database and restore:

   ```sh
   docker compose -f deploy/docker-compose.yml exec postgres \
     pg_restore -U meridian -d meridian --clean --if-exists /tmp/meridian.dump
   ```

3. Restore the MinIO volume / bucket contents if you use object storage.
4. Start the services. Do **not** re-run migrations between restore and
   start unless the restored dump predates a schema change you have since
   applied — migrations are forward-only and append-only, history is never
   edited, so any deployed database replays them identically.

## Verify a restore

Boot a scratch stack pointed at the restored volume and log in. Confirm: an
open ticket renders in the workbench, today's time entries exist, and
`/audit` shows the append-only ledger intact. The upgrade-fixture CI
restores are production-like snapshots — treat your restore rehearsal the
same way, not as a theoretical step.
