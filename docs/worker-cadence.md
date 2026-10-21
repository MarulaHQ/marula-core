# Worker cadence profile

The worker hosts a set of loops. Some exist to react fast, such as the outbox
relay and the Redis stream consumers. Others are housekeeping sweeps that only
need to run every few minutes or hours. This page is the cadence map and the
profile a small install can adopt.

## Cadence map

| Loop | Kind | Default | Knob |
|---|---|---|---|
| outbox relay | notify | 5s safety poll | `RELAY_POLL_MS`, `MARULA_RELAY_WAKEUP` |
| Redis stream consumers (`crates/worker/src/consumer.rs`) | blocking read | 2s block | fixed |
| general job queue | poll | 500ms idle | `QUEUE_IDLE_MS` |
| outbound email send | poll | 2s idle | `EMAIL_POLL_SECS` |
| SLA sweep | sweep | 60s | `EVAL_INTERVAL_SECS` |
| pm sweep | sweep | 60s | `EVAL_INTERVAL_SECS` |
| rotation sweep | sweep | 60s | `EVAL_INTERVAL_SECS` |
| dedupe sweep | sweep | 60s | `EVAL_INTERVAL_SECS` |
| missing-time nudge | sweep | 60s | `TIME_AUDIT_POLL_SECS` |
| scheduled report delivery | sweep | 60s | `DELIVERY_POLL_SECS` |
| org usage refresh | sweep | 300s | `USAGE_REFRESH_SECS` |
| daily digest | sweep | 900s | `DIGEST_POLL_SECS` |
| run retention | sweep | 6h | `RETENTION_SWEEP_SECS` |
| event-outbox retention | sweep | 6h | `OUTBOX_SWEEP_SECS` |

Two things hold for every loop. It paces on the host clock, and the decisions
themselves read `now()` from Postgres, so a longer interval changes the latency
of detection and never correctness. And a failed sweep backs off a fixed 2s
before returning to its cadence: that 2s is error recovery, not a poll, and
raising it would only slow recovery.

## The relay is woken, not polled

`0068_outbox_notify.sql` fires `pg_notify` on the outbox insert, inside the
committing transaction, so the notification exists if and only if the row does.
The worker holds one `PgListener` on `marula_events` and the relay runs a pass
when one arrives. Delivery latency is one round trip instead of up to a quarter
of a second, and an idle worker issues twelve outbox queries a minute instead of
two hundred and forty.

`RELAY_POLL_MS` (default `5000`) is the backstop, not the path: it bounds how
long a missed notification can delay delivery during a listener reconnect.
`MARULA_RELAY_WAKEUP=off` restores pure polling at `RELAY_POLL_MS`, which is the
escape hatch if the listener cannot hold a connection. Setting it to `off` with
`RELAY_POLL_MS=250` reproduces the pre-wakeup worker exactly; that is the
baseline the idle-query measurement compares against.

The notification carries no work of its own. The outbox row is the queue, the
relay reads it in its own transaction, and a notification lost during a
reconnect costs one safety poll. That is why the listener needs no cursor.

## Scaling: two roles

The loops fall into two groups that scale differently.

| `WORKER_ROLES` | Runs |
|---|---|
| `realtime` | the outbox relay and the stream/queue consumers: webhooks, automations, notifier, search, workflow, CSAT, project billing, decision triage, the job queue, and outbound email |
| `housekeeping` | the sweeps: SLA, PM schedules, rotation, dedupe, missing-time nudges, scheduled reports, usage refresh, digest, the workflow-run retention pass, the event-outbox retention pass |
| `all` (default) | both groups in one process |

Every replica can run `realtime`: the relay claims outbox rows inside one
transaction with `FOR UPDATE SKIP LOCKED`, and the consumers read through Redis
consumer groups. The sweeps claim due rows by hand and are not safe to
duplicate, which is why they are the group to run on exactly one process. Before
this switch, scaling the worker horizontally duplicated every loop, sweeps
included.

`/healthz` reports the loops a process does not own as `disabled` and leaves
them out of its health decision, so a realtime-only replica is healthy without
its eight sweeps.

A one-box install that wants the sweeps off the worker entirely can run
`WORKER_ROLES=realtime` and drive the cadence from outside the process. The
database-side half of that is already scheduled: where `pg_cron` is installed,
migration 0145 runs `VACUUM (ANALYZE)` on the hot tables overnight (see
[`docs/site/reference/db-observability.md`](site/reference/db-observability.md)).
The sweeps themselves are Rust and have no SQL entry point, so `pg_cron` cannot
run them — moving one out of the worker means giving it a callable function
first. Nothing in the sweeps needs to live in the worker process; it hosts them
because something has to.

