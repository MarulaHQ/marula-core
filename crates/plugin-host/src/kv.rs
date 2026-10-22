//! Namespaced plugin state: the host side of `kv_get`/`kv_put`/`kv_delete`/`kv_list`
//! (W53-6, ADR 0023 Decision 5).
//!
//! The guest-facing entries stay in [`crate::host_fns::kv`], which is where the
//! gate and the ABI validation live. This module owns everything behind them:
//! the store seam, the two quota numbers Decision 4 fixes, the list-page codec
//! the PDK already documents, and a reference implementation the tests drive.
//!
//! ## Why a store handle and not a database handle
//!
//! A host function's signature is
//! `fn(&mut InvocationState, &dyn GuestMemory, &[i32]) -> Result<HostReturn, Refusal>`
//! and it is synchronous: `isolation::dispatch` calls it from inside a wasmtime
//! `Caller`, which cannot await. `InvocationState` carries the invocation's
//! identity and nothing else — by design, so a host function cannot reach past
//! the invocation it was given.
//!
//! So the store arrives through a handle this module owns. [`install`] sets it
//! once per process (the worker or an `in_process` core-api), [`with_store`]
//! sets it for one thread (a test, or a caller that owns the scope), and
//! [`current`] resolves thread-first then process. A worker is single-threaded,
//! so for the shipped isolation profile the two are the same thing.
//!
//! **When no store is installed every verb refuses `plugin_not_implemented`.**
//! That is not a placeholder: the entry exists and its body is written, but
//! nothing in this lane's files boots the worker with a store, so a process with
//! none genuinely cannot serve plugin state. The wiring is the seam this module
//! publishes — `install(Arc<dyn KvStore>)` at process start — and the two
//! implementations it can be handed are [`MemoryKvStore`] here and
//! `core_db::plugin_kv`'s SQL behind a blocking adapter, which needs a runtime
//! the worker binary does not have yet.
//!
//! ## The two quota numbers
//!
//! `abi` carries every limit a GUEST can read or act on. It does not carry the
//! per-install quota, because a guest cannot act on it: it is the store's
//! accounting policy, and [`crate::host_fns::kv`] is a thin enough layer that a
//! number there would be a number in two places. They live here, beside the
//! store that enforces them, and `core_db::plugin_kv` takes them as arguments
//! the way `plugin_health` takes its quarantine threshold.
//!
//! ## One arithmetic, two enforcers
//!
//! [`admit`] is the quota rule as a pure function. The authoritative enforcer is
//! `core_db::plugin_kv::put`, inside the write's own transaction and under the
//! counter row's lock; [`MemoryKvStore`] calls [`admit`] so a test can prove the
//! boundary without a database, and `db_suite` asserts the same boundary
//! against Postgres. Two tests, one rule, and a disagreement between them fails
//! a build rather than a customer.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::abi::{self, Refusal};

/// Keys one install may hold (ADR 0023 Decision 4: "KV keys per install").
pub const KV_KEYS_PER_INSTALL: i64 = 100_000;

/// Bytes one install may hold (ADR 0023 Decision 4: "KV bytes per install").
pub const KV_BYTES_PER_INSTALL: i64 = 256 * 1024 * 1024;

/// Keys one `kv_list` page returns, as the store's own `i64` so a limit and a
/// `LIMIT` are the same type. The number itself is [`abi::KV_LIST_PAGE`].
pub const KV_LIST_PAGE: i64 = abi::KV_LIST_PAGE as i64;

/// An install's quota position, as the store accounts for it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KvUsage {
    pub bytes: i64,
    pub keys: i64,
}

/// One page of keys. `next_cursor` is `None` on the last page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KvKeyPage {
    pub keys: Vec<String>,
    pub next_cursor: Option<String>,
}

/// Why a store call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KvError {
    /// The write would cross [`KV_BYTES_PER_INSTALL`] or
    /// [`KV_KEYS_PER_INSTALL`]. The write did not happen.
    QuotaExceeded,
    /// No store is installed in this process, so the verb cannot be served.
    Unavailable,
    /// The store's backend failed. The message is for the log, never for the
    /// guest.
    Backend(String),
}

