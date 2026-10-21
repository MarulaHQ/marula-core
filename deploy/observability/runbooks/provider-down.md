# A delivery provider is failing

**Reached from:** `MarulaJobRetryRatioHigh`, `MarulaJobLatencyHigh`

## What the alert means

Jobs are settling as retries instead of done, or one job kind has become slow,
and the cause is outside this product. The queue measures the effect of a failing
dependency without being told which dependency it is, so the first task is
naming it. Webhook receivers, mail providers and upstream HTTP APIs all produce
this shape.

`MarulaJobRetryRatioHigh` fires at a quarter of settlements retrying for fifteen
minutes. `MarulaJobLatencyHigh` fires when a job kind's p95 from claim to settle
passes thirty seconds, which is three capped retry backoffs.

## First three checks

1. Find the job kinds carrying the retries. A single kind dominating is one
   provider. Every kind retrying is this install's own egress or the database:

   ```promql
   sum by (kind) (rate(marula_queue_jobs_total{outcome="retried"}[10m]))
   ```

   The same question in SQL, which also shows the error text:

   ```sh
   docker compose exec postgres psql -U marula -d marula -c \
     "SELECT kind, count(*), max(attempts) AS tries, left(max(last_error), 140) AS error
        FROM jobs WHERE status = 'pending' AND attempts > 0
        GROUP BY kind ORDER BY 2 DESC;"
   ```

2. For `webhook.deliver`, the attempt ledger separates a receiver that rejects
   the payload from one that is not there:

   ```sh
   docker compose exec postgres psql -U marula -d marula -c \
     "SELECT response_status, count(*) FROM webhook_deliveries
        WHERE created_at > now() - interval '1 hour'
        GROUP BY response_status ORDER BY 2 DESC;"
   ```

   A cluster of `4xx` is a receiver refusing this install's signature or payload,
   which is a configuration problem on their side and does not clear by retrying.
   A cluster of `5xx` or of nulls is the receiver being down, which does.

3. For mail, the answer is in `email_outbound.last_error` and the transport
   settings. Follow `mail-provider-down.md` for that branch.

## Escape hatch

Whole-provider outages are survived, not fixed, from this side. The retry budget
is finite and it is the thing absorbing the failure, so the useful moves are to
stop creating work and to keep the budget from being spent on a dependency that
is known to be down.

Disable the webhook subscription that is failing rather than deleting it. A
disabled subscription makes the delivery job settle as done, which is what stops
the retry storm, and re-enabling it later resumes delivery without losing the
secret. A destination that repeated failures paused can be resumed with
`POST /api/v1/admin/dead-work/destinations/<id>/resume` once the receiver is
fixed.

If every kind is retrying, the dependency is likely this install's own egress:
DNS, the proxy, or a firewall change. `docker compose exec worker nc -zv` against
one provider host answers that in one command.

## Afterwards

Name the provider and the failure mode in the incident note. A receiver that
refuses `4xx` will do it again on the next payload shape, and the note is what
stops the next responder from starting at the beginning.
