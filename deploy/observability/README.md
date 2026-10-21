# Observability profile

What ships here: the OTel collector that receives telemetry from `core-api` and
the worker, Tempo for traces, Loki for logs, Grafana with provisioned
datasources and one starter dashboard, and Prometheus as the rule evaluator for
the alert pack.

```
core-api ─┐                          ┌─→ Tempo   traces
worker  ──┼─→ otel-collector ────────┼─→ Loki    logs
          │   (OTLP 4317/4318)       └─→ :18899  metrics (Prometheus exposition)
          │                                  │
          └──────────────────────────────────┴─→ prometheus ─→ rules, SLOs, alerts
                                                     │
                                                     └─→ Grafana (MarulaMetrics)
```

Start it with the observability profile and point both processes at the
collector:

```sh
OTEL_EXPORTER_OTLP_ENDPOINT=http://otel-collector:4318 \
  docker compose -f deploy/docker-compose.yml --profile observability up
```

Grafana is at `:3001`, Prometheus at `:9090`, Tempo at `:3200` and Loki at
`:3100`.

## The alert pack

`prometheus/rules/` is loaded by the shipped Prometheus. `prometheus/rules-optin/`
is for rules that need a signal the operator has to wire first, and each file
there says which. `prometheus/tests/` holds the unit tests.

The pack is written to be portable. An install that runs its own Prometheus,
Mimir, Thanos or a hosted vendor keeps the same rule files and points them at its
own scrape target. Nothing in `rules/` refers to the shipped scrape job except
`MarulaCollectorDown`, which reads Prometheus's own `up` series.

Every rule names the contract metric it reads in a `metric` annotation, and every
alert points at a runbook. `scripts/check-alert-rules.mjs` enforces both
directions, which means a metric rename fails the build instead of orphaning a
rule.

### Metric names

The collector translates OTLP names to Prometheus ones: dots become underscores,
a unit word is appended unless the name already carries it, and a monotonic sum
gets `_total`. The table below is the whole contract and what each series is used
for. The derivation is implemented in `scripts/check-alert-rules.mjs`, so the
left column is checked against `crates/core-otel/contract/metrics.v1.json` on
every run.

| Contract metric | Exported series | Used by |
|---|---|---|
| `marula.http.requests` | `marula_http_requests_total` | availability SLO, burn-rate alerts |
| `marula.http.request.duration` | `marula_http_request_duration_milliseconds_bucket` | latency SLO, latency alerts |
| `marula.org.requests` | `marula_org_requests_total` | diagnostic only, see below |
| `marula.live.sessions` | `marula_live_sessions` | diagnostic only, see below |
| `marula.outbox.lag_ms` | `marula_outbox_lag_ms_milliseconds` | outbox lag alerts, event freshness SLO |
| `marula.relay.batch_size` | `marula_relay_batch_size_bucket` | diagnostic only, see below |
| `marula.queue.depth` | `marula_queue_depth` | worker heartbeat, dead depth, backlog |
| `marula.queue.dead_age_seconds` | `marula_queue_dead_age_seconds` | dead age alert |
| `marula.queue.jobs` | `marula_queue_jobs_total` | webhook dead, retry ratio |
| `marula.queue.claim_to_apply` | `marula_queue_claim_to_apply_milliseconds_bucket` | job latency alert |
| `marula.consumer.pel_depth` | `marula_consumer_pel_depth` | pending list alert |
| `marula.consumer.lag` | `marula_consumer_lag` | consumer lag alert |
| `marula.consumer.oldest_pending_age_seconds` | `marula_consumer_oldest_pending_age_seconds` | pending age alert, consumer freshness SLO |
| `marula.workflow.parked_runs` | `marula_workflow_parked_runs` | diagnostic only, see below |
| `marula.workflow.parked_run_age_seconds` | `marula_workflow_parked_run_age_seconds` | parked run alert |
| `marula.backup.age_seconds` | `marula_backup_age_seconds` | backup age alert, RPO objective |
| `marula.sla.sweep.duration` | `marula_sla_sweep_duration_milliseconds_bucket` | SLA sweep alert |
| `marula.sla.verdicts` | `marula_sla_verdicts_total` | diagnostic only, see below |
| `marula.db.page1_p95` | `marula_db_page1_p95_milliseconds` | page-1 latency trigger (two thirds of the 150 ms budget) |
| `marula.db.page1_sort_index_scan` | `marula_db_page1_sort_index_scan_ratio` | plan-shape trigger, opt-in: `0` is the shape F4 accepted |

| `marula.org.ingest_requests` | `marula_org_ingest_requests_total` | diagnostic only, see below |
| `marula.reconcile.search_index_backlog` | `marula_reconcile_search_index_backlog` | diagnostic only, see below |
| `marula.reconcile.dead_events_replayable` | `marula_reconcile_dead_events_replayable` | diagnostic only, see below |
| `marula.reconcile.dead_events_unrepairable` | `marula_reconcile_dead_events_unrepairable` | diagnostic only, see below |
| `marula.reconcile.workflow_wake_lost` | `marula_reconcile_workflow_wake_lost` | diagnostic only, see below |
| `marula.reconcile.report_retry_pending` | `marula_reconcile_report_retry_pending` | diagnostic only, see below |
| `marula.reconcile.report_retry_exhausted` | `marula_reconcile_report_retry_exhausted` | diagnostic only, see below |
| `marula.consumer.gap_entries` | `marula_consumer_gap_entries` | diagnostic only, see below |
| `marula.consumer.gap_lost` | `marula_consumer_gap_lost` | diagnostic only, see below |
| `marula.embedding.reindex.remaining` | `marula_embedding_reindex_remaining` | diagnostic only, see below |
| `marula.embedding.reindex.runs_pending` | `marula_embedding_reindex_runs_pending` | diagnostic only, see below |
| `marula.embedding.reindex.embedded` | `marula_embedding_reindex_embedded_total` | diagnostic only, see below |

