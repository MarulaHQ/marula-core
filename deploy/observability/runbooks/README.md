# Incident runbooks

Purpose: one page per failure the alert pack can page for, written to be read at
the moment the page arrives. Each one names the alerts that lead to it, the first
three checks in order, and the escape hatch that stops the bleeding before the
cause is understood.

**Pages** means the alert routes to this runbook. **Reached from** means the alert
opens a question this runbook answers as one of its branches, usually after the
first check in the runbook it pages.

## Alert to runbook

| Alert | Severity | Signal | Runbook |
|---|---|---|---|
| `MarulaCollectorDown` | page | collector not scraped | [telemetry-blind](telemetry-blind.md) |
| `MarulaWorkerNotReporting` | page | queue depth absent | [worker-down](worker-down.md) |
| `MarulaOutboxLagHigh` | page | outbox lag > 30 s | [backlog](backlog.md) |
| `MarulaOutboxLagWarn` | ticket | outbox lag > 5 s | [backlog](backlog.md) |
| `MarulaConsumerPendingAgeHigh` | page | oldest pending entry > 5 min | [dead-letter](dead-letter.md) |
| `MarulaConsumerPelDepthHigh` | ticket | pending list > 500 | [dead-letter](dead-letter.md) |
| `MarulaConsumerLagHigh` | ticket | lag > 5000 entries | [backlog](backlog.md) |
| `MarulaConsumerGapLost` | page | a group's position is behind the stream floor | [consumer-gap-lost](consumer-gap-lost.md) |
| `MarulaQueueDeadDetected` | ticket | dead depth > 0 | [dead-letter](dead-letter.md) |
| `MarulaQueueDeadAgeHigh` | page | oldest dead job > 24 h | [dead-letter](dead-letter.md) |
| `MarulaWebhookDeliveryDead` | ticket | a delivery gave up | [dead-letter](dead-letter.md) |
| `MarulaQueueBacklog` | ticket | pending depth > 500 | [backlog](backlog.md) |
| `MarulaJobRetryRatioHigh` | ticket | retries > 25% of settles | [provider-down](provider-down.md) |
| `MarulaJobLatencyHigh` | ticket | claim to settle p95 > 30 s | [provider-down](provider-down.md) |
| `MarulaWorkflowParkedRunStale` | ticket | oldest parked run > 7 d | [backlog](backlog.md) |
| `MarulaSlaSweepSlow` | ticket | SLA sweep p95 > 30 s | [backlog](backlog.md) |
| `MarulaApiAvailabilityBurnFast` | page | 14.4x budget over 1 h | [api-errors](api-errors.md) |
| `MarulaApiAvailabilityBurnSlow` | ticket | 6x budget over 6 h | [api-errors](api-errors.md) |
| `MarulaApiLatencyBudgetBreach` | page | p95 > 150 ms | [api-errors](api-errors.md) |
| `MarulaApiLatencyHeadroomLow` | ticket | p95 > 100 ms | [api-errors](api-errors.md) |
| `MarulaBackupStale` | page | backup age > 26 h | [restore-from-backup](restore-from-backup.md) |
| `MarulaBackupUnobserved` | ticket | backup gauge absent (opt-in) | [restore-from-backup](restore-from-backup.md) |
| `MarulaPage1P95PastTrigger` | ticket | page-1 p95 > 100 ms (two thirds of the 150 ms budget) | [capacity-trigger](capacity-trigger.md) |
| `MarulaPage1SortPlanLostItsIndex` | ticket | page-1 sort plan lost its index scan (opt-in) | [capacity-trigger](capacity-trigger.md) |
| `MarulaRetrievalCrossover` | ticket | a tenant holds ≥ 20,000 embeddings | [retrieval-crossover](retrieval-crossover.md) |

`scripts/check-alert-rules.mjs` fails if an alert has no runbook, if a runbook
names an alert that does not exist, or if a row in this table disagrees with the
severity in the rule file.

## Branch runbooks

These have no alert of their own. They are the answer to a question an alert
opens, and the rule pack cannot tell them apart because the metric that would
does not exist yet.

| Runbook | Reached from | Why it has no alert |
|---|---|---|
| [database-down](database-down.md) | `MarulaWorkerNotReporting` | no reachability series for Postgres |
| [valkey-down](valkey-down.md) | `MarulaWorkerNotReporting`, `MarulaConsumerPelDepthHigh` | no reachability series for Valkey |
| [mail-provider-down](mail-provider-down.md) | `MarulaJobRetryRatioHigh` | no send-outcome series for mail |
| [pooler-degraded](pooler-degraded.md) | `MarulaApiLatencyBudgetBreach`, `MarulaWorkerNotReporting` | PgBouncer is not a scrape target, so nothing reads its pools |
| [retention-and-partitions](retention-and-partitions.md) | `MarulaWorkerNotReporting` | the pass reports to its log and `/healthz`, and has no series of its own |
| [backup-drill-failed](backup-drill-failed.md) | `MarulaBackupStale`, `MarulaBackupUnobserved` | no rule reads `ops.backup_drill_runs` |

The missing series are named, with the shape each one would need, in
`deploy/observability/README.md` under coverage gaps.

## Severity

`page` means a person is woken. It is used when events are not being delivered,
the product is down, data has no copy, or the alert pack itself is blind.

`ticket` means next business day. It is used when work is stuck or an objective
is being spent, and nothing is being lost while it waits.

Routing is not in this repository: the rules set `severity`, and the
Alertmanager (or whatever consumes Prometheus alerts) routes on it. The route
table the labels were written for is in `deploy/observability/README.md`.
