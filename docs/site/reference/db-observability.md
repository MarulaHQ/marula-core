# Database observability

Marula measures performance one endpoint at a time. That leaves the real query
mix unseen and the slow tail unmeasured, so the database now records both. This
page is the access path: what is collecting, how to read it, and what a managed
Postgres will not let you turn on.

Three names matter. Everything else on this page explains them.

| Name | What it is |
|---|---|
| `ops.db_signals` | One row per signal: `installed`, `active`, `note`. Ask this before reading anything else |
| `ops.query_tail` | The statements run against this database, worst total time first |
| `ops.explain_plan(query, with_analyze)` | The plan of one statement, as `jsonb` |

```sql
SELECT signal, installed, active FROM ops.db_signals;
```

`installed` means the extension's SQL objects exist. `active` means the server
is doing the work. The two differ on a managed instance where someone ran
`CREATE EXTENSION pg_stat_statements` without preloading the library: the
views exist and reading them fails. That case is why this view is the first
thing to read.

## What ships where

The stack is split in two because a database server and a migration change
different things.

| Piece | Ships as | Applies to |
|---|---|---|
| `shared_preload_libraries=pg_stat_statements,auto_explain` | container start arguments in `deploy/docker-compose.yml`, the Helm chart, and `scripts/e2e-stack.sh` | any server we start |
| `auto_explain` thresholds and format | same start arguments | any server we start |
| `io_method=worker` | same start arguments | any server we start |
| `pg_cron` and `hypopg` binaries | `deploy/postgres/Dockerfile`, used through `deploy/postgres/compose.extensions.yml` | opt-in overlay only |
| `CREATE EXTENSION` for all three | migration `0145_db_observability_extensions.sql` | every install, including managed |
| `ops.db_signals`, `ops.query_tail`, `ops.explain_plan` | same migration | every install |
| Scheduled `VACUUM (ANALYZE)` jobs | same migration, only where `pg_cron` is installed | installs with pg_cron |

`pg_stat_statements` and `auto_explain` are part of the PostgreSQL contrib set,
so both are already inside the pinned image. `pg_cron` and `hypopg` are not,
and the hardened image the compose stack runs by default has no package
manager. The overlay exists for deployments that want them; it starts from the
official pgvector image instead of the hardened one, which is the trade it
makes. A deployment that skips the overlay gets the statistics, the plan logs,
the views and the function, and no scheduled jobs.

Every extension in the migration is created behind two guards: the server must
offer it, and the command itself is wrapped so a refusal becomes a notice
instead of a failed upgrade. A database that forbids an extension still
upgrades; it just reports `active = false`.

## Reading the query mix

```sql
SELECT calls, mean_ms, total_ms, pct_time, rows_per_call, left(query, 80)
  FROM ops.query_tail
 LIMIT 20;
```

`ops.query_tail` is ordered by total time, not mean time. On a small install
the expensive statement is usually a cheap one run a great many times, and mean
time hides that. `pct_time` says how much of the measured server time one
statement owns. `rows_per_call` is the shape check: a page query returning 20
rows and one returning 4,000 differ there before anyone reads a plan.

The view covers the current database only, and it leaves out statements that
read the statistics, so the observer does not climb its own list.

Two things it does not do:

- **It does not reset.** `pg_stat_statements` accumulates until reset, so the
  numbers are since the last restart or reset, not a rolling window. When you
  want a clean window, run `SELECT pg_stat_statements_reset();` yourself. A
  scheduled reset would delete the evidence someone is in the middle of
  reading.
- **It does not track statements inside functions by default.** PostgreSQL's
  default is `pg_stat_statements.track = top`. Set it to `all` on the server to
  see nested statements, and expect the extra bookkeeping.

## The EXPLAIN signal

`ops.explain_plan()` returns the plan of one statement as `jsonb`.

```sql
-- shape only, does not run the statement
SELECT jsonb_pretty(ops.explain_plan('SELECT * FROM tickets WHERE org_id = $1 LIMIT 20'));

-- measured: runs it, with buffers, under a 5s statement timeout
SELECT jsonb_pretty(ops.explain_plan('SELECT count(*) FROM tickets', with_analyze => true));
```

If you are writing something that consumes a plan, this is the function to
call. The plan is the `Plan` object PostgreSQL's `EXPLAIN (FORMAT JSON)`
returns, so `#>> '{Plan,Plans,0,Node Type}'` is the top node and the rest of the
usual JSON path rules apply.

Three properties are deliberate:

- **Invoker rights.** `with_analyze => true` executes the statement, so a
  definer-rights function would let any caller write with the owner's
  privileges. This one can do exactly what the caller could have typed.
- **A 5s timeout on the measured form.** An accidental `EXPLAIN ANALYZE` of a
  whole-table aggregate stops instead of holding a connection.
