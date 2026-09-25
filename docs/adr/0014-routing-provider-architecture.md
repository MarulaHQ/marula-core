# ADR 0014: Routing providers — seam, built-ins, and the core/marketplace split

Status: accepted (Wave 32 lane 7, 2026-09-25)

## Context

Scheduling shipped with `travel_minutes` as a display placeholder: the column
existed but no rule read it (`crates/core-domain/src/scheduling.rs` said so in
a comment). Dispatch v3 has to turn travel into real capacity math, which means
answering one question everywhere: how long does a leg take?

Distance and duration APIs differ in shape. Some expose a batched N×M matrix,
some only routes, geocoding, or isochrones. Some want departure time, some
arrival time, some neither. Operators want to point Meridian at whatever they
already run: a self-hosted OSRM server, a Mapbox token, or a commercial API we
do not ship.

Two constraints shape the answer. Scheduling must work with no routing server
at all, because self-hosted installs are the default posture. And the core is
open source while premium connectors are not, so vendor-specific integrations
must be able to live outside this repository without core changes (ADR 0013).

## Decision

**One trait owns the boundary.** `crates/core-routing` defines
`RoutingProvider`: matrix, route, geocode, isochrone, travel modes, departure
versus arrival control, canonical seconds and metres. Providers declare what
they support in `ProviderInfo`. An undeclared call is a typed refusal, never an
attempt. `core-mail`'s `MailTransport` is the precedent — trait, real adapters,
and no vendor types past the seam.

**A registry holds the active provider.** `RoutingRegistry` stores providers by
id, tracks the operator-configured id, and exposes a typed status:
`NoneMode`, `Missing { configured }`, or `Configured(info)`. Misconfiguration
never fails boot and never fails a booking.

**NONE mode is a real provider, labeled honestly.** The built-in
`HeuristicRoutingProvider` (id `none`) estimates from great-circle distance ×
road factor + overhead, using the org's per-mile table from typed settings.
Every estimate it returns carries `quality: "heuristic"`. Measured providers
carry `"measured"`. Nothing downstream can mistake one for the other.

**Travel is occupied capacity. Travel adds, never replaces.**
`core-domain::scheduling::occupied_minutes` merges each appointment's span with
the travel block before it, so travel is counted once and capacity math stays
honest. The appointment's own time is untouched.

**Only measured providers produce suggestions.** A heuristic provider does not.
Suggestion-grade features (adjacent-swap, reassign) run only when the active
provider's quality is `measured`. In NONE mode the day view returns
`suggestions.available = false` and renders no suggestion chrome at all. First
measured default: self-hosted OSRM/OpenRouteService (`crates/core-routing`).
First-party hosted default: Mapbox (`plugins/connectors/mapbox`, Apache-2.0,
ADR 0013 standard set).

**Out-of-tree packs speak a contract, not our crate.** A relay or community
provider is an HTTP service implementing the Meridian Routing Contract
(`docs/routing-providers.md`, revision 1) at `/routing/matrix`,
`/routing/route`, `/routing/geocode`, `/routing/isochrone`. The core's
`ContractRoutingProvider` adapter calls it. The operator config names it
`ROUTING_PROVIDER=contract`. Packs declare capabilities, modes, declared quality, and
time control in their plugin manifest (`connector_profile.routing`), which the
plugin-host validates against a closed vocabulary. The core holds no vendor
mapping, no entitlement check, and no license logic — per ADR 0013, marketplace
packs gate themselves. TravelTime and Geoapify are therefore relay connectors:
this repository documents the contract and ships nothing vendor-specific
beyond Mapbox.

**Provider config is operator infrastructure, policy is org settings.**
Selection, base URLs, and tokens come from environment variables, parsed the
way the email transport is (a token is not a preference). The heuristic table
and mileage rates are org-scope typed settings, because they encode policy the
dispatcher can see and change.

## Consequences

- Self-hosted installs get travel estimates with zero extra services. Every
  surface labels them as estimates.
- Adding a commercial routing vendor is a relay repository plus operator
  config, not a core release. The contract doc is the interface. Revision
  changes are additive until the core can call a new revision.
- Dispatchers can set a day start point per technician per day: at home, at
  the office, at a site, or at an address. The auto-suggestion, when no override
  exists, continues from yesterday's last site. Travel estimates run from that
  point, and the snapshot lands on the appointment so old estimates stay
  explainable.
- Real API keys never live in org settings, the database, or a URL we log:
  Mapbox's config redacts its token from `Debug`, and transport errors map to
  fixed strings.
- There is no route cache and no cross-request matrix reuse. A dispatch board
  load costs one batched matrix per technician, plus bounded reassignment
  probes. Heavy installs may want a cache later, and that is a deliberate
  deferral rather than an oversight.
