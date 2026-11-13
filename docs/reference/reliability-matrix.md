# Reliability matrix

> **Audience:** whoever operates an instance, and whoever changes a subsystem and
> needs to know what the change is allowed to cost. One row per subsystem, and
> each row states behaviour at this commit rather than intent.

Seven module comments each described their own pieces of this: the consumer
harness, the webhook sink, the outbox relay, the retention pass, the pooler, the
backup drill, the alert pack. Read separately they answer a question nobody asks.
Read together they answer the one an operator asks at 03:00, which is what this
instance promises, what it does not, and which page says so when the promise
breaks. That is the table.

Every claim cites a file and a line. Where a longer argument already exists, the
row states the guarantee and links out instead of paraphrasing it. Retention
lives in [data lifecycle](data-lifecycle.md), recovery in [RPO and RTO](rpo-rto.md),
the pooler in [connection pooling](connection-pooling.md).

## The matrix

| Subsystem | Guarantees | Does not guarantee | Failure mode | Where the operator sees it |
|---|---|---|---|---|
| **Event stream and consumers**<br>`crates/worker/src/consumer.rs` | Each process reads `marula:events` under its own consumer name, `{group}-{host}-{pid}`, so no two replicas share a pending list (`consumer.rs:8-11`). On restart it drains its own pending entries, then claims entries idle past 60 s from a peer that went away (`:12-16`, `:64`). A failed entry is retried in-process on the consumer delays (`:17-19`), and is written to `dead_events` and acked once the queue's attempt budget is spent (`:20-23`, `:81-83`) | Order across a takeover. A claimed entry is handled after entries the live tail already passed. A handler slower than 60 s can have its entry claimed by a sibling, which is why handlers enqueue with `queue::enqueue_deduped` (`:61-63`). Events leave the stream by length rather than by age, because `MARULA_EVENT_STREAM_MAXLEN` trims the tail (`crates/core-events/src/lib.rs:81`) | A handler that acks without doing its work loses the event with no trace. Two processes sharing one consumer name let a sibling drain an entry mid-handling (`:8-11`) | `marula.consumer.oldest_pending_age_seconds` and per-group lag (`crates/worker/src/stream_health.rs:105-149`), the alerts `MarulaConsumerPendingAgeHigh` and `MarulaConsumerLagHigh`, and the [backlog](../../deploy/observability/runbooks/backlog.md) and [dead-letter](../../deploy/observability/runbooks/dead-letter.md) runbooks |
| **Outbox relay**<br>`crates/core-events/src/lib.rs` | Every committed outbox row reaches the stream and is marked published inside the transaction that read it (`lib.rs:254-281`). A committed insert wakes the relay through `NOTIFY` on `marula_events` instead of waiting out the poll (`:24-30`). Lag is measured before the publish loop, from the oldest unpublished row (`:265-272`) | Exactly once. Publish happens before the transaction commits, so a crash in that window republishes the batch. Consumers dedupe and the relay does not | A publisher error rolls the transaction back and the rows stay unpublished, so lag climbs. A stopped relay stalls live push, webhook delivery and search indexing together, because all three read one stream | `marula.outbox.lag_ms` against the `event-freshness` objective (`deploy/observability/SLOS.md`), the alerts `MarulaOutboxLagWarn` and `MarulaOutboxLagHigh`, and the [backlog](../../deploy/observability/runbooks/backlog.md) runbook |
| **Webhook delivery**<br>`crates/worker/src/webhook_out.rs` | One signed POST per subscription and event, `t=<ts>,v1=<hex>` over the body, with every attempt recorded (`webhook_out.rs:14-17`). The job insert is dedupe-keyed on that pair, so an at-least-once read cannot become a second POST (`:11-13`). A destination that keeps failing is paused at the webhook policy's budget and stops being matched for new events (`:24-27`) | That the receiver acted once. A transport error after the receiver took a 2xx leaves the job retryable and the receiver sees the event again. Delivery order. A paused subscription still runs jobs that were already queued (`:28-33`) | An unset `MARULA_SECRET_KEY` dead-letters every delivery with a loud reason, by design (`:19-22`). Past the attempt budget the job is dead: visible, redrivable, and no longer retried | `webhook_deliveries` and `ops.dead_work`, the alert `MarulaWebhookDeliveryDead`, and the [dead-letter](../../deploy/observability/runbooks/dead-letter.md) runbook |
| **Job queue**<br>`crates/core-db/src/queue.rs` | Claims are single-statement CTEs under `FOR UPDATE SKIP LOCKED`, so two workers never hold one job, and `running` rows left by a crashed claimant are re-armed in the same pass (`queue.rs:1-10`). Ordering and backoff use Postgres `now()`, never a host clock (`:1-10`). Attempts are counted per claim against `max_attempts` (`:23-27`), and the queue dead-letters rather than retrying forever (`:3`) | That a job settles inside its lease. Work that outlives the re-arm window can be claimed twice, which is why long work parks instead of holding a claim. Idempotence of a handler's side effects | A handler that panics after its side effect leaves the job retryable, so the effect must be idempotent or deduped on a key | `ops.dead_work` and `marula.queue.claim_to_apply`, the alerts `MarulaJobLatencyHigh`, `MarulaQueueDeadDetected`, `MarulaQueueDeadAgeHigh`, and the [dead-letter](../../deploy/observability/runbooks/dead-letter.md) and [provider-down](../../deploy/observability/runbooks/provider-down.md) runbooks |
| **Sweeps and retention**<br>`crates/worker/src/data_retention_sweep.rs` | A window per table per profile, with a floor under every one, and a row still owed work is never deleted ([data lifecycle](data-lifecycle.md) §1). The pass reports saturation per table and logs only tables that deleted something (`data_retention_sweep.rs:110-119`). Partition upkeep runs in the same pass, and its failure does not stop the deletes (`:105-109`) | Day precision on the partitioned table. `auth_attempt_log` is dropped by month, so with a 30-day window the oldest row is between 30 and 61 days old ([data lifecycle](data-lifecycle.md) §1). An archive: a row past its window is deleted, not moved | A saturated table pauses 30 s and continues, so the backlog drains slowly rather than at once. With the housekeeping role absent, nothing is deleted at all and the tables grow quietly | `/healthz` `retention_matrix` and the per-table sweep log line, with the [retention and partitions](../../deploy/observability/runbooks/retention-and-partitions.md) runbook |
| **Partitions**<br>`crates/core-db` (`partitions::maintain`) | Monthly partitions for `auth_attempt_log` are created ahead and dropped once the last day in one is past the window, which turns expiry into metadata work instead of a delete (`crates/core-db/migrations/0146_data_lifecycle--breaking.sql`) | That a failed maintenance pass is harmless later. Inserts keep landing in the `DEFAULT` partition, so the product works while `DEFAULT` grows, and it has to be drained before it can be dropped | Silent in the product and visible in one place: `partition maintenance failed; batched retention continues` in the worker log (`data_retention_sweep.rs:105-109`). `DEFAULT` accumulates until someone reads that line | The worker log line above, `/healthz` `retention_matrix`, and the [retention and partitions](../../deploy/observability/runbooks/retention-and-partitions.md) runbook |
| **Transaction pooler**<br>`deploy/pooler/pgbouncer.ini` | Server connections are bounded at 15 per database and user (`pgbouncer.ini:60`) and 40 in total (`:63`) however many application instances are added, and the reset query runs after every transaction, so session state cannot cross tenants (`:53`, [connection pooling](connection-pooling.md)) | Report concurrency: a long report holds a client connection for the length of the query. Migrations and `pg_dump`, which dial Postgres directly. Session state of any kind, so `LISTEN` must use `MARULA_LISTEN_DATABASE_URL` | Two silent ones. A named prepared statement outliving its transaction gives `prepared statement "P_0" does not exist`, which is why the cache ships off (`:54`). A `LISTEN` attached to a pooled connection receives nothing and reports no error | `SHOW POOLS` and `SHOW STATS` on the admin console, the pooler log, `MarulaApiLatencyBudgetBreach` when the queue behind it grows, and the [pooler degraded](../../deploy/observability/runbooks/pooler-degraded.md) runbook |
| **Backups and the drill**<br>`deploy/backup/` | A nightly `pg_dump -Fc` plus a restic snapshot in one pass, WAL-G when a bucket is configured, and a stamp written only after a pass in which no layer failed ([RPO and RTO](rpo-rto.md) §2). A drill every seven days by default restores the newest artifact into a scratch database and records the outcome and wall-clock duration in `ops.backup_drill_runs` ([RPO and RTO](rpo-rto.md) §3) | An RTO from the planning table: two of its three terms are assumptions until a drill lands on your hardware (§3). WAL replay to a recovery target, which needs a second instance (§6). That an erasure reaches archives already written ([data lifecycle](data-lifecycle.md) §4) | A failed layer leaves the stamp stale, and the age gauge climbs. A failed drill does not block the stamp, on purpose, so one signal cannot explain away the other (§3) | `marula.backup.age_seconds` and `ops.backup_drill_runs`, the alerts `MarulaBackupStale` and `MarulaBackupUnobserved`, and the [restore from backup](../../deploy/observability/runbooks/restore-from-backup.md) and [backup drill failed](../../deploy/observability/runbooks/backup-drill-failed.md) runbooks |
| **Alerting**<br>`deploy/observability/` | 23 alerts and two opt-in rules. Each one names the contract metric it reads, a severity of `page` or `ticket`, a `for`, and a runbook that names the alert back (`scripts/check-alert-rules.mjs`) | Coverage. Postgres, Valkey and the mail provider have no reachability series, so their failures arrive as branch runbooks behind somebody else's alert (`deploy/observability/runbooks/README.md:52-59`). A series nothing exports cannot page | A renamed metric leaves a rule reading a series nobody emits, which the check catches before it ships rather than after. A collector that stops being scraped reads as silence, which is what `MarulaCollectorDown` exists for | Alertmanager routes on severity, and the table in `deploy/observability/runbooks/README.md` maps every alert to its runbook |
| **Capacity trigger**<br>`crates/core-db/src/capacity.rs` | A scheduled sampler times the page-1 statement against the largest org by ticket count and reads its plan (`crates/worker/src/capacity_watch.rs:33-58`). The trigger sits at two thirds of the budget: 100 ms against a 150 ms budget (`capacity.rs:55`, `:65`). Firing reopens a written decision rather than asking for a repair | That page 1 is healthy when the series are absent. With the sampler off neither alert can fire, so no data looks like no problem ([capacity trigger](../../deploy/observability/runbooks/capacity-trigger.md)) | Silence. Both alerts are tickets, not pages, because the budget itself belongs to the nightly perf gate | The worker log line `page-1 capacity sample`, the alert `MarulaPage1P95PastTrigger`, the opt-in plan-shape rule, and the [capacity trigger](../../deploy/observability/runbooks/capacity-trigger.md) runbook |

