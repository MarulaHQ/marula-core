//! Pull side: the [`DmsSource`] seam. A source turns one external
//! documentation system's state into a vendor-neutral [`DmsSnapshot`].
//! Shipped implementations:
//!
//! * [`hudu::HuduExport`] — Hudu's export JSON (asset_layouts, assets,
//!   websites, passwords); fixture-testable, zero network.
//! * [`live::HuduLive`] — LIVE REST pulls from the Hudu API with its
//!   native pagination/auth flow (same snapshot vocabulary as the
//!   export parser). IT Glue's source (export, live pull, renderer)
//!   ships as the marketplace `it-glue` connector instead (ADR 0013).
//! * [`rest::RestSource`] — a generic REST DMS: list endpoints +
//!   a declarative field mapping (the escape hatch for "none of the
//!   above" documentation systems).
//!
//! Non-purpose: no incremental/delta protocol (a sync pass consumes one
//! whole snapshot — exports are small and idempotent upserts make full
//! pulls the safe default), no source-side credential handling beyond
//! what the export itself contains (the vault boundary lives in the
//! sink handoff), and no vendor SDKs — HTTP is plain reqwest.

use crate::model::{DmsSnapshot, SyncError};

/// The source seam.
pub trait DmsSource: Send + Sync {
    /// Source system identifier for provenance ("it-glue", "hudu", ...).
    fn name(&self) -> &str;

    /// Parse/pull one full snapshot.
    fn pull(&self) -> Result<DmsSnapshot, SyncError>;
}

pub mod hudu;
pub mod live;
pub mod rest;

/// Typed errors surfaced by source adapters.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    #[error("missing export part: {0}")]
    MissingPart(String),
    #[error("unsupported export shape: {0}")]
    Shape(String),
}