impl KvError {
    /// The typed refusal the guest sees.
    ///
    /// `Backend` maps onto [`Refusal::WorkerFailed`] because the ABI's closed
    /// vocabulary has no "the store is down" code and adding one is an ABI
    /// change this lane is not allowed to make. That mapping is a real
    /// compromise, not a neat fit: it reports a backend outage against the
    /// install, so the pool's three-strike quarantine would eventually stop an
    /// install that did nothing wrong. The fix is an append-only
    /// `plugin_kv_unavailable` code (25) in the next ABI amendment, and until
    /// then a store implementation should prefer to answer from whatever it can
    /// rather than surface `Backend`.
    pub fn refusal(&self) -> Refusal {
        match self {
            KvError::QuotaExceeded => Refusal::KvQuotaExceeded,
            KvError::Unavailable => Refusal::NotImplemented,
            KvError::Backend(_) => Refusal::WorkerFailed,
        }
    }
}

/// The install-scoped store the four verbs call.
///
/// Deliberately four methods and no more: it mirrors the ABI's verbs exactly, so
/// a store cannot quietly grow an operation the guest has no way to reach and
/// the export/erase paths (which are not host functions) stay in `core-db` where
/// their receipts live.
///
/// Every method takes `org_id` and `install_id` from the invocation state, never
/// from the frame's payload: the guest supplies a label and a key, and the host
/// fills the two identities that decide which rows are reachable.
pub trait KvStore: Send + Sync {
    fn get(
        &self,
        org_id: Uuid,
        install_id: Uuid,
        namespace: &str,
        key: &str,
    ) -> Result<Option<Vec<u8>>, KvError>;

    fn put(
        &self,
        org_id: Uuid,
        install_id: Uuid,
        namespace: &str,
        key: &str,
        value: &[u8],
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(), KvError>;

    /// `Ok(false)` when the key was not there. Deleting an absent key is
    /// success, the way every other store behaves.
    fn delete(
        &self,
        org_id: Uuid,
        install_id: Uuid,
        namespace: &str,
        key: &str,
    ) -> Result<bool, KvError>;

    fn list(
        &self,
        org_id: Uuid,
        install_id: Uuid,
        namespace: &str,
        prefix: &str,
        cursor: Option<&str>,
    ) -> Result<KvKeyPage, KvError>;
}

/// The quota rule, as a pure function: given the counter, the size of the value
/// this write replaces (if any) and the new value's length, either the next
/// counter values or [`KvError::QuotaExceeded`].
///
/// An overwrite moves the byte total by the DIFFERENCE and leaves the key count
/// alone, so rewriting a key at the same size costs nothing against the quota —
/// a plugin refreshing a cached entry every minute is not punished for it.
pub fn admit(
    used: KvUsage,
    prior_bytes: Option<i64>,
    value_len: usize,
    max_bytes: i64,
    max_keys: i64,
) -> Result<KvUsage, KvError> {
    let prior = prior_bytes.unwrap_or(0);
    let next = KvUsage {
        bytes: used.bytes - prior + value_len as i64,
        keys: used.keys + i64::from(prior_bytes.is_none()),
    };
    if next.bytes > max_bytes || next.keys > max_keys {
        return Err(KvError::QuotaExceeded);
    }
    Ok(next)
}

// ---------------------------------------------------------------------------
// The handle
// ---------------------------------------------------------------------------

static PROCESS_STORE: RwLock<Option<Arc<dyn KvStore>>> = RwLock::new(None);

thread_local! {
    /// Set by [`with_store`] for one thread. Wins over the process store so a
    /// test can drive a harness without touching what the process installed.
    static THREAD_STORE: RefCell<Option<Arc<dyn KvStore>>> = const { RefCell::new(None) };
}

/// Installs the process's store and returns whatever was there before.
///
/// Called once at process start: by the worker for `pool` isolation, by core-api
/// for `in_process`. Calling it twice replaces the store rather than panicking,
/// because a test binary that builds two runtimes would otherwise be unable to
/// proceed — the return value is how a caller restores the previous one.
pub fn install(store: Arc<dyn KvStore>) -> Option<Arc<dyn KvStore>> {
    let mut slot = PROCESS_STORE
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    slot.replace(store)
}

/// The store this thread resolves to, if any.
pub fn current() -> Option<Arc<dyn KvStore>> {
    if let Some(store) = THREAD_STORE.with(|slot| slot.borrow().clone()) {
        return Some(store);
    }
    PROCESS_STORE
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// Runs `f` with `store` as this thread's store, restoring the previous value
/// afterwards — including when `f` panics, which is what makes it safe to use
/// from a test that may fail an assertion.
pub fn with_store<T>(store: Arc<dyn KvStore>, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Arc<dyn KvStore>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            THREAD_STORE.with(|slot| *slot.borrow_mut() = previous);
        }
    }

