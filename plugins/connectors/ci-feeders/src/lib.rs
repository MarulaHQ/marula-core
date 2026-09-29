//! Marula configuration-item feeder connectors (`plugins/connectors/ci-feeders`
//! — ADR 0003 CI7, Wave 12 lane 8).
//!
//! Purpose: feed device/configuration records from an MSP's source
//! systems — NinjaOne ships here; marketplace feeders (Auvik, IT Glue, and
//! community adapters) map their payloads through the same public
//! [`vendors`] surface — into Marula's configuration
//! identity core through the PUBLIC API seam only: `POST
//! /api/config-items/feed-upsert`, authenticated with a scoped machine
//! API key (`x-api-key`). No Marula internals are imported anywhere in
//! this crate.
//!
//! ADR 0003 conformance is the CI gate, and every piece of the crate
//! exists to honor it:
//!
//! * **Feeds upsert, never blind-insert.** Each record becomes ONE
//!   feed-upsert write: the core resolves it through its
//!   `(source_system, external_id)` link first, then through the
//!   deterministic identity keys — never a blind insert, and a second
//!   sync updates the link this connector created (and only its own).
//! * **Deterministic identity keys.** [`identity`] normalizes serial /
//!   service tag (uppercase alphanumeric), MACs (lower colon form),
//!   hostname + domain (lowercase, trailing dot stripped) EXACTLY per
//!   the core's documented rules, so the same physical device fed with
//!   differently-formatted payload fields resolves to the same canonical
//!   item — determinism is a property of the KEYS, not of luck.
//! * **Idempotent upsert.** Every write carries a DETERMINISTIC
//!   instant-first idempotency key derived from the record's semantic
//!   content (`ci-feeder-{source}-{external_id}-{fingerprint}`): a
//!   retried sync with unchanged content replays the stored result
//!   (never a twin), while genuinely changed content gets a fresh key
//!   and lands as the source's own update.
//! * **None mode.** [`NoneFeederSink`] is the shipped default sink:
//!   zero network I/O, every record counts as "would have synced"
//!   (standalone-first doctrine).
//!
//! # Scope boundary
//!
//! The crate maps VENDOR PAYLOADS (device/configuration records as their
//! public APIs and exports carry them) onto the feed-upsert contract and
//! pushes them. Live vendor HTTP transports (token handling, paging,
//! org→client mapping UIs) ride the adapters tranche; the mapping
//! functions here are pure and fixture-tested, so a transport only needs
//! to fetch JSON and call [`vendors`].

pub mod identity;
pub mod model;
pub mod sink;
pub mod sync;
pub mod vendors;

#[cfg(test)]
mod tests;

pub use identity::IdentityKeysBuilder;
pub use model::FeederRecord;
pub use sink::{
    FeedUpsertOutcome, FeederSink, MarulaFeederConfig, MarulaFeederSink, NoneFeederSink,
    SinkError,
};
pub use sync::{ApplyMode, FeederSync, SyncConfig, SyncReport};
