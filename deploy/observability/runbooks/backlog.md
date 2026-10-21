# Backlog: events or jobs are not draining

**Pages:** `MarulaOutboxLagHigh`, `MarulaOutboxLagWarn`, `MarulaConsumerLagHigh`, `MarulaQueueBacklog`, `MarulaWorkflowParkedRunStale`, `MarulaSlaSweepSlow`

## What the alert means

One stage of the worker has more work than it can settle. The three stages fail
differently and need different answers, so the first job is to find out which one
the alert is talking about. `surface: pipeline` on the alert means the event path
(outbox, then stream, then consumers). `surface: queue` means the job queue.
`surface: workflow` means a sweep that has fallen behind its own cadence.

## First three checks

1. Find the stage that is behind. Run these three and read the one that is
   growing:

   ```sh
   docker compose exec postgres psql -U marula -d marula -c \
     "SELECT count(*) AS unpublished, max(now() - occurred_at) AS oldest
        FROM outbox_events WHERE published_at IS NULL;"

   docker compose exec redis valkey-cli XINFO GROUPS marula:events

   docker compose exec postgres psql -U marula -d marula -c \
     "SELECT status, count(*) FROM jobs GROUP BY status ORDER BY 2 DESC;"
   ```

   `unpublished` growing means the relay is failing to publish. `lag` growing in
   `XINFO GROUPS` means a consumer is failing to keep up. `pending` growing in
   the third query means claims are not happening.

2. Ask the worker what it thinks it owns:

   ```sh
   docker compose exec worker wget -qO- http://localhost:8080/healthz
   ```

   Every loop appears as `alive`, `down` or `disabled`. A `disabled` loop belongs
   to a different `WORKER_ROLES` group and is not a fault on this replica. A
   `down` loop is the one that stopped and its name is the stage.

3. Decide whether the bottleneck is Postgres or Valkey, because the fix differs:

   ```sh
   docker compose exec postgres psql -U marula -d marula -c \
     "SELECT state, count(*) FROM pg_stat_activity GROUP BY state;"

   docker compose exec redis valkey-cli INFO memory | grep -E 'used_memory_human|maxmemory_human'
   ```

   A pile of `active` sessions means the database is the limit. Valkey memory at
   the 192 MB `maxmemory` ceiling is the other limit and it does not degrade: it
   refuses writes.

## Escape hatch

For the event path, the relay's notification listener is the fast path and
`RELAY_POLL_MS` is the backstop. If the listener cannot hold its connection,
turn the backstop into the path:

```sh
MARULA_RELAY_WAKEUP=off RELAY_POLL_MS=250 docker compose up -d worker
```

That is the pre-wakeup behaviour, documented in `docs/worker-cadence.md`, and it
delivers at 250 ms. Add a second replica running `WORKER_ROLES=realtime` when the
backlog is volume rather than breakage. The relay claims outbox rows with
`FOR UPDATE SKIP LOCKED` and the consumers use Valkey groups, so extra realtime
replicas are safe.

For a job backlog, adding pollers will not help if each job is slow. Find the
kind that dominates and stop its source instead:

```sh
docker compose exec postgres psql -U marula -d marula -c \
  "SELECT kind, count(*) FROM jobs WHERE status = 'pending' GROUP BY kind ORDER BY 2 DESC;"
```

For a sweep that is always behind (SLA, retention), move it out of the worker.
`WORKER_ROLES=realtime` plus `pg_cron` running the same passes is the documented
small-install profile in `docs/worker-cadence.md`.

Do not raise `MARULA_EVENT_STREAM_MAXLEN` to make a backlog fit. The shipped
Valkey runs with a 192 MB ceiling and `noeviction`, so a longer stream refuses
`XADD` and takes the relay down together with every consumer.

## Afterwards

Record which stage was behind and what the queue depth was at the peak in the
incident note. If the same stage backs up twice in a week, the threshold is not
the problem and the throughput is.
