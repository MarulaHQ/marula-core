# Worker cadence profile

The worker hosts a set of loops. Some exist to react fast, such as the outbox
relay and the Redis stream consumers. Others are housekeeping sweeps that only
need to run every few minutes or hours. This page is the cadence map and the
profile a small install can adopt.

## Cadence map

| Loop | Kind | Default | Knob |
|---|---|---|---|
| outbox relay | poll | 250ms idle | fixed |
| Redis stream consumers (webhooks, automations, notifier, search, workflow) | blocking read | 2s block | fixed |
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

## The event stream is bounded, and that is not a loop

`marula:events` keeps a fixed number of entries, set by
`MARULA_EVENT_STREAM_MAXLEN` (default `50000`). The bound is applied on every
`XADD`, so nothing schedules it and no cadence setting makes it longer or
shorter. The reason it is a count rather than an age: the shipped Valkey runs
`--maxmemory 192mb --maxmemory-policy noeviction`, so an unbounded stream does
not get slow, it reaches the ceiling and then refuses every write — the relay
stops publishing and every consumer stops at once. Consumers resume at the tail
and reconcile a gap from a REST snapshot, so nothing reads the trimmed history.

One thing does happen at startup: the worker trims any backlog an earlier
unbounded build left in the stream, in chunks of
`STREAM_TRIM_CHUNK`/`STREAM_TRIM_PASSES` (`crates/worker/src/lib.rs`), before
the relay starts writing. Without that, the first `XADD` would carry the whole
trim and hold Valkey's single-threaded loop for its length. If the log says the
backlog was not fully trimmed, the remaining passes continue on later publishes.

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

## Leave these two alone

- The outbox relay (250ms) and the Redis stream consumers (2s blocking read) are
  the real-time path. The relay feeds the live SSE stream, so a raised value shows
  up as delayed browser updates. They are deliberately not in the profile.
- The 2s error backoff inside each sweep, for the reason above.