    let previous = THREAD_STORE.with(|slot| slot.borrow_mut().replace(store));
    let _restore = Restore(previous);
    f()
}

// ---------------------------------------------------------------------------
// The list page, as the PDK documents it
// ---------------------------------------------------------------------------

/// The page the guest parses: `{"keys": [...], "cursor": "..."}`, exactly the
/// shape `packages/plugin-kit/pdk/rust/src/host.rs` documents for `kv_list`.
/// `cursor` is the empty string on the last page, which is also what a guest
/// passes to ask for the first one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WirePage {
    keys: Vec<String>,
    cursor: String,
}

/// Encodes one page as the guest's reply payload.
pub fn encode_page(page: &KvKeyPage) -> Vec<u8> {
    let wire = WirePage {
        keys: page.keys.clone(),
        cursor: page.next_cursor.clone().unwrap_or_default(),
    };
    // A page of keys is `String`s, so this cannot fail; an encoding bug would
    // have to be a `serde` defect, and an empty page is a worse answer than a
    // loud one.
    serde_json::to_vec(&wire).unwrap_or_else(|_| b"{\"keys\":[],\"cursor\":\"\"}".to_vec())
}

// ---------------------------------------------------------------------------
// The reference store
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Stored {
    value: Vec<u8>,
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct Memory {
    /// Keyed by `(install_id, namespace, key)`. `BTreeMap` rather than a hash
    /// map because `kv_list` and the export both promise an ORDER, and a test
    /// that has to sort its own expectations proves nothing about the store's.
    entries: BTreeMap<(Uuid, String, String), Stored>,
    usage: BTreeMap<Uuid, KvUsage>,
}

/// An in-memory store that enforces the same quota, TTL and ordering rules as
/// the SQL one.
///
/// Two uses, and it is not a production default: it is what
/// `cargo test -p plugin-host` drives the four host entries against (that suite
/// must pass with no database), and it is what a caller can install to see the
/// verbs work before the SQL store is wired.
pub struct MemoryKvStore {
    max_bytes: i64,
    max_keys: i64,
    page: i64,
    inner: Mutex<Memory>,
}

impl Default for MemoryKvStore {
    fn default() -> Self {
        MemoryKvStore::new()
    }
}

impl MemoryKvStore {
    /// The ADR's numbers.
    pub fn new() -> Self {
        MemoryKvStore::with_limits(KV_BYTES_PER_INSTALL, KV_KEYS_PER_INSTALL, KV_LIST_PAGE)
    }

    /// Narrowed limits, for a test that cannot allocate 256 MiB to find the
    /// boundary. The production construction is [`MemoryKvStore::new`].
    pub fn with_limits(max_bytes: i64, max_keys: i64, page: i64) -> Self {
        MemoryKvStore {
            max_bytes,
            max_keys,
            page,
            inner: Mutex::new(Memory::default()),
        }
    }

    /// The counter, for a test that wants to watch it move.
    pub fn usage(&self, install_id: Uuid) -> KvUsage {
        self.lock()
            .usage
            .get(&install_id)
            .copied()
            .unwrap_or_default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Memory> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn live(stored: &Stored, now: DateTime<Utc>) -> bool {
    stored.expires_at.is_none_or(|expires_at| expires_at > now)
}

impl KvStore for MemoryKvStore {
    fn get(
        &self,
        _org_id: Uuid,
        install_id: Uuid,
        namespace: &str,
        key: &str,
    ) -> Result<Option<Vec<u8>>, KvError> {
        let memory = self.lock();
        let now = Utc::now();
        Ok(memory
            .entries
            .get(&(install_id, namespace.to_string(), key.to_string()))
            .filter(|stored| live(stored, now))
            .map(|stored| stored.value.clone()))
    }

    fn put(
        &self,
        _org_id: Uuid,
        install_id: Uuid,
        namespace: &str,
        key: &str,
        value: &[u8],
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(), KvError> {
        // No size check here: the host refused an oversized value with
        // `plugin_kv_value_too_large` before the store was called, and the SQL
        // store repeats it as a typed conflict for a caller that bypassed the
        // host. A third copy in a test double would only be a third thing to
        // drift.
        let mut memory = self.lock();
        let id = (install_id, namespace.to_string(), key.to_string());
        // An expired row is still charged to the quota until it is swept, which
        // is the SQL store's rule too: the counter tracks what the table holds,
        // not what a reader can see. Keeping the two identical is the point of
        // the double.
        let prior = memory
            .entries
            .get(&id)
            .map(|stored| stored.value.len() as i64);
        let used = memory.usage.get(&install_id).copied().unwrap_or_default();
        let next = admit(used, prior, value.len(), self.max_bytes, self.max_keys)?;
        memory.entries.insert(
            id,
            Stored {
                value: value.to_vec(),
                expires_at,
            },
        );
        memory.usage.insert(install_id, next);
        Ok(())
    }

    fn delete(
        &self,
        _org_id: Uuid,
        install_id: Uuid,
        namespace: &str,
        key: &str,
    ) -> Result<bool, KvError> {
        let mut memory = self.lock();
        let id = (install_id, namespace.to_string(), key.to_string());
        let Some(removed) = memory.entries.remove(&id) else {
            return Ok(false);
        };
        let used = memory.usage.get(&install_id).copied().unwrap_or_default();
        memory.usage.insert(
            install_id,
            KvUsage {
                bytes: (used.bytes - removed.value.len() as i64).max(0),
                keys: (used.keys - 1).max(0),
            },
        );
        Ok(true)
    }

    fn list(
        &self,
        _org_id: Uuid,
        install_id: Uuid,
        namespace: &str,
        prefix: &str,
        cursor: Option<&str>,
    ) -> Result<KvKeyPage, KvError> {
        let memory = self.lock();
        let now = Utc::now();
        let cursor = cursor.unwrap_or("");
        let mut keys: Vec<String> = memory
            .entries
            .iter()
            .filter(|((install, ns, key), stored)| {
                *install == install_id
                    && ns == namespace
                    && live(stored, now)
                    && key.starts_with(prefix)
                    && (cursor.is_empty() || key.as_str() > cursor)
            })
            .map(|((_, _, key), _)| key.clone())
            .take(self.page as usize + 1)
            .collect();
        let next_cursor = if keys.len() as i64 > self.page {
            keys.truncate(self.page as usize);
            keys.last().cloned()
        } else {
            None
        };
        Ok(KvKeyPage { keys, next_cursor })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_fns::{Harness, HostReturn, InvocationState};

    fn state() -> InvocationState {
        InvocationState::new(Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7())
    }

    /// The value a `Data` reply carries, or the refusal code it carried.
    fn payload(result: Result<HostReturn, Refusal>) -> Result<Vec<u8>, Refusal> {
        match result? {
            HostReturn::Data(frame) => abi::parse_frame(&frame).map(<[u8]>::to_vec),
            other => panic!("expected a data frame, got {other:?}"),
        }
    }

    /// `kv_put` the way the guest calls it: `(ns, ns_len, key, key_len, value, value_len)`.
    fn put_call(
        harness: &mut Harness,
        namespace: &str,
        key: &str,
        value: &[u8],
    ) -> Result<HostReturn, Refusal> {
        let (ns, ns_len) = harness.put_str(namespace);
        let (k, k_len) = harness.put_str(key);
        let v = harness.put(value);
        harness.call(
            "kv_put",
            &[
                ns as i32,
                ns_len as i32,
                k as i32,
                k_len as i32,
                v as i32,
                value.len() as i32,
            ],
        )
    }

    fn get_call(harness: &mut Harness, namespace: &str, key: &str) -> Result<HostReturn, Refusal> {
        let (ns, ns_len) = harness.put_str(namespace);
        let (k, k_len) = harness.put_str(key);
        harness.call(
            "kv_get",
            &[ns as i32, ns_len as i32, k as i32, k_len as i32],
        )
    }

    #[test]
    fn a_value_round_trips_as_bytes_the_core_never_parses() {
        let store = Arc::new(MemoryKvStore::new());
        with_store(store.clone(), || {
            let mut harness = Harness::new(state());
            let install = harness.state().install_id;
            // Not UTF-8: the store is bytes, and a value the core tried to parse
            // would come back changed or refused.
            let value = vec![0xff, 0x00, 0xfe, 0x7f];
            assert_eq!(
                put_call(&mut harness, "cache", "k", &value),
                Ok(HostReturn::Status(0))
            );
            assert_eq!(
                payload(get_call(&mut harness, "cache", "k")),
                Ok(value.clone())
            );
            assert_eq!(store.usage(install), KvUsage { bytes: 4, keys: 1 });
        });
    }

    #[test]
    fn an_absent_key_is_an_empty_frame_and_not_a_refusal() {
        // The ADR's refusal table gives `kv_get` exactly one code
        // (`plugin_kv_namespace_denied`), so absence is not a refusal: an absent
        // key and an empty value are the same answer, and `kv_list` is how a
        // guest tells them apart.
        let store = Arc::new(MemoryKvStore::new());
        with_store(store, || {
            let mut harness = Harness::new(state());
            assert_eq!(
                payload(get_call(&mut harness, "cache", "absent")),
                Ok(Vec::new())
            );
        });
    }

    #[test]
    fn a_namespace_outside_the_label_alphabet_is_refused() {
        let store = Arc::new(MemoryKvStore::new());
        with_store(store, || {
            let mut harness = Harness::new(state());
            for bad in ["", "Cache", "ca che", "cache/slash", &"a".repeat(65)] {
                assert_eq!(
                    put_call(&mut harness, bad, "k", b"v"),
                    Err(Refusal::KvNamespaceDenied),
                    "{bad:?} must not be a namespace"
                );
            }
            // The alphabet the ABI allows, including the three punctuation marks.
            assert_eq!(
                put_call(&mut harness, "a-z_0.9", "k", b"v"),
                Ok(HostReturn::Status(0))
            );
        });
    }

    #[test]
    fn a_key_over_512_bytes_is_a_frame_refusal_not_a_store_refusal() {
        // The ADR has no `plugin_kv_key_*` code, so a malformed key is a
        // malformed frame. Inventing a second namespace refusal for keys would
        // make the audit row say the wrong thing.
        let store = Arc::new(MemoryKvStore::new());
        with_store(store, || {
            let mut harness = Harness::new(state());
            assert_eq!(
                put_call(&mut harness, "cache", &"k".repeat(513), b"v"),
                Err(Refusal::FrameInvalid)
            );
            assert_eq!(
                put_call(&mut harness, "cache", &"k".repeat(512), b"v"),
                Ok(HostReturn::Status(0))
            );
        });
    }

    #[test]
    fn a_value_over_one_mib_is_refused_with_its_own_code() {
        let store = Arc::new(MemoryKvStore::new());
        with_store(store, || {
            let mut harness = Harness::new(state());
            let too_big = vec![0u8; abi::KV_VALUE_MAX_BYTES + 1];
            assert_eq!(
                put_call(&mut harness, "cache", "k", &too_big),
                Err(Refusal::KvValueTooLarge)
            );
            // Exactly at the cap is legal: the limit is inclusive, and an
            // off-by-one here would refuse the largest value the ABI allows.
            let at_cap = vec![0u8; abi::KV_VALUE_MAX_BYTES];
            assert_eq!(
                put_call(&mut harness, "cache", "k", &at_cap),
                Ok(HostReturn::Status(0))
            );
        });
    }

    #[test]
    fn the_quota_refuses_the_write_that_would_cross_it() {
        // 8 bytes of room. The write that fits lands, the one that would cross
        // is refused with the quota code, and the refusal does not damage what
        // is already stored.
        let store = Arc::new(MemoryKvStore::with_limits(8, 100, 500));
        with_store(store.clone(), || {
            let mut harness = Harness::new(state());
            let install = harness.state().install_id;
            assert_eq!(
                put_call(&mut harness, "cache", "a", &[0u8; 8]),
                Ok(HostReturn::Status(0))
            );
            assert_eq!(
                put_call(&mut harness, "cache", "b", &[0u8; 1]),
                Err(Refusal::KvQuotaExceeded)
            );
            assert_eq!(store.usage(install), KvUsage { bytes: 8, keys: 1 });
            assert_eq!(
                payload(get_call(&mut harness, "cache", "a")),
                Ok(vec![0u8; 8]),
                "a refused put leaves the stored value alone"
            );
        });
    }

    #[test]
    fn an_overwrite_moves_the_byte_total_by_the_difference() {
        // The rule that makes a cached entry cheap to refresh: rewriting the
        // same key at the same size costs nothing against the quota.
        let store = Arc::new(MemoryKvStore::with_limits(10, 1, 500));
        with_store(store.clone(), || {
            let mut harness = Harness::new(state());
            let install = harness.state().install_id;
            for _ in 0..5 {
                assert_eq!(
                    put_call(&mut harness, "cache", "k", &[7u8; 10]),
                    Ok(HostReturn::Status(0)),
                    "a same-size overwrite must never cross the quota"
                );
            }
            assert_eq!(store.usage(install), KvUsage { bytes: 10, keys: 1 });
            // Growing past the room it has left is refused.
            assert_eq!(
                put_call(&mut harness, "cache", "k", &[7u8; 11]),
                Err(Refusal::KvQuotaExceeded)
            );
        });
    }

    #[test]
    fn the_key_quota_refuses_the_key_that_would_cross_it() {
        let store = Arc::new(MemoryKvStore::with_limits(KV_BYTES_PER_INSTALL, 2, 500));
        with_store(store, || {
            let mut harness = Harness::new(state());
            assert_eq!(
                put_call(&mut harness, "cache", "a", b"v"),
                Ok(HostReturn::Status(0))
            );
            assert_eq!(
                put_call(&mut harness, "cache", "b", b"v"),
                Ok(HostReturn::Status(0))
            );
            assert_eq!(
                put_call(&mut harness, "cache", "c", b"v"),
                Err(Refusal::KvQuotaExceeded)
            );
            // Rewriting an existing key is not a new key.
            assert_eq!(
                put_call(&mut harness, "cache", "a", b"w"),
                Ok(HostReturn::Status(0))
            );
        });
    }

    #[test]
    fn delete_is_idempotent_and_gives_the_bytes_back() {
        let store = Arc::new(MemoryKvStore::new());
        with_store(store.clone(), || {
            let mut harness = Harness::new(state());
            let install = harness.state().install_id;
            assert_eq!(
                put_call(&mut harness, "cache", "k", b"1234"),
                Ok(HostReturn::Status(0))
            );
            let (ns, ns_len) = harness.put_str("cache");
            let (k, k_len) = harness.put_str("k");
            let args = [ns as i32, ns_len as i32, k as i32, k_len as i32];
            assert_eq!(harness.call("kv_delete", &args), Ok(HostReturn::Status(0)));
            assert_eq!(store.usage(install), KvUsage { bytes: 0, keys: 0 });
            assert_eq!(
                harness.call("kv_delete", &args),
                Ok(HostReturn::Status(0)),
                "deleting an absent key is success"
            );
        });
    }

    #[test]
    fn a_list_page_is_bounded_ordered_and_cursored() {
        let store = Arc::new(MemoryKvStore::with_limits(
            KV_BYTES_PER_INSTALL,
            KV_KEYS_PER_INSTALL,
            2,
        ));
        with_store(store, || {
            let mut harness = Harness::new(state());
            for key in ["a1", "a2", "a3", "b1"] {
                assert_eq!(
                    put_call(&mut harness, "cache", key, b"v"),
                    Ok(HostReturn::Status(0))
                );
            }
            // A key outside the prefix exists and must never appear.
            assert_eq!(
                put_call(&mut harness, "cache", "zz", b"v"),
                Ok(HostReturn::Status(0))
            );

            let mut seen = Vec::new();
            let mut cursor = String::new();
            loop {
                let (ns, ns_len) = harness.put_str("cache");
                let (prefix, prefix_len) = harness.put_str("a");
                let (cur, cur_len) = harness.put_str(&cursor);
                let page = payload(harness.call(
                    "kv_list",
                    &[
                        ns as i32,
                        ns_len as i32,
                        prefix as i32,
                        prefix_len as i32,
                        cur as i32,
                        cur_len as i32,
                    ],
                ))
                .expect("a page");
                let wire: WirePage = serde_json::from_slice(&page).expect("the documented JSON");
                assert!(wire.keys.len() <= 2, "a page never exceeds the cap");
                seen.extend(wire.keys);
                if wire.cursor.is_empty() {
                    break;
                }
                cursor = wire.cursor;
            }
            assert_eq!(seen, vec!["a1", "a2", "a3"]);
        });
    }

    #[test]
    fn a_page_prefix_is_a_prefix_and_not_a_pattern() {
        // `%` and `_` in a prefix must match themselves: a prefix that quietly
        // became a wildcard would read keys outside the prefix the guest asked
        // for.
        let store = Arc::new(MemoryKvStore::with_limits(KV_BYTES_PER_INSTALL, 100, 500));
        with_store(store, || {
            let mut harness = Harness::new(state());
            for key in ["a_b", "axb"] {
                assert_eq!(
                    put_call(&mut harness, "cache", key, b"v"),
                    Ok(HostReturn::Status(0))
                );
            }
            let (ns, ns_len) = harness.put_str("cache");
            let (prefix, prefix_len) = harness.put_str("a_");
            let (cur, cur_len) = harness.put_str("");
            let page = payload(harness.call(
                "kv_list",
                &[
                    ns as i32,
                    ns_len as i32,
                    prefix as i32,
                    prefix_len as i32,
                    cur as i32,
                    cur_len as i32,
                ],
            ))
            .expect("a page");
            let wire: WirePage = serde_json::from_slice(&page).expect("json");
            assert_eq!(wire.keys, vec!["a_b"]);
        });
    }

    #[test]
    fn an_expired_key_is_invisible_before_anything_sweeps_it() {
        let store = Arc::new(MemoryKvStore::new());
        let install = Uuid::now_v7();
        let org = Uuid::now_v7();
        let past = Utc::now() - chrono::Duration::seconds(1);
        store
            .put(org, install, "cache", "gone", b"v", Some(past))
            .expect("the store accepts a past expiry");
        assert_eq!(store.get(org, install, "cache", "gone").expect("get"), None);
        let page = store.list(org, install, "cache", "", None).expect("list");
        assert!(page.keys.is_empty(), "an expired key is not listed");
        // The bytes stay charged until the sweep reclaims them, which is the
        // SQL store's rule as well.
        assert_eq!(store.usage(install), KvUsage { bytes: 1, keys: 1 });
    }

    #[test]
    fn one_install_cannot_see_another_installs_keys() {
        let store = Arc::new(MemoryKvStore::new());
        let org = Uuid::now_v7();
        let mine = Uuid::now_v7();
        let theirs = Uuid::now_v7();
        store
            .put(org, theirs, "cache", "k", b"secret", None)
            .expect("put");
        assert_eq!(store.get(org, mine, "cache", "k").expect("get"), None);
        assert!(store
            .list(org, mine, "cache", "", None)
            .expect("list")
            .keys
            .is_empty());
        assert!(!store.delete(org, mine, "cache", "k").expect("delete"));
        assert_eq!(
            store.get(org, theirs, "cache", "k").expect("get"),
            Some(b"secret".to_vec()),
            "the other install's value is untouched"
        );
    }

    #[test]
    fn without_a_store_every_verb_refuses_with_the_typed_code() {
        // Nothing installed: the process genuinely cannot serve plugin state, so
        // every verb says so instead of inventing an answer. This is also the
        // state `cargo test -p plugin-host` runs in by default, which is why
        // `host_fns::tests` still sees `plugin_not_implemented`.
        assert!(current().is_none());
        let mut harness = Harness::new(state());
        assert_eq!(
            get_call(&mut harness, "cache", "k"),
            Err(Refusal::NotImplemented)
        );
        assert_eq!(
            put_call(&mut harness, "cache", "k", b"v"),
            Err(Refusal::NotImplemented)
        );
        let (ns, ns_len) = harness.put_str("cache");
        let (k, k_len) = harness.put_str("k");
        let args = [ns as i32, ns_len as i32, k as i32, k_len as i32];
        assert_eq!(
            harness.call("kv_delete", &args),
            Err(Refusal::NotImplemented)
        );
        let (prefix, prefix_len) = harness.put_str("");
        let (cur, cur_len) = harness.put_str("");
        assert_eq!(
            harness.call(
                "kv_list",
                &[
                    ns as i32,
                    ns_len as i32,
                    prefix as i32,
                    prefix_len as i32,
                    cur as i32,
                    cur_len as i32,
                ]
            ),
            Err(Refusal::NotImplemented)
        );
    }

    #[test]
    fn a_backend_failure_maps_onto_an_existing_refusal_and_never_invents_a_code() {
        assert_eq!(KvError::QuotaExceeded.refusal(), Refusal::KvQuotaExceeded);
        assert_eq!(KvError::Unavailable.refusal(), Refusal::NotImplemented);
        assert_eq!(
            KvError::Backend("disk on fire".into()).refusal(),
            Refusal::WorkerFailed
        );
    }

    #[test]
    fn the_quota_arithmetic_is_the_adr_boundary() {
        let used = KvUsage { bytes: 0, keys: 0 };
        // Exactly at the cap is admitted; one byte over is not.
        assert_eq!(
            admit(used, None, 10, 10, 1),
            Ok(KvUsage { bytes: 10, keys: 1 })
        );
        assert_eq!(admit(used, None, 11, 10, 1), Err(KvError::QuotaExceeded));
        // The key count is a separate wall.
        assert_eq!(
            admit(KvUsage { bytes: 0, keys: 1 }, None, 1, 1024, 1),
            Err(KvError::QuotaExceeded)
        );
        // An overwrite that shrinks gives room back.
        assert_eq!(
            admit(KvUsage { bytes: 10, keys: 1 }, Some(10), 4, 10, 1),
            Ok(KvUsage { bytes: 4, keys: 1 })
        );
    }

    #[test]
    fn the_two_quota_numbers_are_the_adr_numbers() {
        // Pinned here because they are the only Decision 4 numbers that live
        // outside `abi`: the catalogue deliberately omits them, since a guest
        // can neither read nor act on a per-install quota.
        assert_eq!(KV_KEYS_PER_INSTALL, 100_000);
        assert_eq!(KV_BYTES_PER_INSTALL, 268_435_456);
        assert_eq!(KV_LIST_PAGE, 500);
        assert_eq!(KV_LIST_PAGE, abi::KV_LIST_PAGE as i64);
    }

    #[test]
    fn with_store_restores_the_previous_handle_even_when_it_panics() {
        // A test that fails an assertion must not leave a store installed for
        // whatever runs next on this thread.
        assert!(current().is_none(), "no store to begin with");
        let inner: Arc<dyn KvStore> = Arc::new(MemoryKvStore::with_limits(1, 1, 1));
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            with_store(inner, || {
                assert!(current().is_some(), "the thread store is visible");
                panic!("the closure failed");
            })
        }));
        assert!(caught.is_err());
        assert!(
            current().is_none(),
            "the thread handle was cleared on unwind"
        );
    }
}