## What the table leaves out

Four things belong to the matrix's subject and are not rows, because each is a
decision rather than a mechanism:

- **Object store integrity.** `MARULA_OBJECT_GC` ships off, and turning it on is a
  separate decision with its own safety properties ([data lifecycle](data-lifecycle.md) §3).
- **Tenant erasure.** It deletes rows, object keys, the search index and RAG vectors by
  cascade, and it does not touch the event stream or archives ([data lifecycle](data-lifecycle.md) §4).
- **The replication story.** There isn't one in this repository. Everything above
  describes a single Postgres, and a second one is a deployment shape, not a code path.
- **Restore of a `durable` install.** The drill proves the artifact is retrievable. WAL
  replay is an incident-level exercise with a second instance ([RPO and RTO](rpo-rto.md) §6).

## Where these rows are enforced

The table is checked, not trusted. Each row is held in place by something that fails
when the row stops being true:

| Row | What keeps it honest |
|---|---|
| Consumers, webhook sink | The harness tests in `crates/worker/src/consumer.rs:1158-1396` and the delivery tests in `crates/worker/src/webhook_out.rs:620-781` |
| Outbox relay | `crates/core-events` contract tests, and the `event-freshness` objective that spends a budget when lag grows |
| Job queue | `crates/worker/tests/webhook_restart_replay.rs` and `crates/worker/tests/signed_delivery_e2e.rs` |
| Sweeps, retention, partitions | `crates/worker/src/data_retention_sweep.rs` and the matrix in `crates/core-db/src/retention.rs` |
| Pooler | The descriptor test that pins the statement-cache default against the shipped pooler (`crates/marula-core-cli/src/deploy.rs:2078`), and the RLS probe in [connection pooling](connection-pooling.md) |
| Backups, drill | `deploy/backup/drill.sh` and the rows it writes to `ops.backup_drill_runs` |
| Alerting | `scripts/check-alert-rules.mjs`, which fails when a rule's metric leaves the contract or its runbook stops naming it |
| Capacity trigger | The crate test tying `PAGE1_TRIGGER_MS` to `PAGE1_BUDGET_MS` (`crates/core-db/src/capacity.rs:471`) |
| Retry budgets behind every row | `scripts/check-reliability.mjs`, which fails when a retry path stops using the one policy |

## Keeping the numbers in one place

Every budget in this page is a policy decision that lives in
`crates/core-domain/src/retry.rs`, and [the levers page](reliability-levers.md)
projects it into a table so an operator can read the resolved values without reading
Rust. Nothing here restates a number that a generated page already carries, and
`scripts/generate-reliability-levers.mjs --check` fails when the projection and the
policy disagree.
