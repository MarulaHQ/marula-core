# PG17 → PG18 cut runbook (existing pgdata volume)

Applies to both deployment shapes proven by Wave 36 lane 2
(`p36-infra-transition`): the self-host compose stack and the desktop test
app's embedded Postgres. Both now run PG18 (`pgvector/pgvector:pg18`).
A cluster created on PG17 will not boot under 18's binaries, so an existing
volume needs a one-time cut.

## Which leg to use

| Situation | Leg |
| --- | --- |
| Compose stack, small/dev data, downtime OK | dump/restore (§1) |
| Large data, or you want stats carried over | pg_upgrade (§2) |

`pg_upgrade` retains optimizer statistics, so queries don't grind on
cold plans after cutover. Dump/restore is simpler and enough below a few
GB. Both are forward-only — take a backup first either way
(`deploy/backup/`, or `pg_dumpall` in §1 is itself the backup).

## 0. Detect

```sh
docker run --rm -v marula_pgdata:/v pgvector/pgvector:pg18 cat /v/PG_VERSION 2>/dev/null \
  || docker run --rm -v marula_pgdata:/v alpine cat /v/data/PG_VERSION
```

`17` (or lower) → cut needed. `18` → nothing to do.

## 1. Dump/restore leg (compose) — proven 2026-09-23

The PG18 image also changed its volume layout (`/var/lib/postgresql/18/…`,
pg_ctlcluster style) — mount the VOLUME ROOT, not `/data` (already applied
in `deploy/docker-compose.yml`).

1. **Start the old cluster with its own binaries and dump it.** The volume
   keeps the old `pg17` image usable:

   ```sh
   docker run -d --name p17-cut -v marula_pgdata:/var/lib/postgresql \
     pgvector/pgvector:pg17 sleep 600
   docker exec p17-cut su postgres -c \
     "pg_ctl -D /var/lib/postgresql -o '-p 5499' -w start"
   docker exec p17-cut su postgres -c \
     "pg_dumpall -p 5499 -U marula > /tmp/dump.sql"
   docker cp p17-cut:/tmp/dump.sql ./marula-pg17-dump.sql
   docker exec p17-cut su postgres -c "pg_ctl -D /var/lib/postgresql -m fast -w stop"
   docker rm -f p17-cut
   ```

   `-U marula` (not `postgres`) — the compose bootstrap superuser is
   `marula`; there is no `postgres` role.

2. **Wipe the volume and start PG18** (the dump IS the backup):

   ```sh
   docker compose -f deploy/docker-compose.yml down
   docker volume rm marula_pgdata
   docker compose -f deploy/docker-compose.yml up -d postgres
   ```

3. **Restore.** `CREATE ROLE marula` fails benignly (the fresh initdb
   already made it a superuser) — run without `ON_ERROR_STOP` and filter:

   ```sh
   docker compose -f deploy/docker-compose.yml cp marula-pg17-dump.sql postgres:/tmp/dump.sql
   docker compose -f deploy/docker-compose.yml exec -e PGPASSWORD=marula postgres sh -c \
     'psql -U marula -d postgres -v ON_ERROR_STOP=0 -f /tmp/dump.sql 2>&1 \
      | grep -i error | grep -iv "already exists"'
   ```

   The grep must print NOTHING except role/database-already-exists lines.

4. **Verify + bring the stack up:**

   ```sh
   docker compose -f deploy/docker-compose.yml exec postgres psql -U marula -d marula \
     -tc "SELECT version FROM _sqlx_migrations ORDER BY version DESC LIMIT 1;"
   # → 0102
   docker compose -f deploy/docker-compose.yml up -d
   ```

## 2. pg_upgrade leg (in place, keeps statistics)

Run from a throwaway container with BOTH major versions' binaries mounted
(17's binaries live in the old image). `--link` halves the copy time but
requires old+new on one filesystem — hence the volume-root mount from §1:

```sh
pg_upgrade -b <pg17-bin> -B <pg18-bin> \
  -d /var/lib/postgresql/17 -D /var/lib/postgresql/18 \
  -j 2 --link
```

Then `ANALYZE` per database (pg_upgrade restores stats for the planner but
`vacuumdb --all --analyze-in-stages` is still recommended), and delete the
old version directory only after a soak period.

## 3. Desktop test app (embedded Postgres)

The desktop app cuts automatically: on launch, `desktop/src/main.rs` sees
`pgdata/PG_VERSION` = 17, starts the old cluster with its retained 17
binaries (`pginstall/17.11.0/`), `pg_dumpall`s it, archives the directory
to `pgdata-pg17.bak`, initializes a fresh 18 cluster, and replays the dump
before running migrations. No operator action; the archive stays for
rollback to the previous app build. Verified live: PG17 cluster with data →
PG18.6 serving, migrations at 0102, demo seed intact.

## Post-cut checklist

- [ ] `_sqlx_migrations` head is `0102`
- [ ] `SELECT version();` → `PostgreSQL 18.x`
- [ ] ticket list renders (p95 spot check: `scripts/perf-smoke.sh`)
- [ ] `vacuumdb --all --analyze-in-stages` run (§1 leg only)
