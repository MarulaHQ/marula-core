//! The connector-side identity ledger (`SyncState`) — the memory that
//! makes "second sync updates its own record, never creates a twin"
//! true across runs.
//!
//! The ledger records, per source key: which Meridian record the source
//! record resolved to, what the connector last wrote there (fields +
//! fingerprint), and when. Resolution always consults the ledger FIRST;
//! deterministic identity keys are the fallback (a lost ledger must not
//! create twins — serial/hostname matches rediscover the record), and
//! anything ambiguous surfaces as a reviewable merge candidate.
//!
//! In-process default keeps everything in memory; runs that need
//! durability use [`crate::state_file::FileSyncState`] (shipped Wave 15
//! lane 4: one atomically-rewritten JSON file). Operators with their own
//! storage implement the [`SyncState`] trait directly — the crate
//! deliberately does NOT persist content bodies, and never stores
//! credential secrets (see the crate doc's vault boundary).

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What the connector last wrote to one Meridian record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// The source record's deterministic handle
    /// (`Identity::handle`): `<guid>:<source>:<external_id>`.
    pub source_key: String,
    /// Which Meridian side the record lives on ("asset" | "document").
    pub meridian_kind: &'static str,
    pub meridian_id: Uuid,
    /// The field values the connector wrote (the curated-conflict
    /// baseline: a field that has since drifted from this is either
    /// source churn or a human edit — the engine distinguishes them).
    pub last_written_fields: serde_json::Value,
    /// Content fingerprint of the source record at write time.
    pub last_written_fingerprint: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl LedgerEntry {
    pub fn is_asset(&self) -> bool {
        self.meridian_kind == "asset"
    }

    pub fn is_document(&self) -> bool {
        self.meridian_kind == "document"
    }
}

/// The ledger seam. Keyed by source key and Meridian record id.
pub trait SyncState: Send + Sync {
    fn lookup(&self, source_key: &str) -> Option<LedgerEntry>;
    /// Reverse direction: which source key owns one Meridian record.
    fn source_key_of(&self, meridian_id: Uuid) -> Option<String>;
    fn remember(&self, entry: LedgerEntry);
    /// Forget one mapping (e.g. the sink 404'd on an update — the
    /// record was deleted Meridian-side; the next sync re-resolves).
    fn forget(&self, source_key: &str);
}

/// In-process ledger (the shipped default). Cheap, no persistence —
/// every process start re-resolves deterministic identities, which is
/// SAFE: identity keys are deterministic, so a cold ledger still
/// updates the same Meridian record instead of creating a twin.
#[derive(Debug, Default)]
pub struct MemorySyncState {
    by_source_key: Mutex<BTreeMap<String, LedgerEntry>>,
    by_meridian_id: Mutex<BTreeMap<Uuid, String>>,
}

impl MemorySyncState {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SyncState for MemorySyncState {
    fn lookup(&self, source_key: &str) -> Option<LedgerEntry> {
        self.by_source_key
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(source_key)
            .cloned()
    }

    fn source_key_of(&self, meridian_id: Uuid) -> Option<String> {
        self.by_meridian_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&meridian_id)
            .cloned()
    }

    fn remember(&self, entry: LedgerEntry) {
        let mut by_key = self
            .by_source_key
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut by_id = self
            .by_meridian_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(previous) = by_key.get(&entry.source_key) {
            by_id.remove(&previous.meridian_id);
        }
        by_id.insert(entry.meridian_id, entry.source_key.clone());
        by_key.insert(entry.source_key.clone(), entry);
    }

    fn forget(&self, source_key: &str) {
        let mut by_key = self
            .by_source_key
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = by_key.remove(source_key) {
            self.by_meridian_id
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&entry.meridian_id);
        }
    }
}