## The event stream is bounded, and that is not a loop

`marula:events` keeps a fixed number of entries, set by
`MARULA_EVENT_STREAM_MAXLEN` (default `50000`). The bound is applied on every
`XADD`, so nothing schedules it and no cadence setting makes it longer or
shorter. The reason it is a count rather than an age: the shipped Valkey runs
`--maxmemory 192mb --maxmemory-policy noeviction`, so an unbounded stream does
not get slow, it reaches the ceiling and then refuses every write — the relay
stops publishing and every consumer stops at once. A consumer that comes back
continues from its own pending list and the live tail, never from the start of
the stream, so nothing reads the trimmed history; a gap is reconciled from a
REST snapshot.

One thing does happen at startup: the worker trims any backlog an earlier
unbounded build left in the stream, in chunks of
`STREAM_TRIM_CHUNK`/`STREAM_TRIM_PASSES` (`crates/worker/src/lib.rs`), before
the relay starts writing. Without that, the first `XADD` would carry the whole
trim and hold Valkey's single-threaded loop for its length. If the log says the
backlog was not fully trimmed, the remaining passes continue on later publishes.

## When a stream consumer fails on an entry

Every stream consumer runs the same harness, and the harness treats one entry
as the unit of work. A handler that fails leaves its entry in the group's
pending list and the process retries it after a delay from the retry policy
(`Subsystem::Consumer`: 250 ms doubling to a 10 s cap). The attempt count, the
last error and the next attempt time live in `consumer_deliveries`, one row per
group and entry. When the entry runs out of attempts — the queue's budget, so a
unit of work gets the same number of tries whether it is a job or a stream entry
— it is written to `dead_events` with its event identity, the consumer that gave
up, the last error and the raw payload, and it is acknowledged so it stops
holding back the entries behind it.

Two tables to watch, and both are empty on a healthy install because only
failed entries are written to them:

```sql
SELECT consumer_group, count(*) FILTER (WHERE dead_at IS NULL) AS retrying,
       count(*) FILTER (WHERE dead_at IS NOT NULL) AS dead
  FROM consumer_deliveries GROUP BY consumer_group;

SELECT consumer_group, entry_id, event_type, attempts, last_error, dead_at
  FROM dead_events ORDER BY dead_at DESC LIMIT 20;
```

A restarted process takes over the entries a dead replica left, once they have
been idle for a minute, so a crashed worker does not strand work under its old
consumer name.

## Small-install profile

A one-box install with one or two orgs has little work for the sweeps and few
jobs for the queue, so the fast idles mostly wake up to find nothing. This set
cuts that idle churn to a minute or more per loop:

```sh
EMAIL_POLL_SECS=60
QUEUE_IDLE_MS=60000
EVAL_INTERVAL_SECS=300
TIME_AUDIT_POLL_SECS=300
DELIVERY_POLL_SECS=300
USAGE_REFRESH_SECS=900
DIGEST_POLL_SECS=3600
RETENTION_SWEEP_SECS=86400
OUTBOX_SWEEP_SECS=86400
```

What each one trades:

- `EMAIL_POLL_SECS=60`: outbound mail can wait up to a minute after enqueue.
- `QUEUE_IDLE_MS=60000`: a webhook or effect job can wait up to a minute.
- `EVAL_INTERVAL_SECS=300`: SLA evaluation, project billing, secret rotation and
  dedupe detection run every five minutes instead of every minute, so SLA breach
  detection trails by up to five minutes.
- `TIME_AUDIT_POLL_SECS` and `DELIVERY_POLL_SECS` at 300: missing-time nudges and
  scheduled reports sweep every five minutes.
- `USAGE_REFRESH_SECS=900` and `DIGEST_POLL_SECS=3600`: monitoring numbers and the
  daily digest update less often. Neither is user-facing latency.
- `RETENTION_SWEEP_SECS=86400` and `OUTBOX_SWEEP_SECS=86400`: the two retention
  windows are enforced once a day instead of every six hours. A published outbox
  row is already relayed, so neither one is a delivery path. The only cost is
  that a day's worth of expired rows lingers until the next pass. The WINDOWS
  themselves (`OUTBOX_RETENTION_DAYS`, `workflow.run_retention_days`) decide how
  much is kept. These two decide how promptly.

## Leave the stream consumers alone

- The Redis stream consumers' 2s blocking read is the real-time path: each read
  is one connection wait, not a query per tick, and it is what turns a published
  event into a webhook, an automation, a notification or a workflow step. The
  relay that feeds them is notification-driven now, so a raised block only adds
  latency to work that already arrived.
- The 2s error backoff inside each sweep, for the reason above.