- **Its own `search_path` (`public`).** Schema-qualify anything else, for
  example `ops.v1_tickets`.

The second parameter is named `with_analyze` because `analyze` is a reserved
word and cannot be a parameter name.

For the tail that nobody asked about, `auto_explain` writes the plan of any
statement slower than `auto_explain.log_min_duration` (200ms in the shipped
configuration) to the server log as JSON. On compose:

```sh
docker compose logs postgres | grep 'duration:'
```

200ms sits above the 150ms page budget on purpose: only a statement that has
already broken the budget writes a plan. Lower it to 50ms while chasing a
regression, raise it on slower hardware. The full set of arguments is in
`deploy/docker-compose.yml` and the Helm chart, and the same lines are in
`docs/perf.md`.

## Scheduled maintenance

Where `pg_cron` is installed, migration 0145 schedules six jobs, ten minutes
apart from 03:10 local server time:

| Job | Table |
|---|---|
| `marula-vacuum-tickets` | `tickets` |
| `marula-vacuum-jobs` | `jobs` |
| `marula-vacuum-outbox-events` | `outbox_events` |
| `marula-vacuum-audit-events` | `audit_events` |
| `marula-vacuum-search-index` | `search_index` |
| `marula-vacuum-consumer-deliveries` | `consumer_deliveries` |

```sql
SELECT jobname, schedule, command, active FROM cron.job ORDER BY jobid;
```

Each one is a plain `VACUUM (ANALYZE)`: no `FULL`, no freeze override, nothing
that holds an exclusive lock. One table per job and ten minutes apart so two
vacuums never compete for the same I/O window. Re-running the migration
replaces a job by name rather than duplicating it.

`pg_cron` runs SQL. It cannot run the worker's Rust sweeps, so the housekeeping
loops described in [`docs/worker-cadence.md`](../../worker-cadence.md) stay in
the worker. What moved into the database is the maintenance that was never a
sweep.

## Trying a candidate index without building it

`docs/perf.md` F4 measured a candidate index on the ticket list at 0.66ms
against 31ms and deliberately did not ship it, because page-1 p95 has headroom.
`hypopg` makes that index visible to the planner for one session without
writing anything.

```sql
SELECT hypopg_create_index($$
  CREATE INDEX ON tickets (
    org_id,
    (CASE priority WHEN 'p1' THEN 0 WHEN 'p2' THEN 1 WHEN 'p3' THEN 2 ELSE 3 END),
    due_at NULLS LAST, created_at, id
  )
$$);

SELECT index_name FROM hypopg_list_indexes;

SET enable_seqscan = off;   -- only when measuring on a small fixture
SELECT ops.explain_plan($$
  SELECT id FROM tickets
   WHERE org_id = '00000000-0000-0000-0000-000000000001'::uuid
   ORDER BY (CASE priority WHEN 'p1' THEN 0 WHEN 'p2' THEN 1 WHEN 'p3' THEN 2 ELSE 3 END),
            due_at NULLS LAST, created_at, id
   LIMIT 20
$$);

SELECT hypopg_reset();
```

Two facts to keep in mind. The hypothetical index lives in the session that
created it, so the create and the explain have to share a connection. And bind
a literal for the org filter, not `gen_random_uuid()` or another volatile
expression: a volatile predicate stops the planner from using any index, and
the plan will say so.

## Watching the F4 trigger

`docs/perf.md` F4 deferred that index with a condition attached: look again when
page-1 p95 reaches about two thirds of the budget, or when the sort plan
regresses. Both halves are metrics now, emitted by the worker's
`capacity_watch` sample:

| Metric | Unit | Means |
|---|---|---|
| `marula.db.page1_p95` | `ms` | p95 of the endpoint's page-1 statement, timed on the install's largest org |
| `marula.db.page1_sort_index_scan` | `1` | `1` when the ORDER BY is served by an ordered index scan, `0` when the planner materializes the sort |

The plan arm is read from `ops.explain_plan` on the same statement the endpoint
issues, with its binds filled in, so the signal cannot drift from the query. The
latency arm runs warmups and forty timed requests, which is a measurement rather
than a probe: it belongs to a scheduled job, and until one runs, both series are
absent.

Two rules read them. `marula-capacity-trigger.rules.yaml` fires at 100ms, which
is two thirds of the 150ms budget, as a ticket. `rules-optin/marula-capacity-plan-shape.rules.yaml`
watches the plan, opt-in because `0` is the shape F4 measured and accepted, not a
fault. `deploy/observability/runbooks/capacity-trigger.md` is the response to
both.

## Managed Postgres

Checked 2026-10-21 against each provider's current extension documentation. The
column that matters is not "supported" but "what does enabling cost".

