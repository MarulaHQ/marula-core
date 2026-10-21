# Service level objectives

Six objectives cover the surfaces the metric contract describes. Each one names
the series that measures it, the window it is measured over, and the alert that
spends it. The recording rules are in
`prometheus/rules/marula-slo.rules.yaml`, and the two files are changed together.

These are the first written targets in the repository. Two of them reuse numbers
the repo already enforces and four are set here, which is why every row cites
where its number comes from. An objective with no citation is a wish.

| Id | Surface | Objective | Window | Measured by |
|---|---|---|---|---|
| `api-availability` | HTTP API | 99.9% of requests are not 5xx | 30 days | `marula.http.requests` |
| `api-latency` | HTTP API | 95% of requests finish under 150 ms | 30 days | `marula.http.request.duration` |
| `event-freshness` | outbox relay | oldest unpublished row under 5 s, 99% of the time | 30 days | `marula.outbox.lag_ms` |
| `consumer-freshness` | stream consumers | oldest unacked entry under 60 s, 99% of the time, per group | 30 days | `marula.consumer.oldest_pending_age_seconds` |
| `job-latency` | job queue | p95 claim-to-settle under 30 s | 30 days | `marula.queue.claim_to_apply` |
| `backup-rpo` | backup | last successful backup under 26 h | every day | `marula.backup.age_seconds` |

## Where the numbers come from

`api-latency` is the 150 ms product budget from `docs/perf.md` §1. The perf smoke
proves it on a fixed harness at 50k tickets. This objective measures the same
number on the install that is running, so a regression that only appears with real
data shows up here.

`event-freshness` uses `RELAY_POLL_MS` (5000 ms) as its line, from
`docs/worker-cadence.md`. That value is the backstop, so an install that sits at
the line is delivering by polling rather than by notification, which is the alert
`MarulaOutboxLagWarn`.

`consumer-freshness` uses 60 s, the idle threshold after which a restarted worker
takes over another replica's pending entries. An entry held longer than that is
work the system already believes nobody owns.

`job-latency` uses 30 s, which is three capped consumer backoffs. The retry policy
caps a single backoff at 10 s.

`backup-rpo` uses 26 hours: one nightly pass on the default 03:00 schedule plus
two hours of grace for a slow dump and a late stamp.

`api-availability` at 99.9% is a choice made here. Nothing in the repository
states an availability target, and an SLO that waits for one is an SLO nobody
measures. 99.9% is the common floor for a service that a business runs its
tickets on, and 0.1% of a month is 43 minutes of allowance, which is small enough
to be meaningful and large enough to survive a rolling deploy.

## Error budgets

| Id | Budget | Allowance over 30 days |
|---|---|---|
| `api-availability` | 0.1% of requests | 43 minutes of unavailability |
| `api-latency` | 5% of requests | 5 in 100 requests slower than 150 ms |
| `event-freshness` | 1% of time | about 7 hours of lag above 5 s |
| `consumer-freshness` | 1% of time, per group | about 7 hours of held entries |
| `job-latency` | 5% of jobs | 5 in 100 jobs slower than 30 s |
| `backup-rpo` | none | an RPO is a promise, not a ratio |

The HTTP objectives are spend-based: every failing or slow request draws from the
same monthly allowance. The two freshness objectives are time-based, because the
gauges that measure them are sampled once per pass rather than per event.

## Alerts against the budget

`api-availability` uses the multi-window burn rates from the SRE workbook. A page
at 14.4 times the budget rate over an hour spends 2% of the month, and a ticket at
6 times over six hours spends 5%. Both require a short window to agree, so an
incident that recovers stops paging without anyone muting it.

`api-latency` is enforced directly by `MarulaApiLatencyBudgetBreach` at the 150 ms
line and `MarulaApiLatencyHeadroomLow` at two thirds of it, which is the point
`docs/perf.md` F4 uses for "act before this is a regression". Burn-rate
multipliers do not translate usefully to a 95% objective: 14.4 times a 5% budget
is 72% of requests slow, and a page at that level arrives after the answer is
obvious.

The two freshness objectives are enforced by the pipeline alerts
(`MarulaOutboxLagHigh`, `MarulaConsumerPendingAgeHigh`), and their recording rules
exist to make the budget visible on a dashboard rather than to page.

## Reading the freshness numbers honestly

`marula.outbox.lag_ms` samples the age of the worst unpublished row once per relay
pass. So `event-freshness` measures the share of time the pipeline was behind,
not the share of events that were late. A single slow event in an otherwise quiet
hour moves the first number and barely moves the second.

The recording rule says which one it computes:
`marula:sli_outbox_fresh:ratio_rate1h` is `1` minus the average of the samples
that were over the line. Treat it as a time ratio.

## Changing an objective

Change the number in the rule file and in this table, in one commit. The burn-rate
thresholds are written as `multiplier * budget`, so a new objective moves the
threshold with it and leaves nothing to remember.

An objective that is met comfortably for a quarter is a candidate for tightening.
An objective that is missed every month and has no owner is a candidate for
deletion, because a target nobody acts on teaches people to ignore the board.
