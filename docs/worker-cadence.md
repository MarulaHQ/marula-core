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

Two things hold for every loop. It paces on the host clock, and the decisions
themselves read `now()` from Postgres, so a longer interval changes the latency
of detection and never correctness. And a failed sweep backs off a fixed 2s
before returning to its cadence: that 2s is error recovery, not a poll, and
raising it would only slow recovery.

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

## Leave these two alone

- The outbox relay (250ms) and the Redis stream consumers (2s blocking read) are
  the real-time path. The relay feeds the WebSocket hub, so a raised value shows
  up as delayed browser updates. They are deliberately not in the profile.
- The 2s error backoff inside each sweep, for the reason above.
