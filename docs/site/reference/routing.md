# Routing and travel reference

System facts about the routing provider seam, its endpoints, its settings, and
the reconciliation surface. For the vendor-facing wire contract see
[Routing providers](../../routing-providers.md).

## Provider selection

| Environment variable | Values | Default |
|---|---|---|
| `ROUTING_PROVIDER` | `none`, `osrm`, `ors`, `mapbox`, `contract` | `none` |
| `ROUTING_OSRM_URL` | OSRM base URL | — |
| `ROUTING_ORS_URL` | OpenRouteService base URL | — |
| `ROUTING_ORS_API_KEY` | ORS key (hosted instances) | unset |
| `MAPBOX_ACCESS_TOKEN` | Mapbox token | unset |
| `MAPBOX_PROFILE` | `driving`, `driving-traffic`, `walking`, `cycling` | `driving` |
| `ROUTING_CONTRACT_ID` | Pack id (adapter registers `contract:<id>`) | `pack` |
| `ROUTING_CONTRACT_URL` | Contract base URL | — |
| `ROUTING_CONTRACT_TOKEN` | Bearer token for the pack | unset |

A named provider with a missing required value falls back to NONE mode and logs
a warning. The active posture is visible at `GET /api/dispatch/routing` under
`status`: `provider_id`, `provider_label`, `quality` (`heuristic` or
`measured`), `capabilities`, `modes`, `suggestions_available`, and a typed
`reason` when degraded (`no_provider_configured`, `provider_not_registered`).

## Org settings

| Key | Type | Default | Meaning |
|---|---|---|---|
| `routing.heuristic_minutes_per_mile` | integer 1–60 | 2 | Estimate pace in NONE mode |
| `routing.heuristic_overhead_minutes` | integer 0–120 | 5 | Fixed minutes per estimated leg |
| `routing.heuristic_road_factor_percent` | integer 100–300 | 125 | Straight-line → road distance |
| `travel.mileage_rate_minor` | integer 0–100000 | 67 | Reimbursement per unit, minor units |
| `travel.mileage_rate_unit` | `mi` \| `km` | `mi` | Unit the rate is quoted per |
| `travel.reimburse_techs` | boolean | true | Include mileage in technician reimbursement |

## Appointment travel fields

`travel_minutes` is travel time immediately before `starts_at`. It occupies
dispatch capacity through `occupied_minutes`, and it never shrinks the booked
time itself. Provenance rides alongside:

| Field | Meaning |
|---|---|
| `travel_from` | Resolved start-point snapshot (`kind`, `label`, `site_id`, coordinates, `source`) |
| `travel_distance_meters` | Provider distance for the leg |
| `travel_provider` | `none`, `osrm`, `mapbox`, or `contract:<id>` |
| `travel_quality` | `heuristic` or `measured` |

Manual minutes carry no provider fields. On a move, only provider-stamped
estimates recompute.

## Endpoints

| Route | Purpose |
|---|---|
| `GET /api/dispatch/routing?date=` | Day travel plans, legs, typed warnings, suggestions |
| `GET /api/dispatch/journey-start?user_id=&date=` | Resolved start point plus any explicit override |
| `PUT /api/dispatch/journey-start` | Set the technician-day start point (instant envelope) |
| `GET /api/dispatch/reconciliation?from=&to=&user_id=` | Mileage/time/expense rollups (max 366 days) |
| `POST /api/dispatch/travel-drafts/{id}/expense` | Book a draft's mileage as a ticket expense |

Warning kinds: `missing_start_point`, `missing_coordinates`, `zero_transfer`,
`insufficient_transfer`, `provider_error`. Suggestion kinds: `swap_adjacent`,
`reassign`. Suggestions require a measured provider. In NONE mode
`available: false` with a typed reason.

## Reconciliation semantics

- Rollups read travel drafts in the window, not time entries. A submitted draft
  has both.
- Distance converts to the rate unit before pricing (1 mi = 1.609344 km).
- `reimbursement_minor` is the technician total at the org rate, zero when
  `travel.reimburse_techs` is off.
- `expense_minor` and `billable_minor` sum linked expenses in the org base
  currency. Expenses in other currencies count as `expensed_count` only.
- One expense per draft. The endpoint is idempotent through the standard
  envelope. A second call without a key returns a conflict.
