//! First-party Mapbox provider (ADR 0013 standard set) behind the
//! `core-routing` seam.
//!
//! Purpose: real network durations/distances (Matrix + Directions),
//! forward geocoding, and reachability isochrones from Mapbox, with the
//! operator's access token supplied by config. The typical use is the
//! measured travel estimate and the dispatcher's geocode-on-save path.
//!
//! None-mode: without an access token the provider is simply absent from
//! the registry — scheduling falls back to the built-in heuristic and
//! never dials Mapbox. The token never lands in a URL the crate logs, in
//! an error excerpt, or in `Debug` output (see [`MapboxConfig`]'s manual
//! `Debug`).
//!
//! Non-purpose: token storage/rotation (operator config), matrix caching
//! (callers may batch), or vendor types leaking past the seam — every
//! response normalizes onto `core_routing::types`.

pub mod provider;

#[cfg(test)]
mod tests;

pub use provider::{MapboxConfig, MapboxRoutingProvider};
