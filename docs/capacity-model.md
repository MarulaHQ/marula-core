# Capacity model

How many agents one instance serves, and how many instances a target user
count wants. The numbers come from measurements, the arithmetic is small, and
the knobs the plan touches are the deployment parameters from the descriptor.

## The anchor

`perf-harness journey` runs a realistic agent session over the ticket
workspace and reports what one instance sustains. The recorded anchor is the
knee of the 50k sweep in `docs/perf.md` §7, run against a release core-api on a
scratch Postgres:

| users | requests/s | p95 |
|---|---|---|
| 256 | 485 | 48ms |
| 384 | 695 | 93ms |
| 512 | 748 | 257ms (over budget) |

The knee is 384 concurrent sessions at 695 requests/second and 93ms p95, so the
anchor is **695 requests/second, 25.9ms p50, 500ms think-time**. The source run
is `design/qa-p49-perf-journey/journey-50k-u384.json`.

## The arithmetic

Two lines. One instance holds

    users_per_instance = throughput_rps × (think_ms + p50_ms) / 1000

which is Little's law: a user occupies the box for its think-time plus the time
its request takes. On the anchor that is `695 × (500 + 25.9) / 1000`, about
**366 per instance**. A target user count then wants

    instances = ceil(target_users / users_per_instance)

## Sizing table

From the anchor, at the same 500ms think-time:

| target users | instances | Postgres connections |
|---|---|---|
| 250 | 1 | 71 |
| 500 | 2 | 142 |
| 1000 | 3 | 213 |
| 2000 | 6 | 426 |

Regenerate it, or size another point, with the calculator:

```sh
perf-harness capacity \
  --report design/qa-p49-perf-journey/journey-50k-u384.json \
  --target-users 1000
```

## Connections

Each instance opens about 71 Postgres connections at the default pool ceilings:
core-api app 20 and reporting 25, worker 20 and delivery 5, and one LISTEN
connection. `docs/perf.md` §5 has the full table. Postgres defaults to 100, so
the second instance needs either a raised `max_connections` or a session pooler
in front of the pools. The pool ceilings are deployment parameters. The
descriptor names them.

## Where the knobs come from

The plan reads deployment parameters, not invented ones. `marula deploy
describe --json` (`docs/deploy-descriptor.md`) lists them with kinds and
defaults: the pool ceilings that set connections, the worker cadence that sets
idle load, and the `all-in-one` toggle that decides how many processes share an
instance. A consumer does not guess these. It reads them.

## The two measurements

- **Per-user request rate: measured.** The session holds 1.8 to 1.9
  requests/second per user while the box has headroom, falling toward 1.5 as it
  saturates. This was the first open assumption from the R1 capacity note.
- **Dataset penalty: measured.** On a quiet box, doubling the dataset to 100k
  leaves throughput and the median untouched and raises the session p95 by 1.13
  to 1.30 times (an exponent of about 0.2 to 0.4), concentrated in the board
  list. Since users-per-instance reads the median, it is unchanged between 50k
  and 100k. `docs/perf.md` §7 has the run and the raw reports are in
  `design/qa-p49-100k-penalty/`.

## Limits

- The anchor is one machine under contention (a parallel lane's stack shared the
  box). Absolute throughput is hardware-bound. The shape is what travels.
- The model scales linearly with instances, which holds until Postgres saturates
  or the pooler becomes the bottleneck. The connection table is the first limit
  to hit, not the last.
- The think-time is a model knob, not a measurement. Every quoted
  users-per-instance figure states it, because halving it halves the user count.
