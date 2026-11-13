# Consumer gap lost: the stream trimmed entries a group never read

**Pages:** `MarulaConsumerGapLost`

## What the alert means

One consumer group is reading from a position older than the stream's first
entry. `marula:events` keeps a fixed number of entries
(`MARULA_EVENT_STREAM_MAXLEN`, default 50,000) and trims on every `XADD`, so
everything between that group's last-delivered id and the stream floor has been
dropped. This is not backlog: a backlog is entries that still exist and will
arrive. These entries do not exist anywhere in Valkey any more.

The gap is reported by the reconciliation sweep, which samples every group's
position against the stream bounds every 15 minutes
(`MARULA_RECONCILE_SECS`). The alert needs two passes to agree, so a group
recreated during a trim does not page.

## First three checks

1. Name the group and measure the window. The difference between the two ids is
   how many entries are gone:

   ```sh
   docker compose exec redis valkey-cli XINFO STREAM marula:events | head -20
   docker compose exec redis valkey-cli XINFO GROUPS marula:events
   ```

   Compare the stream's `first-entry` id with that group's `last-delivered-id`.
   Both are `<unix-ms>-<seq>`, so the millisecond difference is also how long the
   group has been behind.

2. Find out whether the events survived in Postgres. The outbox keeps the payload
   of every event the product emitted, and it is the reason a trimmed stream is
   recoverable at all:

   ```sh
   docker compose exec postgres psql -U marula -d marula -c \
     "SELECT event_type, count(*), min(occurred_at), max(occurred_at)
        FROM outbox_events
       WHERE occurred_at > now() - interval '1 day'
       GROUP BY event_type ORDER BY 2 DESC;"
   ```

   Rows older than the outbox retention window are gone from here too. If the gap
   is older than that window, the outbox cannot help and the third check is the
   last one that can.

3. Read the dead-entry ledger for the same window. An entry that exhausted its
   retry budget was copied to `dead_events` with its raw payload before it was
   acknowledged, so a consumer that was failing rather than absent leaves a
   recoverable record:

   ```sh
   docker compose exec postgres psql -U marula -d marula -c \
     "SELECT consumer, count(*), min(created_at), max(created_at)
        FROM dead_events GROUP BY consumer ORDER BY 2 DESC;"
   ```

## Escape hatch

Decide in this order, and write down which one you took.

1. **Recover from the outbox.** Redrive the affected window through the
   dead-work API with `source` `event`, which re-publishes into the stream and
   keeps the dedupe key:

   ```sh
   curl -X POST "$API/api/v1/admin/dead-work/event/<id>/redrive" -H "Authorization: Bearer $TOKEN"
   ```

2. **Accept the loss and move the group to the live tail.** Once the window is
   either recovered or written off, the group must stop reading from a position
   that no longer exists. `$` sets it to the newest entry, so the group resumes
   on live traffic and the alert resolves:

   ```sh
   docker compose exec redis valkey-cli XGROUP SETID marula:events <group> $
   ```

   Do this only after check 1 named the group. Setting the wrong group's id
   forwards a live consumer past entries it has not read, which turns one gap
   into two.

3. **If this is the second loss in a week, the stream bound is the problem.** The
   bound is a count because the shipped Valkey runs `noeviction` at 192 MB, so
   raising `MARULA_EVENT_STREAM_MAXLEN` trades this gap for refused writes. The
   durable answer is a second `WORKER_ROLES=realtime` replica so the group keeps
   up, not a longer stream. That arithmetic is in `docs/worker-cadence.md`.

## Afterwards

Record the group, the size of the window in entries, and whether the outbox
covered it. A group that loses a window twice is a consumer that is not keeping
up with its own arrival rate, which the lag and PEL alerts describe before the
loss does.
