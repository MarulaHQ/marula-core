# Turn on travel time and mileage reconciliation

> **Audience:** admin/dispatcher — provider config is a settings-owner call;
> the daily start points and reconciliation are dispatcher work.

This guide configures a routing provider, sets a technician's day start, reads
the travel warnings the dispatcher sees, and reconciles logged mileage into
reimbursable expenses. Nothing here is required for scheduling to work: with
no provider configured, Marula estimates travel from the per-mile table and
labels every number as an estimate.

## Choose a provider

Set `ROUTING_PROVIDER` in the API's environment and restart it. The API logs
the routing posture at boot, and `GET /api/dispatch/routing` reports it.

| Value | What it needs | What you get |
|---|---|---|
| `none` (default) | nothing | Per-mile estimates, no route suggestions |
| `osrm` | `ROUTING_OSRM_URL` (self-hosted OSRM) | Measured durations, route suggestions |
| `ors` | `ROUTING_ORS_URL`, optional `ROUTING_ORS_API_KEY` | Measured durations, route suggestions |
| `mapbox` | `MAPBOX_ACCESS_TOKEN`, optional `MAPBOX_PROFILE` | Measured durations, geocoding, isochrones, suggestions |
| `contract` | `ROUTING_CONTRACT_URL`, optional `ROUTING_CONTRACT_TOKEN`, `ROUTING_CONTRACT_ID` | Whatever the pack declares |

Commercial packs from the relay marketplace arrive as `contract` providers.
Their manifest declares capabilities and modes, plus the time control it
honors. You supply the URL and the token. The wire contract they implement is in
[Routing providers](../../routing-providers.md).

Tokens belong in the environment file your deployment already uses for SMTP
and Postmark credentials. Do not put them in org settings.

## Tune the estimate table (no provider)

With `none`, Marula estimates each leg as straight-line distance × the road
factor, at the per-mile pace, plus a fixed overhead. The defaults are 2 minutes
per mile, 5 minutes overhead, and a 125% road factor. Change them in
**Settings → General**:

- *Travel estimate: minutes per mile*
- *Travel estimate: fixed overhead*
- *Travel estimate: road factor %*

Estimates carry `quality: "heuristic"`. The board and calendar say "estimated"
where it matters.

## Set a technician's day start

Each day's travel runs from a start point:

1. Open the dispatch board for the day.
2. Call `PUT /api/dispatch/journey-start` with the technician, the day, and the
   kind: `home`, `office`, `site`, or `address`. A site start needs `site_id`;
   the other kinds accept `address` or explicit coordinates.
3. Read it back with `GET /api/dispatch/journey-start`. The response shows the
   resolved point and whether it came from your override or the auto-suggestion.

With no override, the day starts where yesterday's last booked site was. A day
with no override and no history has no start point, and the routing view says
so per technician instead of guessing.

Booking an on-site appointment then computes travel into it and stores the
minutes on the appointment. Manual minutes win: pass `travel_minutes` greater
than zero when creating or patching a booking and nothing recomputes over it.

## Read warnings and suggestions

`GET /api/dispatch/routing?date=YYYY-MM-DD` returns, per technician, the
ordered stops, each leg's minutes and distance, and typed warnings:

| Warning | Meaning |
|---|---|
| `missing_start_point` | No start point resolved for that day |
| `missing_coordinates` | The stop's site has no coordinates |
| `zero_transfer` | Bookings touch, but the route needs time between them |
| `insufficient_transfer` | The gap is shorter than the required travel |
| `provider_error` | The provider failed. The reason label rides along |

Route suggestions (`swap_adjacent`, `reassign`) appear only when the active
provider reports measured quality. They are proposals — apply one by moving the
block on the board. With an estimate-grade provider the field comes back
`available: false` and no suggestion content renders.

## Reconcile travel costs

Technicians log actual travel as drafts on **Dispatch → Travel & mileage**, or
from an approval row's *log travel* link, which prefills the ticket.

1. Add a draft: ticket, time window, distance, unit, vehicle, billable flag.
2. Submit it. Submission turns it into a travel time entry and the draft
   becomes immutable.
3. Press *Expense* on a submitted draft. Marula prices the distance at the
   org rate and books a ticket expense for the reimbursement, once per draft.

The reconciliation card above the table totals the last 14 days per technician:
trips, distance, time, booked expenses, and the reimbursement owed. Rates live
in **Settings → General**:

- *Mileage reimbursement rate (minor units)* — 67 means 0.67 per unit.
- *Mileage rate unit* — `mi` or `km`. Logged distances convert for the rollup.
- *Reimburse technician mileage* — turn off to keep mileage informational.

Amounts appear in the org's base currency. Expenses booked while a foreign
currency rate is locked keep their own currency and are excluded from the
base-currency total.