The reconciliation pass (`R4`), the machine-ingest counter (`R12`) and the
embedding re-index (`R14`) add twelve series to the contract. None carries a
rule yet, and none is an oversight: the thresholds they would fire on are the
first thing a real deployment measures, and a rule invented from a laptop is a
rule that pages for the wrong reason. They are in the contract and in this table
so the gap is visible rather than silent.

Seventeen metrics carry no alert and that is deliberate. `marula.org.requests` is the
per-org counter a premium pack bills or quotas from, and an alert on it would page
someone about a customer's traffic volume. `marula.org.ingest_requests` is the same
counter for machine traffic, and it carries the same objection — a connector's
volume is its owner's business. The eight reconciliation gauges are backlog and
deficit counts: a backlog that is being drained is the system working, and the
number that matters is its direction over hours, which no honest rule can read
from one scrape. `marula.consumer.gap_lost` is the one of the nine that will earn
a rule first: a lost gap means events between the last processed id and the stream
bounds are gone, and a sustained non-zero is data loss rather than backlog. `marula.live.sessions` is a live
connection count, and a low one means an idle evening. `marula.relay.batch_size`
is the distribution an operator reads while diagnosing a backlog it already has a
rule for. `marula.sla.verdicts` counts SLA ladder verdicts, which are the product
working as designed, and the breach rate is a business metric rather than an
availability one. `marula.workflow.parked_runs` is the count behind the
parked-run age alert: the age carries the threshold, the count says how many
runs are behind it, and neither is charted because the age already pages. The
three embedding re-index series are a progress report: `remaining` and
`runs_pending` fall while a switch drains, and an alert on a value that is
supposed to fall would page every operator who changes the model. A stalled job
is recognisable from the same series sitting flat for hours, which is a
dashboard question, not a page.

### Coverage gaps

Three signals the task list for this pack asks for are not in the contract, so no
rule can read them. They are recorded here rather than approximated with a rule
over something else, because a rule that fires for the wrong reason is worse than
a known blind spot.

| Missing signal | What it would take | Why it matters |
|---|---|---|
| Email send outcomes | a counter `marula.email.send` with attributes `outcome` and `transport` | dead and retrying mail is invisible today. The dead-letter alert sees queue jobs, and mail is not a queue job |
| Provider error rate | a counter or histogram `marula.provider.requests` with `provider`, `operation` and `outcome` | the pack infers provider health from queue retries, which hides the provider name and misses providers used outside the queue |
| Dependency reachability | gauges `marula.postgres.up` and `marula.valkey.up`, written on the worker's own cadence | a database or Valkey outage reaches the pack only as the worker heartbeat going quiet, so the page says "worker" and the diagnosis is a runbook branch |

Until those exist, `mail-provider-down.md`, `database-down.md` and
`valkey-down.md` are reached by inference. Each one says so at the top.

### Severity and routing

Rules carry `severity: page` or `severity: ticket` and a `surface` label
(`telemetry`, `worker`, `pipeline`, `queue`, `workflow`, `api`, `backup`).
Delivery is not in this repository. A route table that matches the labels:

```yaml
route:
  receiver: ticket-queue
  group_by: [severity, surface]
  routes:
    - matchers: [severity = "page"]
      receiver: pager
      repeat_interval: 30m
    - matchers: [severity = "ticket"]
      receiver: ticket-queue
      repeat_interval: 24h
```

## SLOs

`SLOS.md` carries the objective table and the error-budget arithmetic.
`prometheus/rules/marula-slo.rules.yaml` implements the recording rules those
objectives need. The two are changed together.

## Validation

Two layers, and both run in the group gate. The structural checks need Node and
nothing else. The syntax and firing checks need docker and the pinned Prometheus
image:

```sh
# structure of the pack, against the metric contract
node scripts/check-alert-rules.mjs

# structural checks only, on a machine without docker
node scripts/check-alert-rules.mjs --no-promtool
```

That command runs two container steps of its own when docker is present, and
skips them with a note when it is not:

```sh
# config syntax, plus every rule file the globs resolve to. The pack is mounted
# where the container expects it, so the rule_files globs are checked as shipped.
docker run --rm --entrypoint promtool \
  -v "$PWD/deploy/observability/prometheus:/etc/prometheus:ro" \
  prom/prometheus@sha256:6559acbd5d770b15bb3c954629ce190ac3cbbdb2b7f1c30f0385c4e05104e218 \
  check config /etc/prometheus/prometheus.yml

# the five rules that must fire, and the five cases that must stay quiet
docker run --rm --entrypoint promtool \
  -v "$PWD/deploy/observability/prometheus:/p:ro" \
  prom/prometheus@sha256:6559acbd5d770b15bb3c954629ce190ac3cbbdb2b7f1c30f0385c4e05104e218 \
  test rules /p/tests/marula-core.test.yaml
```

The image digest is the same one the compose profile runs, so the gate and the
stack agree on the parser.

## Non-purpose

No sampling or redaction policy: that happens at the source in `crates/core-otel`.
No retention or long-term storage: the profile keeps a local TSDB and a local
Loki, and an install that needs months keeps them elsewhere. No TLS, because this
is the development profile. No Alertmanager container: the rules evaluate, and
delivery is the operator's.
