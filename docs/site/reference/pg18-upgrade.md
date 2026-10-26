---
description: "Marula runs Postgres 18 + pgvector everywhere since the 2026-09 transition (deploy/docker-compose.yml, the Helm chart, and the desktop test app all pin…"
---

# Upgrading Postgres 17 to 18

Marula runs Postgres 18 + pgvector everywhere since the 2026-09 transition
(`deploy/docker-compose.yml`, the Helm chart, and the desktop test app all
pin `pgvector/pgvector:pg18` or its embedded equivalent). This page is the
cut procedure for an existing PG17 volume: what to run, what breaks, and how
to check the result.

Two paths:

| Path | Downtime | When to pick it |
|---|---|---|
| dump / restore | dump + restore window (minutes for small orgs) | default choice. Simple, portable, verifiable |
| `pg_upgrade` in place | seconds of read-only cutover | large clusters where dump time hurts |

## Before you touch anything

> [!WARNING]
> Do not start the cutover until a backup has restored once. A backup that has
> never restored is not a backup.

1. Take a backup that has actually restored once. If you use the backup
   profile, run `deploy/backup/restore.sh` against last night's dump first.
   A backup that has never restored is not a backup.
2. Stop writes. In compose, `docker compose stop core-api worker web`.
3. Note the current major: `cat <pgdata>/PG_VERSION` should print `17`.

## Path A — dump / restore (compose)

The PG18 container images changed their data layout: since the 18 images,
the docker-library entrypoint expects `pg_ctlcluster`-style
major-version directories under the volume root. A volume that PG17 wrote
directly into `/var/lib/postgresql/data` will not boot under the 18 image,
and the entrypoint says so. Two consequences:

- New installs should mount the volume at `/var/lib/postgresql` (the
  compose file does now), so a future 18 → 19 cut can use `pg_upgrade`
  without hitting a mount-point boundary.
- An old volume needs the dump / restore hop below regardless.

```sh
cd deploy

# 1. Dump with the OLD binaries: the 18 image refuses to start this volume.
docker run --rm --name pg17-cut -v marula_pgdata:/var/lib/postgresql \
  pgvector/pgvector:pg17 sleep 600 &
sleep 8
docker exec pg17-cut su postgres -c \
  "pg_ctl -D /var/lib/postgresql/data -o '-p 5499' -w start"
docker exec pg17-cut su postgres -c \
  "pg_dumpall -p 5499 -U <superuser> > /tmp/dump.sql"
docker exec pg17-cut su postgres -c \
  "pg_ctl -D /var/lib/postgresql/data -m fast -w stop"
docker cp pg17-cut:/tmp/dump.sql ./pg17-dump.sql
docker rm -f pg17-cut

# 2. Fresh PG18 volume. The compose mount must be /var/lib/postgresql.
docker compose down
docker volume rm marula_pgdata        # the dump is your safety net
docker compose up -d postgres
docker compose cp ./pg17-dump.sql postgres:/tmp/dump.sql
docker compose exec postgres sh -c \
  'psql -U marula -d postgres -v ON_ERROR_STOP=0 -f /tmp/dump.sql' \
  2>&1 | grep -iv "already exists"
```

Expect `role "marula" already exists` style errors: the fresh initdb
created the superuser, and the dump re-creates it. Those are the only
errors you should see. Then:

```sh
docker compose up -d
docker compose exec postgres psql -U marula -d marula \
  -c "SELECT version FROM _sqlx_migrations ORDER BY version DESC LIMIT 1;"
docker compose exec postgres psql -U marula -d marula -c "ANALYZE;"
```

The migration head should read `102`. `ANALYZE` matters: dump / restore
does not carry optimizer statistics, and without it the first queries run
on guesses. pg_upgrade (path B) does not have this problem.

## Path B — pg_upgrade in place

Faster cutover, more moving parts. Run it with both binary sets available,
both servers stopped, and the new cluster initialized empty:

```sh
/usr/lib/postgresql/18/bin/pg_upgrade \
  --old-bindir /usr/lib/postgresql/17/bin \
  --new-bindir /usr/lib/postgresql/18/bin \
  --old-datadir <old-pgdata> --new-datadir <new-pgdata> \
  --link --check          # dry-run first, then drop --check
```

`--link` hard-links instead of copying, which keeps the cutover fast.
The old cluster must stay untouched afterwards until you delete it. Since Postgres 17, pg_upgrade
carries optimizer statistics over, so there is no slow-plans grind after
cutover. Keep the old datadir around until you have verified the app.

On Docker the mount-point boundary gets in the way of `--link`. If your
volume mounts at a subdirectory of a single volume, follow the
docker-library note about mounting at `/var/lib/postgresql` first.

## The desktop test app cuts itself

The macOS test app embeds Postgres and upgrades its own data dir. On
startup it reads `pgdata/PG_VERSION`. When the stored major differs from the
pinned one it starts the old cluster with the retained old binaries, runs
`pg_dumpall`, archives the old data dir to `pgdata-pg17.bak`, initializes
the new cluster, and replays the dump before migrations. You get the same
guarantees as path A with the old volume kept for rollback. The cut log
lands in `pg17-cut.log` inside the app data dir
(`~/Library/Application Support/Marula/`).

## Garage note (unrelated to Postgres but same window)

Garage rejects an RPC secret that is not 32 bytes of hex. The dev secret in
the compose file is hex-encoded on purpose. If you bring your own, generate
it with `openssl rand -hex 32` or the container dies at first boot with
`Invalid RPC secret key`.

## After the cut

- Re-run the perf smoke (`scripts/perf-smoke.sh`) — it now spins PG18. The
  ticket-list p95 at 50k tickets measured 14–15 ms after the transition
  (keyset page at offset 40k). The PG17 baseline was 29.9 ms and the budget
  is 150 ms.
- Core-db suite green on 18: 76/76 (`cargo test -p core-db --test db_suite`).
- Delete the archived PG17 volume only after a week of clean operation.
