//! Shared live-REST transport primitives for Meridian's first-party
//! connector adapters (adapters tranche 2, Wave 12 lane 9).
//!
//! The lane-8 feeder crates established the SEMANTIC half of the
//! adapter seam: vendor payloads normalize onto the feed contract and
//! push through the public feed-upsert API. What was left to the
//! adapters tranche is the TRANSPORT half — the parts every live vendor
//! pull needs and no two vendors do identically:
//!
//! * **Vendor auth.** [`auth::VendorAuth`] carries the three flows the
//!   supported vendors use: a static key under a named header (IT
//!   Glue: `x-api-key`), a bearer token the operator supplies (ImmyBot,
//!   Hudu), and the OAuth2 client-credentials grant (M365 Graph) with
//!   an in-process token cache that refreshes ahead of expiry.
//! * **Native pagination flows.** [`client::VendorClient`] pages JSON
//!   lists the way each vendor actually pages them: M365 Graph's
//!   `@odata.nextLink` continuation URLs, IT Glue JSON:API's
//!   `links.next` + `page[number]`, and plain page/size parameters with
//!   short-page termination (ImmyBot, Hudu). A pull follows every page
//!   or fails with a typed error — a silently truncated inventory is a
//!   wrong inventory.
//! * **Vendor-org → Meridian-client mapping.** [`org_map`]:
//!   the operator's explicit mapping, resolved deterministically —
//!   an unmapped vendor org is an ERROR, never a guess (records are
//!   never attributed to a client the operator did not name).
//!
//! Everything here speaks plain HTTP + JSON: no Meridian internals are
//! imported, so connector crates (feeder-line and DMS-line alike) share
//! one transport vocabulary. Credential material is never logged —
//! `Debug` impls redact.

pub mod auth;
pub mod client;
pub mod org_map;

#[cfg(test)]
mod tests;

pub use auth::VendorAuth;
pub use client::{PageFlow, VendorClient, VendorError};
pub use org_map::{OrgMapping, OrgMappingBuilder, OrgMappingError};
