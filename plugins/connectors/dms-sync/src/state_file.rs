//! The durable [`SyncState`][crate::state::SyncState] ledger (Wave 15
//! lane 4 `p15-docs-closeout`): the shipped
//! [`MemorySyncState`][crate::state::MemorySyncState] keeps everything in
//! process, which forces a one-time re-adoption pass on every cold start
//! (deterministic identity keys make that SAFE — the same Meridian
//! records get found — but it still costs a pass and loses the
//! curated-conflict baseline). [`FileSyncState`] gives an operator-side
//! run durability: the ledger persists as one JSON document, atomically
//! rewritten after every write, so a restart, the next sync, or an
//! interrupted run resumes from the LAST WRITTEN baseline.
//!
//! # Failure model
//!
//! * Writes are atomic (temp file + rename): a crash kills at most the
//!   one entry being written; the previous complete ledger survives.
//! * A missing file is a normal cold start: an empty ledger initializes
//!   and the first sync re-resolves through identity keys.
//! * An unreadable file, or a ledger whose kind vocabulary no longer
//!   round-trips, REFUSES to load — an operator must hear their ledger is
//!   unreadable rather than have the connector silently re-scan (silent
//!   re-adoption can resurrect merge candidates they already
//!   adjudicated).
//! * A failed persist after a mutation is a COUNTED failure — the
//!   in-memory state is kept so the run continues, and
//!   [`FileSyncState::persist_failures`] tells the operator a write never
//!   reached the disk ledger (the next cold start will re-adopt: safe,
//!   but not the durable state they hoped for).
//!
//! The crate deliberately does NOT persist content bodies, and never
//! stores credential secrets (the crate doc's vault boundary).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::{LedgerEntry, SyncState};

/// A load/persist failure that makes durable operation impossible.
#[derive(Debug, thiserror::Error)]
pub enum FileSyncStateError {
    #[error("cannot read or write the ledger file {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Not parseable as a ledger, or carries records this build does not
    /// understand (e.g. a future kind vocabulary).
    #[error("ledger file {path} is not a readable version-1 ledger")]
    UnreadableLedger { path: PathBuf },
    #[error("ledger file {path} is version {version}; this build reads version 1 only")]
    UnknownVersion { path: PathBuf, version: u32 },
}

/// One persisted ledger record. The in-memory [`LedgerEntry`] owns a
/// `&'static str` kind (the sync engine branches on it exhaustively), so
/// the persisted form carries the runtime string and round-trips it
/// against the shipped `asset` / `document` vocabulary on load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredEntry {
    marula_kind: String,
    marula_id: Uuid,
    last_written_fields: serde_json::Value,
    last_written_fingerprint: String,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct FileLedger {
    /// 1 = current shape. Loaders refuse unknown versions rather than
    /// guess at record semantics.
    version: u32,
    entries: BTreeMap<String, StoredEntry>,
}

impl StoredEntry {
    fn from_ledger(entry: &LedgerEntry) -> Self {
        Self {
            marula_kind: entry.marula_kind.to_string(),
            marula_id: entry.marula_id,
            last_written_fields: entry.last_written_fields.clone(),
            last_written_fingerprint: entry.last_written_fingerprint.clone(),
            updated_at: entry.updated_at,
        }
    }

    fn into_ledger(self, source_key: &str) -> Option<LedgerEntry> {
        match self.marula_kind.as_str() {
            "asset" | "document" => {
                let marula_kind: &'static str = if self.marula_kind == "asset" {
                    "asset"
                } else {
                    "document"
                };
                Some(LedgerEntry {
                    source_key: source_key.to_string(),
                    marula_kind,
                    marula_id: self.marula_id,
                    last_written_fields: self.last_written_fields,
                    last_written_fingerprint: self.last_written_fingerprint,
                    updated_at: self.updated_at,
                })
            }
            _ => None,
        }
    }
}

/// File-backed [`SyncState`] ledger. Single-writer semantics: one
/// connector process owns the ledger file at a time.
#[derive(Debug)]
pub struct FileSyncState {
    path: PathBuf,
    entries: Mutex<BTreeMap<String, LedgerEntry>>,
    persist_failures: Mutex<u64>,
}

