//! `core-routing` — the distance/duration provider seam (Wave 32 lane 7).
//!
//! Purpose: one capability-generic boundary between Meridian's scheduling
//! math and ANY distance/duration API. Providers are pluggable through a
//! registry; the trait speaks matrix, route, geocode, and isochrone, with
//! departure-vs-arrival time control, travel modes, time + distance in
//! canonical units (seconds, metres), and batched matrix requests.
//!
//! Three provider families ship here:
//!
//! * [`heuristic`] — the built-in NONE-mode estimate (great-circle × road
//!   factor + overhead from the typed per-mile table). Scheduling and
//!   warnings work with no routing server; every estimate is honestly
//!   labeled [`RoutingQuality::Heuristic`].
//! * [`osrm`] — the first measured default: a self-hosted OSRM table/route
//!   server, with the OpenRouteService dialect for self-hosted ORS.
//! * [`contract`] — the out-of-tree adapter: any provider pack (first-party
//!   relay connectors like TravelTime/Geoapify, or community packs) that
//!   implements the documented Meridian Routing Contract over HTTP. The
//!   relay repos ship the vendor mapping; the core never names a vendor.
//!   See `docs/routing-providers.md` for the normative wire contract.
//!
//! First-party in-core provider: the Mapbox adapter lives in
//! `plugins/connectors/mapbox` (ADR 0013 standard set) and implements the
//! same trait.
//!
//! House precedent: `core-mail::transport::MailTransport` — trait +
//! real adapters + noop/recording doubles, vendor SDK types never leak
//! past the seam.
//!
//! Non-purpose: no scheduling policy (occupancy, warnings, suggestions
//! live in `core-domain`), no persistence (provider selection is operator
//! config; see `docs/routing-providers.md`), no credential storage.

pub mod contract;
pub mod error;
pub mod heuristic;
pub mod osrm;
pub mod provider;
pub mod registry;
pub mod types;

#[cfg(test)]
mod tests;

pub use contract::{ContractConfig, ContractRoutingProvider};
pub use error::RoutingError;
pub use heuristic::{HeuristicConfig, HeuristicRoutingProvider, HEURISTIC_PROVIDER_ID};
pub use osrm::{OsrmConfig, OsrmDialect, OsrmRoutingProvider};
pub use provider::{cell_from_values, estimate_travel, ProviderInfo, RoutingProvider};
pub use registry::{RoutingRegistry, RoutingStatus, RoutingStatusView};
pub use types::{
    estimate_minutes, GeoPoint, GeocodeHit, GeocodeRequest, IsochroneContour, IsochroneRequest,
    IsochroneResponse, MatrixAnnotations, MatrixCell, MatrixRequest, MatrixResponse, Place,
    RouteLeg, RouteRequest, RouteResponse, RoutingCapability, RoutingQuality, TimeControl,
    TravelEstimate, TravelMode, MAX_MATRIX_CELLS,
};
