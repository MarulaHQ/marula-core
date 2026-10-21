# Capacity trigger — page 1 is close to its budget, or its plan changed

Two alerts share this runbook. Both answer the same question: is the ticket-list
index that `docs/perf.md` F4 measured worth shipping now, and has anything moved
the decision out from under us.

| Alert | What it says |
|---|---|
| `MarulaPage1P95PastTrigger` | The measured page-1 SQL p95 reached two thirds of the 150 ms budget |
| `MarulaPage1SortPlanLostItsIndex` | The page-1 ORDER BY is no longer served by an ordered index |

Neither alert is a page. A page is the 150 ms budget itself, and the nightly perf
gate owns it.

## What the numbers are

`worker capacity_watch::sample` times the same page-1 statement the endpoint runs
(`core_db::tickets::list_tickets` with no filters, fifty rows) and reads that
statement's plan through `ops.explain_plan`. It samples the install's largest org
by ticket count — the worst page 1, which is where the decision pressure comes
from.

The sampler runs on a schedule, not on traffic. If the series are absent, the
sampler is not wired on this install, and neither alert can fire: check the
worker log for `page-1 capacity sample` before assuming page 1 is healthy.

## `MarulaPage1P95PastTrigger`

1. Read the sample line: `p95_ms`, the org, the row count, and the plan chain.
2. Compare against the budget with the harness, not with the alert: run the perf
   harness at this install's ticket count and read `page1_default` p95. The
   alert's number comes from one org on a production pool, the harness measures a
   seeded fixture with the same shape, and F4's 31 ms came from the harness.
3. If the harness agrees the headroom is gone, the decision F4 deferred has
   arrived: ship the candidate index. Its definition, measured effect (0.66 ms
   against 31 ms) and the reasoning that kept it off are in F4, and the recipe can
   be tried without building it. `hypopg` plus the `CREATE INDEX` statement is in
   `docs/site/reference/db-observability.md`, under "Trying a candidate index
   without building it".
4. If the harness disagrees, the sample is reading a specific org's data or a
   specific cold plan. Record the org and the numbers in the ticket; do not
   silence the alert.

## `MarulaPage1SortPlanLostItsIndex`

This alert only makes sense once F4's index has shipped or the plan has been
re-baselined; that is why the rule is opt-in. Before reading it as a regression,
confirm the index exists:

```sql
SELECT indexname FROM pg_indexes
 WHERE tablename = 'tickets' AND indexdef LIKE '%due_at%';
```

- **No such index.** The rule was loaded early. The install is on the baseline F4
  accepted (a top-N heapsort over the org's rows); stop reading, and turn the rule
  off until the index ships.
- **Index present.** Something took the ordered path away. Read the plan:

```sql
SELECT jsonb_pretty(ops.explain_plan($$
  SELECT id FROM tickets
   WHERE org_id = '<the org from the sample>'::uuid
   ORDER BY (CASE priority WHEN 'p1' THEN 0 WHEN 'p2' THEN 1 WHEN 'p3' THEN 2 ELSE 3 END),
            due_at NULLS LAST, created_at, id
   LIMIT 50
$$));
```

  The usual causes, in the order worth checking: the sort tuple in the query no
  longer matches the index (a column order or `NULLS` clause changed), `ANALYZE`
  has not run since a bulk load so the planner's row estimate is wrong, or the
  sort needs more `work_mem` than the session has and the planner chose a
  different path. `ops.db_signals` and `ops.query_tail` (same page) answer the
  middle one: `pg_stat_statements` shows the statement's cost moving.
- A `Sort Method` of `external merge Disk:` in the plan is a different problem
  from a missing index — it means the sort outgrew `work_mem`. Raising it is a
  configuration decision, and the disk sort is the reason the sample reports the
  method at all.

## Re-baselining

Both alerts exist to reopen a written decision, so closing them means writing the
new one down: update F4 in `docs/perf.md` with the measured number and the plan
shape, and adjust `PAGE1_TRIGGER_MS` in `core-db/src/capacity.rs` (the crate test
ties it to the budget) or turn the opt-in rule off, as the case requires. An
alert resolved by a rule edit alone will come back with nobody any the wiser.
