# Pooler degraded: connections queueing, statements missing, or a listener gone quiet

**Reached from:** `MarulaApiLatencyBudgetBreach`, `MarulaWorkerNotReporting`

## What this means

There is no alert for the pooler, because there is no series for it. PgBouncer does
expose `SHOW STATS`, and nothing in the pack scrapes it. So the pooler arrives here as
one of three symptoms behind somebody else's alert, and two of the three are silent.

The shipped topology is in `deploy/pooler/pgbouncer.ini`: transaction mode, 15 server
connections per (database, user) pair (`:60`), 40 in total (`:63`), the reset query
after every transaction (`:53`), and the prepared-statement cache off (`:54`). The
reasoning, with measurements, is [connection pooling](../../../docs/reference/connection-pooling.md).

The symptom that costs the most time is the quiet one. A `LISTEN` attached to a pooled
connection receives nothing and reports no error, so live push and the relay's wakeup
both stop while every request keeps succeeding.

## First three checks

1. Is the pooler the bottleneck, or is Postgres:

   ```sh
   psql "postgres://marula@127.0.0.1:6432/pgbouncer" -c "SHOW POOLS" -c "SHOW STATS"
   ```

   Read `cl_waiting` against `sv_active` and `sv_idle`. Clients waiting while servers sit
   idle means the pools are exhausted by long holders, which is usually a report rather
   than a bug. Clients waiting with no idle servers means Postgres is the constraint and
   this runbook is the wrong page.

2. Does the API log carry a prepared-statement error:

   ```sh
   docker compose logs --tail=200 core-api | grep -i "prepared statement"
   ```

   `prepared statement "P_0" does not exist` means the statement cache is on behind a
   transaction pooler. `MARULA_DB_STATEMENT_CACHE` has to be `0`, and
   `the_statement_cache_fits_the_shipped_pooler` in
   `crates/marula-core-cli/src/deploy.rs:2078` pins that default against the pooler's
   `max_prepared_statements`. Fix the setting, not the query.

3. Is anything still listening:

   ```sql
   SELECT pid, usename, state, query
     FROM pg_stat_activity
    WHERE query ILIKE 'listen%';
   ```

   In the split profile that list belongs on the direct connection, which
   `MARULA_LISTEN_DATABASE_URL` names. Empty while the product is up means the listener
   is either pointed at the pooler or not started, and the fix is the environment rather
   than the database.

## Escape hatch

Roll back to a direct connection, which `deploy/pooler/README.md` documents under
Rollback: point `DATABASE_URL` back at Postgres (`MARULA_APP_DATABASE_URL` overrides it
in compose), clear `MARULA_LISTEN_DATABASE_URL`, and leave
`MARULA_DB_STATEMENT_CACHE` at `0` unless you have made room for a cache.

The ceiling comes back with the rollback, and
[connection pooling](../../../docs/reference/connection-pooling.md) lists it: one
instance's pools total 54 and two total 108, against a default Postgres of 100. That
gap is the number the pooler exists to remove.

## Afterwards

Record which of the three it was, and the numbers from `SHOW POOLS` at the time. Two
follow-ups are worth a row rather than a note: a scrape target for the pooler, and an
alert on `cl_waiting`, since the pack currently cannot tell a queue from a slow query.

If you found `server_reset_query_always` at `0`, stop and treat it as a tenant-isolation
fault. The setting is what makes a pooled connection safe to hand to another tenant, the
measurement is in [connection pooling](../../../docs/reference/connection-pooling.md),
and no performance reason justifies the window it opens.