impl FileSyncState {
    /// Loads the ledger at `path`, or initializes an EMPTY one when the
    /// file does not exist. A fresh path is explicitly a cold start: SAFE
    /// (identity keys), but it skips the curated baseline until the next
    /// successful run's first write lands.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, FileSyncStateError> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Ok(Self::in_memory(path, BTreeMap::new()));
        }
        let raw = fs::read_to_string(&path).map_err(|source| FileSyncStateError::Io {
            path: path.clone(),
            source,
        })?;
        let ledger: FileLedger = serde_json::from_str(&raw)
            .map_err(|_| FileSyncStateError::UnreadableLedger { path: path.clone() })?;
        if ledger.version != 1 {
            return Err(FileSyncStateError::UnknownVersion {
                path: path.clone(),
                version: ledger.version,
            });
        }
        let mut in_memory = BTreeMap::new();
        for (source_key, stored) in ledger.entries {
            let entry = stored
                .into_ledger(&source_key)
                .ok_or_else(|| FileSyncStateError::UnreadableLedger { path: path.clone() })?;
            in_memory.insert(source_key, entry);
        }
        Ok(Self::in_memory(path, in_memory))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many mappings the ledger carries (health reporting).
    pub fn len(&self) -> usize {
        self.lock_entries().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Failed persist attempts since load. A sync engine running WITH
    /// durable semantics should compare this to 0 after the run.
    pub fn persist_failures(&self) -> u64 {
        *self
            .persist_failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn in_memory(path: PathBuf, entries: BTreeMap<String, LedgerEntry>) -> Self {
        Self {
            path,
            entries: Mutex::new(entries),
            persist_failures: Mutex::new(0),
        }
    }

    fn lock_entries(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, LedgerEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn snapshot_stored(&self, entries: &BTreeMap<String, LedgerEntry>) -> FileLedger {
        FileLedger {
            version: 1,
            entries: entries
                .iter()
                .map(|(k, v)| (k.clone(), StoredEntry::from_ledger(v)))
                .collect(),
        }
    }

    fn persist(&self, entries: &BTreeMap<String, LedgerEntry>) {
        if write_atomic(&self.path, &self.snapshot_stored(entries)).is_err() {
            *self
                .persist_failures
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        }
    }
}

fn write_atomic(path: &Path, ledger: &FileLedger) -> Result<(), FileSyncStateError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| FileSyncStateError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    }
    let json =
        serde_json::to_string_pretty(ledger).map_err(|_| FileSyncStateError::UnreadableLedger {
            path: path.to_path_buf(),
        })?;
    let temp = sibling_temp(path);
    fs::write(&temp, json).map_err(|source| FileSyncStateError::Io {
        path: temp.clone(),
        source,
    })?;
    fs::rename(&temp, path).map_err(|source| FileSyncStateError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Sibling temp name: per-process (two writers never share the temp), and
/// reclaimed by the rename.
fn sibling_temp(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "ledger".to_string());
    path.with_file_name(format!(".{}.tmp-{}", name, std::process::id()))
}

impl SyncState for FileSyncState {
    fn lookup(&self, source_key: &str) -> Option<LedgerEntry> {
        self.lock_entries().get(source_key).cloned()
    }

    fn source_key_of(&self, marula_id: Uuid) -> Option<String> {
        self.lock_entries()
            .values()
            .find(|entry| entry.marula_id == marula_id)
            .map(|entry| entry.source_key.clone())
    }

    fn remember(&self, entry: LedgerEntry) {
        let mut entries = self.lock_entries();
        entries.insert(entry.source_key.clone(), entry);
        self.persist(&entries);
    }

    fn forget(&self, source_key: &str) {
        let mut entries = self.lock_entries();
        if entries.remove(source_key).is_some() {
            self.persist(&entries);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source_key: &str, marula_id: Uuid) -> LedgerEntry {
        LedgerEntry {
            source_key: source_key.to_string(),
            marula_kind: "asset",
            marula_id,
            last_written_fields: serde_json::json!({"name": "Server A"}),
            last_written_fingerprint: "ff".to_string(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dms-sync-file-state-{}-{tag}",
            uuid::Uuid::now_v7().simple()
        ));
        fs::create_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn persists_across_restarts_and_forgetting() {
        let dir = temp_dir("persists");
        let path = dir.join("sync-ledger.json");
        let a = Uuid::from_u128(0x5A11);
        {
            let state = FileSyncState::load(&path).expect("init empty");
            assert!(state.is_empty());
            state.remember(entry("g1:s1:svc-1", a));
            assert_eq!(state.len(), 1);
            assert_eq!(state.persist_failures(), 0);
        }
        {
            let state = FileSyncState::load(&path).expect("load back");
            assert_eq!(state.len(), 1);
            let found = state.lookup("g1:s1:svc-1").expect("entry present");
            assert_eq!(found.marula_id, a);
            assert_eq!(found.last_written_fingerprint, "ff");
            assert_eq!(state.source_key_of(a).as_deref(), Some("g1:s1:svc-1"));
        }
        {
            let state = FileSyncState::load(&path).expect("load for forget");
            state.forget("g1:s1:svc-1");
            assert!(state.is_empty());
            state.forget("g1:s1:svc-1"); // double-forget stays idempotent
            assert!(state.is_empty());
        }
        {
            let state = FileSyncState::load(&path).expect("reload");
            assert!(state.is_empty());
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unreadable_and_unknown_version_refuse() {
        let dir = temp_dir("bad");
        let path = dir.join("l.json");
        fs::write(&path, "not json").unwrap();
        assert!(matches!(
            FileSyncState::load(&path),
            Err(FileSyncStateError::UnreadableLedger { .. })
        ));
        fs::write(&path, r#"{"version": 99, "entries": {}}"#).unwrap();
        assert!(matches!(
            FileSyncState::load(&path),
            Err(FileSyncStateError::UnknownVersion { .. })
        ));
        let _ = fs::remove_dir_all(dir);
    }
}