| Signal | Self-host (our images) | Amazon RDS | Azure Flexible Server | Cloud SQL |
|---|---|---|---|---|
| `pg_stat_statements` | on | yes — custom parameter group, then reboot | yes — `azure.extensions` allow list plus preload | yes — instance flags |
| `auto_explain` | on | yes — parameter group (`shared_preload_libraries`), then reboot | yes — module, needs preload only, no allow list | yes — `cloudsql.enable_auto_explain` |
| `pg_cron` | overlay image | yes — parameter group plus a reboot, then `CREATE EXTENSION` in the one `cron.database_name` database | yes — allow list plus preload; jobs must live in one database | yes — `cloudsql.enable_pg_cron` |
| `hypopg` | overlay image | yes — create the extension, no preload | yes, but it is **not allow-listed for major-version upgrades**: drop it before a version bump or the upgrade pre-check fails | **no** — not shipped |
| `io_method` | pinned to `worker` | server parameter `io_method`, start-only | server parameter, `worker` or `sync` only | instance flag |

Three consequences worth planning for:

- **On a managed instance, our migration cannot turn any of this on.** Changing
  `shared_preload_libraries` is a control-plane change that ends in a restart,
  and several providers reboot the whole instance. Do it in a maintenance
  window, then let the migration create the extension objects on the next
  deploy. Until then `ops.db_signals` reports `installed = false` or
  `active = false`, and every consumer sees that instead of an error.
- **`pg_cron` is the one most often held back.** No large provider forbids it
  outright today, but it is the extension whose enablement is least like
  flipping a flag: it needs a preload, a restart, and a single named database
  to hold its job table. Small and specialised providers do not ship it at all.
  The self-host images have no such restriction, and `deploy/postgres/Dockerfile`
  is what keeps that true.
- **`hypopg` is the one genuinely forbidden.** Cloud SQL does not ship it. On
  Azure it works but blocks a major-version upgrade until it is dropped, which
  is an unpleasant surprise to find on upgrade day.

Sources: [RDS extension list](https://docs.aws.amazon.com/AmazonRDS/latest/PostgreSQLReleaseNotes/postgresql-extensions.html),
[RDS pg_cron](https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/PostgreSQL_pg_cron.html),
[Azure extension considerations](https://learn.microsoft.com/en-us/azure/postgresql/extensions/concepts-extensions-considerations),
[Cloud SQL extensions](https://docs.cloud.google.com/sql/docs/postgres/extensions).

## PostgreSQL 18 async I/O

PostgreSQL 18 is the first release with an asynchronous I/O subsystem. Five
settings arrived with it and one existing default changed, which is worth
knowing before tuning anything.

| Setting | Default | Context | What it does |
|---|---|---|---|
| `io_method` | `worker` | postmaster | `worker` (a pool of I/O processes), `io_uring` (Linux kernel interface), `sync` (pre-18 behaviour) |
| `io_workers` | `3` | sighup | Number of I/O worker processes when `io_method = worker` |
| `io_combine_limit` | 128kB | user | Largest combined read |
| `io_max_combine_limit` | 128kB | postmaster | Server-wide clamp on the above |
| `io_max_concurrency` | `-1` | postmaster | `-1` computes a value from buffer pool and process counts, capped at 64 |
| `effective_io_concurrency` | `16` (was 1) | user | Raised in 18; 16 matches modern storage |
| `maintenance_io_concurrency` | `16` (was 10) | user | Same, for maintenance work |

We pin `io_method=worker` and change nothing else. `worker` is the default, and
pinning it is a deliberate act: the setting is start-only, so pinning it means
a later release cannot change how this cluster issues I/O without us noticing.
`io_workers=3` is fine for the two-CPU container the compose stack runs.

On a larger host, `io_workers` is the setting to raise — roughly a quarter of
the core count is the published advice, and the stock 3 is a Raspberry Pi
default. `io_uring` is not worth trying inside a container: several container
runtimes disable it for security reasons.

To see what a server actually resolved:

```sql
SELECT name, setting, context FROM pg_settings WHERE name LIKE 'io\_%' ORDER BY name;
SELECT * FROM pg_aios;   -- in-flight asynchronous I/O, PG18 only
```

`io_max_concurrency` is worth reading even though nobody sets it. It starts at
`-1` and is computed at boot from the buffer pool and process counts, so two
instances with identical configuration files can run different ceilings. It is
the first number to compare when two machines on the same storage behave
differently.

## Turning it off

Remove the `-c` arguments from the server and the statistics stop being
collected; the views stay and report the truth about it. Drop the extension in
the usual way if you want the objects gone:

```sql
SELECT ops.db_signals;          -- what was active
DROP VIEW IF EXISTS ops.query_tail;
DROP EXTENSION IF EXISTS pg_stat_statements;   -- statistics only
DROP EXTENSION IF EXISTS hypopg;
DROP EXTENSION IF EXISTS pg_cron;              -- unschedules its jobs with it
```

`ops.db_signals` and `ops.explain_plan` keep working: they need no extension.
