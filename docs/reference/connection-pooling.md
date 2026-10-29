# Connection pooling

A pooler goes between the application pools and Postgres. It exists for one
number: connections. `docs/perf.md` §5 sizes the pools, `docs/capacity-model.md`
turns instance counts into that number, and both arrive at the same wall.

The shipped deployment runs PgBouncer in transaction mode. This page is the
reasoning, the two settings that make transaction pooling safe, and the
measurements behind them. The config itself is `deploy/pooler/pgbouncer.ini`;
`deploy/pooler/README.md` is how to run it.

## Why the ceiling arrives first

Each process opens pools of its own. At the defaults that was 71 connections per
instance: 20 for the API's app pool, 25 for its reporting pool, one for the
`LISTEN` connection, 20 for the worker and 5 for the worker's delivery pool.
Postgres defaults to `max_connections = 100`, so the second instance asks for
142 against 100 and the fleet stops there — before CPU matters, and long before the
capacity model's 366-user-per-instance ceiling does.

The reporting pool was the second-largest line in that table and the least
justified. 25 connections for a surface a user opens occasionally, against the
hot path's measured saturation at concurrency 8 (§5), was headroom nobody had
asked for. It is 8 now: the hot path's own proven concurrency, which is a floor
a cold surface can live on and a number an operator can defend.

| Process | Pool | Before | After | Env |
| --- | --- | --- | --- | --- |
| core-api | app | 20 | 20 | `MARULA_DB_POOL_MAX` |
| core-api | reporting (RLS role) | 25 | 8 | `MARULA_REPORTING_POOL_MAX` |
| core-api | LISTEN | 1 | 1 (direct) | — |
| worker | app (owner role) | 20 | 20 | `MARULA_DB_POOL_MAX` |
| worker | delivery reporting | 5 | 5 | `DELIVERY_POOL_MAX` |
| **client total, one instance** | | **71** | **54** | |
| **two instances** | | **142** | **108** | |

Client connections are what the application opens. What Postgres pays for is the
server side, and that is the pooler's number now: 15 per (database, user) pair,
so 30 across the app role and the owner role, however many instances are added.
Measured through the shipped config: 40 concurrent sessions held a peak of
15 server connections.

## Transaction pooling, not session pooling

A session pooler maps one client connection to one server connection for the
client's whole life. Our clients are pools that hold their connections open
permanently, so session pooling would change nothing for the first 15 of the
client connections and leave the rest queued behind them. Transaction pooling hands the server
connection back after every transaction, which is the only shape that
multiplexes a pool of pools.

The price is that session state cannot survive a transaction. Two kinds of state
in this codebase would have broken silently, and both are handled:

- **`LISTEN`.** The API's live-delivery connection and the worker's relay wakeup
  are subscriptions. A subscription attached to a connection that changes hands
  is a subscription nobody receives on, with no error anywhere. Both processes
  now dial `MARULA_LISTEN_DATABASE_URL` for that one connection, straight to
  Postgres.
- **RLS scope.** Every query binds its org with
  `set_config('app.org_id', …, true)`, which is transaction-local. That works
  through a pooler. It is also the claim this page refuses to assert, so it was
  measured — below.

## The two settings

### `server_reset_query = DISCARD ALL`, and why it needs its companion

PgBouncer only runs the reset query in session pooling unless
`server_reset_query_always = 1`, so the shipped config sets both. With the reset
off, the pooler is a tenant-isolation hole. Two clients, one server connection
(`default_pool_size = 1`), the first leaving a session-level setting behind:

```
-- client 1
SET app.org_id = '11111111-1111-1111-1111-111111111111';

-- client 2, no binding of its own
SELECT coalesce(current_setting('app.org_id', true), '<NULL>');
SELECT count(*) FROM tickets;
```

| Reset query | Client 2 inherits | It reads |
| --- | --- | --- |
| off | `11111111-…` (tenant A) | tenant A's ticket: 1 row |
| on | empty | `ERROR: invalid input syntax for type uuid: ""` |

