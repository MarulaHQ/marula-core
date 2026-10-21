# Postgres down or unreachable

**Reached from:** `MarulaWorkerNotReporting`, `MarulaOutboxLagHigh`, `MarulaApiAvailabilityBurnFast`

## What the alert means

The worker's queue loop stopped reporting, or the API started returning 5xx, and
Postgres is the first thing to rule in or out. Every stage of the product reads
the database, so a database that answers nothing looks like six unrelated
failures at once.

`GET /api/v1/health` will still answer `200 ok` while this is happening. That
endpoint is a liveness probe for the API process and says nothing about the
database, so it is not evidence either way.

## First three checks

1. Is the server answering, and is it the server or the credentials:

   ```sh
   docker compose exec postgres pg_isready -U marula -d marula
   docker compose exec core-api env | grep -c '^DATABASE_URL='
   ```

   `pg_isready` failing is the server. A server that answers while the worker
   logs `password authentication failed` is a credential or role problem, and
   restarting will not fix it.

2. Is the server up but exhausted? Connections, not CPU, are the usual ceiling:

   ```sh
   docker compose exec postgres psql -U marula -d marula -c \
     "SELECT state, count(*) FROM pg_stat_activity GROUP BY state;"
   docker compose exec postgres psql -U marula -d marula -c \
     "SHOW max_connections;"
   ```

   A large `idle in transaction` count means a caller is holding sessions open.
   Sessions at `max_connections` means new work is refused rather than slowed,
   and the answer is the pooler profile in `docs/perf.md` §5, not a restart.

3. Is the volume full or the disk out of space? A full disk turns into a
   read-only database with very little in the log:

   ```sh
   docker compose exec postgres df -h /var/lib/postgresql/data
   docker compose exec postgres psql -U marula -d marula -c \
     "SELECT pg_size_pretty(pg_database_size('marula'));"
   ```

## Escape hatch

Nothing in the worker needs manual intervention once the database answers again.
The worker's failed loops back off for two seconds and retry, the outbox keeps
the rows it has not published, and the consumers resume from their pending lists
and the live stream tail. Restarting the worker in a loop is the one action that
makes recovery slower, because each start replays listener setup.

If the database is gone rather than slow, stop the product from writing to it and
go to `restore-from-backup.md`. If it is up but out of connections, raise
`MARULA_DB_POOL_MAX` on the API and the worker only after checking what is
holding the sessions, since two replicas at a larger pool reach the ceiling
sooner.

A restore that lands in the same volume with the same credentials restores the
service. A restore that lands in a new volume needs the connection string moved
first, or every process keeps talking to the empty one.

## Afterwards

Record whether this was availability, capacity or credentials. The last two are
the ones that return, and the alert that fires first tells you which surface went
quiet.
