# Dead letter: work was abandoned

**Pages:** `MarulaQueueDeadDetected`, `MarulaQueueDeadAgeHigh`, `MarulaWebhookDeliveryDead`, `MarulaConsumerPendingAgeHigh`, `MarulaConsumerPelDepthHigh`

## What the alert means

A unit of work spent its retry budget and was written off. Four ledgers can do
that, and the alert names which family is involved through its `surface` label
and its metric: `marula.queue.depth{state="dead"}` and
`marula.queue.jobs{outcome="dead"}` are the job queue,
`marula.consumer.pel_depth` and `marula.consumer.oldest_pending_age_seconds` are
the stream consumers.

Nothing is lost yet. A dead row keeps its payload, so this is a decision that has
been deferred to you rather than data that has disappeared.

## First three checks

1. List what is dead, newest first. `ops.dead_work` unions all four ledgers and
   carries the source token the redrive API wants:

   ```sh
   docker compose exec postgres psql -U marula -d marula -c \
     "SELECT source, id, subject, attempts, left(last_error, 120) AS error, dead_since
        FROM ops.dead_work ORDER BY dead_since DESC LIMIT 20;"
   ```

2. Read the reason on the row that matters. For a webhook, the attempt ledger
   carries the response code of every try. A `4xx` is the receiver rejecting the
   payload, a `5xx` or a null status is the receiver being down:

   ```sh
   docker compose exec postgres psql -U marula -d marula -c \
     "SELECT attempt, response_status, last_error, created_at
        FROM webhook_deliveries WHERE webhook_id = '<id>' ORDER BY attempt;"
   ```

3. Check whether the cause is still there before redriving. A redrive into the
   same broken receiver spends the retry budget again and lands back in this
   alert. For mail, `email_outbound.last_error` holds the transport's words.

## Escape hatch

Decide per row, and only after check 3 says the cause is gone. Both verbs are
`Admin:Edit` and audited:

```sh
# requeue one row, due now
curl -X POST "$API/api/v1/admin/dead-work/webhook/<id>/redrive" -H "Authorization: Bearer $TOKEN"

# write it off instead, so it stops being counted and stops being retried
curl -X POST "$API/api/v1/admin/dead-work/webhook/<id>/cancel" -H "Authorization: Bearer $TOKEN"
```

`source` is one of `job`, `email`, `webhook` or `event`. A redriven webhook keeps
its dedupe key, so the receiver sees a duplicate rather than a second delivery of
the same event. A second redrive of the same row is a no-op rather than a second
requeue.

For a destination that keeps failing, the pause is automatic and the verb that
undoes it is `POST /api/v1/admin/dead-work/destinations/<id>/resume`. Fix the
receiver first, then resume, then redrive the rows.

For a stream consumer entry (`MarulaConsumerPendingAgeHigh`), redrive belongs to
the reconciliation work, not to this API: the row in `dead_events` carries the
raw payload and the group name, and the consumer resumes from the live tail.

One reading worth knowing before you count rows: an email cancelled by a user is
stored with status `dead`, because the email ledger has no separate terminal
state for a cancel. The dead set therefore includes rows nobody needs to act on.
`ops.dead_work` marks cancelled rows through `cancelled_at`, and the job queue
keeps its own count honest.

## Afterwards

Say in the incident note which rows were redriven and which were written off.
A dead row that is cancelled without a reason is a decision the next reader
cannot audit.
