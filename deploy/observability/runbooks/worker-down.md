# The worker is not reporting

**Pages:** `MarulaWorkerNotReporting`
**Reached from:** (that alert)

## What the alert means

`marula.queue.depth` is written on every queue tick, so its absence means the
worker's queue loop is not running. Four minutes of absence on top of the
collector's five-minute series expiry makes roughly nine minutes from the last
tick to the page.

While this lasts, nothing is claimed from the job queue and nothing is published
from the outbox. The work is not lost: the outbox keeps unpublished rows and the
jobs table keeps pending ones. Live updates stop, webhooks stop, and the client
sees a product that has quietly gone read-only.

## First three checks

1. Is the process there at all, and has it been restarting:

   ```sh
   docker compose ps worker
   docker compose logs --tail=80 worker
   ```

   A restart loop is a start-up failure, not a stall, and the log's last lines
   name the missing setting or unreachable dependency.

2. Ask the worker directly. It answers even when it is unhealthy:

   ```sh
   docker compose exec worker wget -qO- http://localhost:8080/healthz
   ```

   The body carries `status`, the `roles` label, and one key per loop whose value
   is `alive`, `down` or `disabled`. For this alert the expected answer is
   `down` on the queue loop, or no answer at all if the process is gone.

3. Which of the two dependencies it cannot reach:

   ```sh
   docker compose exec worker nc -zv postgres 5432
   docker compose exec worker nc -zv redis 6379
   ```

   Postgres unreachable means `database-down.md`. Valkey unreachable means
   `valkey-down.md`. Both reachable with a dead queue loop means look at the
   worker's own configuration next.

## Escape hatch

Restart it once, and read the log of the restart rather than the log of the
previous run:

```sh
docker compose restart worker
```

A restore point exists for the thing that matters. Nothing is published while the
worker is down, so the outbox is a queue that is waiting rather than a stream
that has dropped work. Once the worker is back, the relay publishes the backlog
in batch order and the consumers catch up on their own.

Check `WORKER_ROLES` before restarting anything. A replica started with
`WORKER_ROLES=housekeeping` runs no queue loop and therefore never emits the
series this alert reads. If the install splits roles, this alert has to be
evaluated against the `realtime` replica, and a housekeeping-only process is the
one false positive the rule has.

## Afterwards

Note whether this was a crash, a dependency or a configuration change. A worker
that dies without a crash and without a log line is worth a second look at the
container's OOM state.
