# Mail provider down

**Reached from:** `MarulaJobRetryRatioHigh`

## What the alert means

The alert that gets you here is the queue-wide retry signal, so this page starts
with a confession: outbound mail has no metric of its own. The worker's mail loop
writes its state to the `email_outbound` table and nowhere else, so a provider
outage shows up as retries across the queue, as a user reporting that a reply
never arrived, or not at all.

The alert pack records that gap rather than hiding it. Closing it needs one
counter over send outcomes, and until it exists this runbook is reached by
inference.

## First three checks

1. Read the outbound ledger. `queued` with a `next_attempt_at` in the past and a
   growing `attempts` count is a provider that is refusing:

   ```sh
   docker compose exec postgres psql -U marula -d marula -c \
     "SELECT status, count(*), max(attempts) AS max_attempts
        FROM email_outbound GROUP BY status ORDER BY 2 DESC;"

   docker compose exec postgres psql -U marula -d marula -c \
     "SELECT id, attempts, max_attempts, left(last_error, 160) AS error, next_attempt_at
        FROM email_outbound WHERE status IN ('queued','dead')
        ORDER BY updated_at DESC LIMIT 10;"
   ```

   `max_attempts` defaults to 8. A row that reached it is in `dead` and needs the
   dead-letter runbook, not this one.

2. Confirm which transport is selected, and that its settings are the ones you
   expect. A typo in the transport name does not fall back to another provider:

   ```sh
   docker compose exec worker env | grep -E '^(EMAIL_TRANSPORT|SMTP_HOST|SMTP_PORT)='
   ```

   The org setting of the same name wins over the environment for that org's
   mail, so check the setting too if one org is affected and others are not.

3. Test the provider from the container that has to reach it, not from your
   laptop. A working laptop and a blocked container egress look identical from
   the ledger:

   ```sh
   docker compose exec worker sh -c 'nc -zv "$SMTP_HOST" "${SMTP_PORT:-25}"'
   ```

## Escape hatch

Mail is the one delivery path with a safe "off" switch: an install with no
transport configured records the message and stops trying, and the ledger keeps
the row for a later redrive. If the provider is down for an extended period, that
is better than a retry storm that buries the queue signal other alerts depend on.

When the provider is back, redrive the rows that gave up through the dead-work
API with source `email`, then watch the `queued` count drain. Do not bulk-redrive
while the ledger still shows transport errors: each attempt is charged to the
same budget that just failed.

A provider that throttles rather than refuses looks like a lower failure rate
spread over hours. `last_error` usually says so, and the fix is a lower send rate,
not a longer retry window.

## Afterwards

Record the provider's response and the window it was down. If this repeats,
the missing send-outcome counter is the thing to build, and this runbook's first
paragraph is the argument for it.
