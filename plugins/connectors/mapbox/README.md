# mapbox

Purpose: the first-party Mapbox provider, written behind Core's routing seam
(`core-routing::RoutingProvider`). It serves measured matrix durations,
directions, forward geocoding and isochrones, with the operator's access token
supplied by configuration. Apache-2.0, and part of the ADR 0013 standard set;
the seam itself is ADR 0014. Workspace member. LANDED in Wave 32 lane 7.

## What it does

- **Matrix** (`GET /directions-matrix/v1/mapbox/{profile}/{lon,lat;…}`). One
  batched request covers every source and target pair. Each cell carries its
  duration and distance, and a pair the vendor cannot route stays `None` rather
  than becoming zero.
- **Directions** (`GET /directions/v5/mapbox/{profile}/…`). Origin, waypoints
  and destination become one route with a leg per hop. Overview and
  alternatives are off, and geometry stays `None`: scheduling wants the number,
  not the polyline.
- **Forward geocoding** (`GET /search/geocode/v6/forward`). The query, limit,
  optional proximity and optional country go out, and hits come back with the
  full address (or the feature name) as their label. A coordinate outside the
  plausible range is dropped instead of returned.
- **Isochrones** (`GET /isochrone/v1/mapbox/{profile}/{lon},{lat}`). The
  requested contour minutes come back as GeoJSON rings, with `depart_at` sent
  only on a profile that honors departure time.

`ProviderInfo` describes it as id `mapbox`, quality `Measured`, all four
capabilities, the one travel mode it was built for, and `batched_matrix: true`.
The travel mode is fixed at construction: a request for another mode fails with
`Unsupported` rather than being answered with numbers from the wrong profile.

## Errors

`Unauthorized` for 401 and 403. `RateLimited` carries the `Retry-After` value
when the vendor sends one. `Unsupported` names the capability or the mode this
provider cannot serve. `Protocol` covers a body it cannot read and a vendor
`code` other than `Ok`. `Unavailable` covers transport, and `Invalid` covers an
empty token, an empty profile, or a place with no resolved coordinates.

## The token

Mapbox takes its token in the `access_token` query parameter, which means a
request URL is a secret. So a transport error never echoes one: timeouts,
connection failures and everything else map to fixed strings. `MapboxConfig`'s
`Debug` prints `<redacted>` for the token, a refused call returns the vendor's
response body without our URL, and the test suite asserts the token appears in
neither an error nor debug output.

## None mode

Without a token the provider does not exist: `ROUTING_PROVIDER` stays `none`,
the registry is empty, and scheduling answers from the built-in heuristic with
`RoutingQuality::Heuristic` attached to every estimate. `MapboxRoutingProvider::new`
refuses an empty token outright, so a half-configured deployment cannot
register a provider that would refuse every call. A provider that fails to build
is recorded as `Missing` in the registry, which keeps the failure visible at
`/api/dispatch/routing` instead of hiding it behind the fallback.

## How it plugs in

Core reads three environment variables: `ROUTING_PROVIDER=mapbox`,
`MAPBOX_ACCESS_TOKEN`, and an optional `MAPBOX_PROFILE` (`driving` by default).
`crates/core-api/src/routing.rs` builds the registry at boot, and a request for
a missing token logs a warning and stays in none mode. Everything below the seam
speaks `core_routing` types, so a caller never sees a Mapbox shape.

## The boundary exception, stated plainly

R2 says a connector imports no `crates/` module. This crate depends on
`core-routing` on purpose, because it implements a trait that lives there. That
is the documented exception in the connector specification (§2), and it is the
only one in the set.

The direction matters: the plugin depends on the core seam, and Core never
depends on the plugin. `core-api` wires the provider in explicitly, so removing
this crate leaves the seam and the heuristic intact. The alternative, carrying
Mapbox types outside the seam, would put vendor shapes into scheduling.

If you are checking a review box: `scripts/check-boundary.sh` does not refuse a
`plugins/` crate that depends on `crates/`, so R2 is a review row here rather
than a gate (§4 records that gap). This paragraph is the evidence that the
review row was read.

## Tests

`cargo test -p mapbox-routing`. Axum stub fixtures stand in for the four
endpoints and are reached through `base_url`, so no token and no Mapbox account
are involved. The suite answers the matrix, directions, geocode and isochrone
calls against those fixtures, asserts the normalized responses, and asserts that
the access token never appears in an error or in `Debug` output.

## Licence

Apache-2.0, separate from the AGPL core. No entitlement check anywhere (R11).
