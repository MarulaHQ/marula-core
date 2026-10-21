# Retention backlog, or partition maintenance failing

**Reached from:** `MarulaWorkerNotReporting`

## What this means

The retention pass does two jobs and reports them separately, which is the whole
difficulty of this page. A partition-maintenance failure is logged and the pass carries
on with the batched deletes (`crates/worker/src/data_retention_sweep.rs:105-109`), and a
table that hits its batch ceiling marks the pass saturated (`:110-111`). So the product
is healthy while two different things go wrong underneath it.

That split is deliberate. A failed maintenance pass still leaves `INSERT`s landing in the
`DEFAULT` partition, so nothing breaks at the point of use. The cost arrives later:
`DEFAULT` grows, and every row in it has to move before that partition can be dropped.

The windows, the floors and the reasons one table is partitioned while the others are
deleted in batches live in [data lifecycle](../../../docs/reference/data-lifecycle.md).

## First three checks

1. Is the housekeeping role running at all:

   ```sh
   docker compose exec -T core-api wget -qO- http://127.0.0.1:8082/healthz | jq .retention_matrix
   ```

   The matrix reports `partitions` and the batched deletes separately. Nothing deleting,
   with a healthy API, usually means `WORKER_ROLES` is set to `realtime` and the sweeps
   are in the role that is not running.

2. Which table is behind, and by how much:

   ```sh
   docker compose logs --tail=500 worker | grep "retention sweep"
   ```

   Only tables that deleted something log a line, so silence is not evidence of a healthy
   pass: it can be a pass deleting nothing because it is not running. A saturated table
   pauses 30 s instead of an hour, so a backlog does drain, only slowly.

3. Are the partitions where they should be:

   ```sql
   SELECT relname, n_live_tup
     FROM pg_stat_user_tables
    WHERE relname LIKE 'auth_attempt_log%'
    ORDER BY relname;
   ```

   The maintainer keeps three months ahead (`crates/core-db/src/partitions.rs:65`). A
   missing future month means maintenance is not running. A growing
   `auth_attempt_log_default` means rows are arriving that no partition covers, which is
   the same fault seen from the other side.

## Escape hatch

Raise `MARULA_RETENTION_BATCH` above its default of 1000 for one restart and let the
pass drain with bigger statements. It trades lock duration for pass count, so watch the
delete latency while it runs and put it back afterwards.

Do not drop `auth_attempt_log_default` by hand while it holds rows. The maintainer moves
them into a real partition on its next pass and reports the count as `moved_rows`
(`crates/core-db/src/partitions.rs:78-88`). Dropping the partition first destroys the
rows it holds, and the retention window did not ask for that.

## Afterwards

Record the window, the table, the backlog at the time, and whether the fault was the
pass or the partition. A pass that reports `no retention window resolved for this table`
is a configuration fault rather than a load one: the maintainer cannot find a window for
the table, so it skips it.

One follow-up is already written down: `audit_events` is deleted in batches, and three
consecutive passes at the batch ceiling is the documented trigger to revisit partitioning
that table ([data lifecycle](../../../docs/reference/data-lifecycle.md), section 2). If
this is the third, that is the row to file.
