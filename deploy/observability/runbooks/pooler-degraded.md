# Pooler degraded: connections queueing, statements missing, or a listener gone quiet

**Reached from:** `MarulaApiLatencyBudgetBreach`, `MarulaWorkerNotReporting`

## What this means

There is no alert for the pooler, because there is no series for it. PgBouncer does
expose `SHOW STATS`, and nothing in the pack scrapes it. So the pooler arrives here as
one of three symptoms behind somebody else's alert, and two of the three are silent.

<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:1ffa9db30e223db26d333f17771e9f81957a6d9b74fbff236cbce6046555cca2 -->

The `pooler` profile in `deploy/pooler/pgbouncer.ini` uses transaction mode.
Each (database, user) pair has 15 server connections by default
(`default_pool_size = 15`). If a client waits longer than the default
`reserve_pool_timeout` of 5 seconds, the pool can use up to five additional
connections (`reserve_pool_size = 5`), for a total of 20. The hard ceiling is
40 server connections per database across all users (`max_db_connections = 40`).
The pooler runs the reset query after every transaction. PgBouncer tracks prepared
statements (`max_prepared_statements = 100`); the application's own cache is off
(`MARULA_DB_STATEMENT_CACHE = 0`), so a statement name does not outlive the
transaction that created it. A default install dials Postgres directly and has no
pooler to degrade, so this page applies only when that profile is on.

<!-- /coderabbit-ai-generated:v1 -->
The reasoning, with measurements, is [connection pooling](../../../docs/reference/connection-pooling.md).

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
   the test `the_statement_cache_fits_the_pooler_profile` in
   `crates/marula-core-cli/src/deploy.rs` pins that default against the pooler's
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
