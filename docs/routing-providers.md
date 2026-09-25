# Routing providers — the Meridian Routing Contract (revision 1)

Any team can add a distance/duration vendor to Meridian without touching core
code. The provider runs as a service that implements the four endpoints below;
the operator points Meridian at it with environment variables. The core's
contract adapter does the rest. TravelTime, Geoapify, and any other commercial
API fit here as relay connector packs (ADR 0013). Mapbox is the one vendor that
also ships in this repository, as a compile-time provider
(`plugins/connectors/mapbox`) rather than a contract service.

This page is normative for revision 1. `contract_revision` mismatch is a
validation error, so extending the contract means a new revision and additive
core support.

## What the core calls

| Endpoint | Purpose |
|---|---|
| `POST {base_url}/routing/matrix` | Batched N×M durations/distances |
| `POST {base_url}/routing/route` | One route with per-leg totals |
| `POST {base_url}/routing/geocode` | Address → coordinate |
| `POST {base_url}/routing/isochrone` | Reachability contours |

Content type is JSON both ways. Send `contract_revision` fields only in the
manifest. The wire does not carry a revision header in revision 1.

## Auth

The operator configures a bearer token (`ROUTING_CONTRACT_TOKEN`). When set,
the core sends `Authorization: Bearer <token>` on every call. Packs that need a
different scheme terminate TLS and rewrite at their own edge. The core never
signs requests in revision 1.

## Canonical units

- Durations: whole seconds.
- Distances: metres.
- Coordinates: WGS-84 decimal degrees.
- Times: RFC 3339 UTC timestamps.

Values the pack cannot produce are `null` or omitted. Never fabricate a zero
for an unreachable pair: scheduling treats absence as unknown and warns.

## Matrix

Request:

```json
{
  "mode": "driving",
  "depart_at": "2026-09-25T08:30:00Z",
  "arrive_by": null,
  "annotations": { "duration": true, "distance": true },
  "sources": [{ "point": { "latitude": 52.5, "longitude": 13.4 }, "label": "Depot" }],
  "targets": [{ "point": { "latitude": 52.6, "longitude": 13.5 }, "label": "Site" }]
}
```

Response:

```json
{
  "cells": [
    {
      "source_index": 0,
      "target_index": 0,
      "duration_seconds": 900,
      "distance_meters": 12400
    }
  ]
}
```

Rules:

- `sources` and `targets` are lists of place objects. A place carries
  `point` (resolved coordinate), `address`, and `label`. A pack with the
  geocode capability may resolve `address`. A pack without it answers `null`
  cells for unresolved places instead of guessing.
- `depart_at` and `arrive_by` are mutually exclusive. A pack that cannot honor
  the time control it declared answers as if no time was given, and the core
  records the estimate as not time-controlled.
- Batching is expected: one request may carry many sources and targets. The
  contract cap is 2500 cells per request. The core rejects larger requests
  before calling. A pack may impose a lower cap and must fail the request
  rather than silently answering a subset.
- Cell indexes address the request's `sources` and `targets` arrays.

## Route

Request: `mode`, `origin`, `destination`, `waypoints` (ordered, may be empty),
`depart_at`, `arrive_by`. Response:

```json
{
  "legs": [{ "duration_seconds": 900, "distance_meters": 12400 }],
  "total_duration_seconds": 900,
  "total_distance_meters": 12400,
  "geometry": null
}
```

`geometry` stays opaque to the core. Return a polyline or GeoJSON string when
you have one. Scheduling ignores it.

## Geocode

Request: `query`, `near` (optional bias point), `limit`, `country` (optional).
Response:

```json
{
  "hits": [
    {
      "label": "Unter den Linden 1, Berlin",
      "latitude": 52.517,
      "longitude": 13.3889,
      "confidence": null,
      "provider_place_id": "mapbox.abc"
    }
  ]
}
```

Return at most `limit` hits, best first. A pack that cannot geocode does not
declare the capability. The core refuses the call before it leaves the process.

## Isochrone

Request: `center`, `mode`, `contours_minutes` (at most four bands), optional
`depart_at`. Response: `contours` as `{ "minutes": 15, "geometry_geojson":
"{…}" }`, where `geometry_geojson` is a JSON-encoded GeoJSON Polygon or
MultiPolygon.

## Failure mapping

The core maps HTTP status onto typed errors. Packs should use:

| Status | Core type | Meaning |
|---|---|---|
| 401, 403 | Unauthorized | Credential missing or rejected |
| 429 | RateLimited | Back off. `Retry-After` is honored when present |
| 5xx | Upstream | Transient. Safe to retry with backoff |
| 4xx | Upstream | Request-level refusal (do not include secrets in the body) |

Error bodies are truncated to one 200-character line and never surface a
request URL, because a pack's URL may carry a token in a query string. Do not
echo credentials in error bodies.

## Pack manifest and operator config

A pack declares its routing surface in its plugin manifest so consent screens
show exactly what the core will call:

```json
{
  "artifact_kind": "http_connector",
  "capabilities": ["routing.matrix", "routing.route", "routing.geocode"],
  "connector_profile": {
    "routing": {
      "capabilities": ["matrix", "route", "geocode"],
      "modes": ["driving"],
      "base_url": "https://relay.example.com/traveltime",
      "quality": "measured",
      "time_control": "departure",
      "contract_revision": 1
    }
  }
}
```

The plugin-host validates every value against a closed vocabulary
(`core_domain::entities::plugins`). Routing capabilities resolve to the
scheduling VIEW gate. A routing grant never confers scheduling edits.

Operator configuration:

```bash
ROUTING_PROVIDER=contract
ROUTING_CONTRACT_ID=traveltime
ROUTING_CONTRACT_URL=https://relay.example.com/traveltime
ROUTING_CONTRACT_TOKEN=<operator secret>
```

The core registers the adapter under `contract:<id>`. If the pack is
unreachable, booking still succeeds. Estimates fall back to the heuristic and
the routing day view reports `provider_unavailable` per leg.

## WASM packs

Revision 1 ships the HTTP path only. A WASM routing pack would need host
functions for outbound HTTP plus a registration entry point. Neither exists
yet, so a WASM pack cannot register a routing provider today. The manifest
vocabulary and severity mapping above are the contract either path has to
satisfy, and this section is the honest gap rather than a promise.

## Fixture expectations for relay repos

Your test suite should cover, against a local fixture server:

- A 1×1 and an N×M matrix with one unreachable cell (`null` both ways).
- A matrix call carrying `depart_at`, and one where you drop it and answer
  anyway.
- Rounding: fractional seconds and metres arrive as whole numbers.
- 401, 429 with `Retry-After`, and 502 bodies, mapped to the statuses above.
- Geocode with zero hits (empty list, not an error).

The core-side equivalents live in `crates/core-routing/src/tests.rs` and
`plugins/connectors/mapbox/src/tests.rs`. Lift them as a starting point.

## What the core will not do

- No vendor names in core code, no entitlement or license checks (ADR 0013).
- No retries inside the adapter. Callers decide.
- No secret storage: operator environment or the pack, not the database.
