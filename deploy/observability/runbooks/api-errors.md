# API errors or latency over budget

**Pages:** `MarulaApiAvailabilityBurnFast`, `MarulaApiLatencyBudgetBreach`
**Reached from:** `MarulaApiAvailabilityBurnSlow`, `MarulaApiLatencyHeadroomLow`

## What the alert means

One of two objectives is being missed. `api-availability` allows 0.1 percent of
requests to fail with a 5xx over thirty days. `api-latency` asks that 95 percent
of requests finish inside 150 ms, which is the product budget `docs/perf.md` §1
enforces in the perf smoke. A fast burn has fourteen times the budget rate
sustained for an hour behind it, and at that rate the month is gone in two days.

Client errors are not counted. A `4xx` wave is a client problem and does not
belong in this alert.

## First three checks

1. Which route and which status is carrying it. One route burning the budget is a
   defect and a spread across routes is a capacity or dependency problem:

   ```promql
   topk(5, sum by (route) (rate(marula_http_requests_total{status=~"5.."}[10m])))
   ```

2. Read the error itself. The API logs carry `correlation_id`, and Loki has them:

   ```logql
   {service_name="core-api"} |= "ERROR" | json | line_format "{{.correlation_id}} {{.message}}"
   ```

   A burst of the same message is one defect. A spread of different messages is
   a shared dependency, and the next check names it.

3. Is the database the shared dependency:

   ```sh
   docker compose exec postgres psql -U marula -d marula -c \
     "SELECT state, count(*) FROM pg_stat_activity GROUP BY state;"
   docker compose exec postgres psql -U marula -d marula -c \
     "SELECT query, mean_exec_time FROM pg_stat_statements
        ORDER BY mean_exec_time DESC LIMIT 5;"
   ```

   Sessions at the pool ceiling produce `499`-looking client cancellations and
   slow responses rather than 5xx, so latency without errors plus a full pool is
   this case.

## Escape hatch

The one safe move during a burn is to stop making the failure worse. If a deploy
precedes the burn window, roll it back rather than diagnosing on a serving
system. If a single route is both the error source and not on the critical path,
block it at the edge with Caddy and the budget stops being spent.

Do not raise the pool maximum to clear a saturation symptom. Two replicas at a
larger pool reach the same ceiling sooner, and each new connection makes the tail
worse before it makes the head better.

For a latency burn with a healthy database, check the request rate against the
capacity model in `docs/capacity-model.md` before changing anything. A budget
that is being missed at a rate the install has never served is a capacity
decision, not an incident.

## Afterwards

Write down which objective moved and by how much, with the burn rate rather than
the peak. The budget is what the next postmortem will ask about, and the peak
percentage is the number that answers nothing.
