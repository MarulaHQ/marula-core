//! Marula DMS sync connector (`connectors/dms-sync` — the private build tracker Money +
//! docs T3, external-DMS parity directive 2026-09-10).
//!
//! Purpose: move an org's documentation BETWEEN an external documentation
//! system (IT Glue, Hudu, or a generic REST DMS) and Marula's
//! docs-platform without losing data — via the PUBLIC Marula API seam
//! only (the same pattern the NinjaOne/Hudu reference connectors prove:
//! scoped machine API key, resource REST, instant-first envelope).
//!
//! The crate is deliberately self-contained: no Marula internals are
//! imported. It is a library of four pieces:
//!
//! * **Pull** — [`source`]: a [`DmsSource`] parses vendor exports (IT
//!   Glue JSON:API export files, Hudu export JSON) or speaks a generic
//!   REST DMS, producing one vendor-neutral [`DmsSnapshot`] of layouts,
//!   assets, documents, links, and (gated) credentials.
//! * **Push** — [`push`]: parity in the other direction. A Marula
//!   snapshot read through the public API is rendered back OUT in the
//!   same export format (`render_hudu_export`) or pushed record-at-a-time
//!   to a generic REST endpoint. IT Glue's source and renderer ship as
//!   the marketplace `it-glue` connector (ADR 0013).
//! * **Sink** — [`sink`]: the Marula-side apply seam ([`DmsSink`]) plus
//!   the shipped [`MarulaRestSink`] and the documented
//!   [`NoneDmsSink`]. External layouts map onto `document_layouts` (org
//!   layouts are created for external types; seeded `is_system` rows are
//!   READ and matched, never mutated), asset instances land in
//!   `device_assets` with typed classification (`layout_id` +
//!   schema-validated `fields`), and relationships land in
//!   `document_links` (resolved labels; unresolved ends are surfaced in
//!   the run report, never silently dropped).
//! * **Engine** — [`sync`]: ADR 0003 semantics. Feeds UPSERT through
//!   deterministic identity keys (source GUID, serial, hostname+domain):
//!   a source's second sync updates its own record, never creates a
//!   twin. Conflicts become reviewable merge candidates — never silent
//!   overwrites. Every write rides the public instant-first envelope, so
//!   the server's audit trail, provenance browser, and IF7 undo apply
//!   unchanged; the connector adds provenance via change summaries and a
//!   `source_ref` field on the layouts it owns. Writes reuse the
//!   registered `asset.*` / `document.*` contract events by construction
//!   (they are emitted by the core's own routes) — this crate introduces
//!   no new event families.
//!
//! # Vault boundary (binding)
//!
//! External credentials/passwords are NOT pulled into Marula unless the
//! operator explicitly enables credential mapping
//! (`SyncConfig::map_credentials`, default `false`). When enabled they
//! are handed to the sink's vault path once — the audited
//! `POST /api/passwords` create, which seals the secret AES-256-GCM at
//! rest server-side — and are never logged, serialized into reports, or
//! retained in connector state. There is no blanket credential surface:
//! what an agent sees remains whatever the `Module::Passwords` matrix
//! already allows their role.
//!
//!  # Connector toggle
//!
//! The org-level `connectors.dms_sync` setting is the operator-side
//! enable/disable surface for this connector (default ON; Wave 15 lane
//! 4): a sync run that starts while it is off reports disabled before
//! touching a single Marula record.
//!
//! None mode
//!
//! [`NoneDmsSink`] is the shipped default sink: zero network I/O, every
//! call returns [`SinkError::Disabled`]. An org with no Marula
//! connection configured operates standalone (standalone-first
//! doctrine); every call site must treat `Disabled` as "would have
//! synced". The [`DmsSource`] side has no implicit none mode — sources
//! are constructed with explicit input (a parsed export or a REST base
//! URL), so absence of a source is a construction-time decision, not a
//! runtime surprise.
//!
//! # Non-purposes
//!
//! No OAuth/token refresh (operators rotate scoped API keys in their own
//! secret store), no Marula schema changes (external shapes map onto
//! `document_layouts` / `device_assets` / `document_links` as-is), no
//! credential storage on the connector side, no server-side merge-queue
//! UI (conflicts are reported to the operator in the run report; the
//! ADR 0003 merge-queue surface is a future ConfigItem-track concern),
//! and no IT Glue/Hudu write APIs beyond the generic REST pusher (parity
//! push ships the export renderers first).

pub mod identity;
pub mod model;
pub mod push;
pub mod sink;
pub mod source;
pub mod state;
pub mod state_file;
pub mod sync;

#[cfg(test)]
mod tests;

pub use identity::{Identity, IdentityKey, IdentityKeyKind};
pub use model::{
    Credential, DmsSnapshot, FieldKind, LinkEndpoint, LinkKind, Relation, SyncField, SyncLayout,
    SyncRecord,
};
pub use sink::{DmsSink, SinkError};
pub use source::{DmsSource, SourceError};
pub use state::{LedgerEntry, MemorySyncState, SyncState};
pub use state_file::{FileSyncState, FileSyncStateError};
pub use sync::{ApplyMode, SyncConfig, SyncEngine, SyncReport};