The second row is the point. A reset that runs after every transaction leaves no
tenant value to inherit, so the worst case is an error, not another tenant's
rows. The error is honest fail-closed behaviour: `DISCARD ALL` clears the value
but leaves the custom setting *defined* as an empty string, so a query that
forgot to bind raises `22P02` instead of returning nothing. That behaviour
predates the pooler, and `crates/core-db/tests/common/mod.rs` documents it.
It is loud in the right direction.

The cost of running it after every transaction is measurable: `pgbench` at four
clients on a four-statement transaction went from 889 transactions per second
with the reset off to 738 with it on. Seventeen percent, bought back by the
connection ceiling this whole page is about.

### Prepared statements: the cache is off, and it has to be

sqlx caches prepared statements per connection and reuses them by name. A name
lives on the *server* connection. Transaction pooling gives that connection to
someone else after every transaction, and the reset query deallocates the
statement, so the next transaction's `Bind` names something that no longer
exists. Measured through the shipped config:

| Statement shape | Result |
| --- | --- |
| parse once, reuse by name (`pgbench -M prepared`, sqlx cache > 0) | 0 transactions, `ERROR: prepared statement "P_0" does not exist` |
| parse and execute inside the transaction (`pgbench -M extended`, sqlx cache 0) | 8093 transactions, 1619 tps, 2.47 ms average |

`MARULA_DB_STATEMENT_CACHE` defaults to 0, which disables the cache. sqlx then
parses under a fresh name and executes inside the same transaction, which is the
shape the pgbench row above passed. `crates/core-db/tests` asserts the capacity
reaches sqlx's connect options rather than dying in a struct.

A bounded cache is possible instead, and it is the right answer for a deployment
that runs *without* a pooler: set `max_prepared_statements` above the cache size
and stop running the reset query after every transaction. Both halves have to
move together. PgBouncer's own statement tracking kept named statements working
in that arrangement, but the same measurement showed almost none of the benefit
once the reset query runs anyway, so the shipped config does not ask for it. A
test in `crates/marula-core-cli/src/deploy.rs` pins the pair: the descriptor's
cache default, the pooler's `max_prepared_statements`, and the reset setting must
agree.

What disabling it costs, with the reset query off on both sides so the only
difference is the parse: 628 transactions per second cached against 572
uncached at four clients, 6.37 ms against 6.99 ms average on the same
four-statement transaction. About 0.15 ms per statement. That is the price of
the shape that works, and `scripts/perf-smoke.sh` now measures it — if its
budget ever trips, the fix is to pair a cache with the pooler rather than to
turn the reset query off.

## RLS through the pooler, measured

The claim under test: `set_config(…, true)` is transaction-local and sqlx pins a
transaction to one connection, so an org's rows stay its own behind a transaction
pooler. Asserting that would have been cheap and worthless, so two tenants were
seeded and queried through the pooler.

Each transaction binds one tenant, counts its own rows, then asks for the *other*
tenant's row by primary key:

```sql
BEGIN;
SELECT set_config('app.org_id', :'org_id', true);
SELECT count(*) FROM tickets;                          -- own rows
SELECT count(*) FROM tickets WHERE org_id = :'other';  -- the other tenant's row
COMMIT;
```

Through the shipped pooler as `marula_app`:

| Run | Transactions | Correct | Cross-tenant rows returned |
| --- | --- | --- | --- |
| alternating tenants, serial | 40 | 40 | 0 |
| 8 concurrent workers, alternating tenants | 160 | 160 | 0 |
| direct to Postgres, as a control | 10 | 10 | 0 |

Every transaction saw exactly one row of its own tenant and zero of the other's,
including the explicit attempt to name the other tenant's UUID. The pooler did
not change the answer. "RLS cannot work behind a pooler" is folklore about
session-scoped bindings; this codebase binds per transaction.

## What the pooler does not fix

- **Long-running reports still hold a client connection** for the length of the
  query. The pooler bounds what Postgres sees, not how many reports one instance
  can run at once.
- **The connection ceiling still exists, one layer down.** `max_db_connections`
  is 40 in the shipped config. Raising instances past what 40 server connections
  can serve moves the queue from Postgres to the pooler.
- **Migrations stay direct.** The `migrate` job and `pg_dump` dial Postgres
  themselves; DDL and advisory locks want a session nobody else takes.
