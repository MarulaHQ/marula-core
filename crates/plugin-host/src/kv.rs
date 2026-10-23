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
//! once per process (the worker, or an `in_process` core-api), [`with_store`]
//! sets it for one thread (a test, or a caller that owns the scope), and
//! [`current`] resolves thread-first then process. A worker is single-threaded,
//! so for the shipped isolation profile the two are the same thing.
//!
//! ## Three stores, one seam
//!
//! - [`MemoryKvStore`] — quota, TTL and ordering rules without a database. It is
//!   what `cargo test -p plugin-host` drives the four host entries against, and
//!   it must keep passing with no database and no parent process.
//! - [`BridgeKvStore`] — what BOTH transports install. It holds no connection
//!   and no tenant identity: it hands one call to a
//!   [`HostCallChannel`] and returns the answer, and the parent answers from
//!   its own record of the invocation (`crate::runtime::HostCallService`).
//!   A pool worker's channel writes a frame to stdout and blocks on the reply;
//!   `in_process`'s channel is a message to the task that owns the transaction.
//!   The store is the same in both, which is what ADR 0023 Decision 2 means by
//!   "the transport changes nothing about what a plugin may do".
//! - `core_db::plugin_kv`'s SQL — the parent's side of that bridge
//!   (`crate::exec`), inside the invocation's own transaction.
//!
//! **When no store is installed every verb refuses `plugin_not_implemented`.**
//! That is still a real answer, not a leftover: a bare `PluginRuntime::invoke`
//! passes [`crate::runtime::NoHostCalls`] and a unit test installs nothing, and
//! both are processes with nowhere to put the bytes.
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
use crate::wire;

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
    /// The invocation has spent the parent's host-call allowance
    /// (`crate::runtime::MAX_HOST_CALLS_PER_INVOCATION`). Only
    /// [`BridgeKvStore`] produces it: a store inside the parent answers the
    /// call itself and has no allowance to spend.
    HostCallBudgetExhausted,
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
    /// `plugin_kv_unavailable` code in the next ABI amendment (26, since 25 is
    /// `plugin_credential_refused`), and until then a store implementation
    /// should prefer to answer from whatever it can rather than surface
    /// `Backend`.
    ///
    /// `HostCallBudgetExhausted` derives [`Refusal::FuelExhausted`] for the same
    /// reason: the closed vocabulary has no "host-call allowance" code, and fuel
    /// is the code it already has for an invocation that used up the work it was
    /// given. A dedicated code belongs in the same amendment as the one above.
    pub fn refusal(&self) -> Refusal {
        match self {
            KvError::QuotaExceeded => Refusal::KvQuotaExceeded,
            KvError::Unavailable => Refusal::NotImplemented,
            KvError::HostCallBudgetExhausted => Refusal::FuelExhausted,
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
// The bridge: the worker asks the parent (W53-11)
// ---------------------------------------------------------------------------

/// Re-exported so the KV seam keeps its historical path. The transport, the
/// counter and the correlator check live in [`crate::bridge`], because the HTTP
/// transport shares them: two counters would repeat a `call_id` and the parent
/// would refuse the second call of either kind.
pub use crate::bridge::HostCallChannel;

/// The store a POOL worker installs: it implements [`KvStore`] by asking the
/// parent.
///
/// The worker holds no database handle, no tenant identity and no credential.
/// Every call it makes is one frame on a pipe it already owns, answered by the
/// parent from the parent's own record of the invocation — which is why the
/// frames name only an invocation id and a call number (see `wire`'s header).
///
/// ## Why the invocation is bound per run
///
/// The store is installed once, at worker start (there is one worker per
/// process and its pipes never change). The invocation id is not, so
/// [`BridgeKvStore::begin_invocation`] sets it immediately before
/// `sandbox.run` and [`BridgeKvStore::end_invocation`] clears it afterwards.
/// A call with no bound invocation is [`KvError::Unavailable`] rather than a
/// frame with a guessed id: the parent would refuse it anyway, and inventing
/// one would put a wrong correlator in a log.
pub struct BridgeKvStore {
    client: Arc<crate::bridge::HostCallClient>,
}

impl BridgeKvStore {
    /// From the channel alone, which is how a test builds one.
    pub fn new(channel: Arc<dyn HostCallChannel>) -> Self {
        BridgeKvStore {
            client: crate::bridge::HostCallClient::new(channel),
        }
    }

    /// From the client the process already shares with the HTTP transport.
    /// This is the constructor the worker uses: one counter, both features.
    pub fn with_client(client: Arc<crate::bridge::HostCallClient>) -> Self {
        BridgeKvStore { client }
    }

    /// Binds the store to the invocation about to run.
    pub fn begin_invocation(&self, invocation_id: Uuid) {
        self.client.begin_invocation(invocation_id);
    }

    /// Unbinds the invocation. A host call after this point is unavailable
    /// rather than attributed to the invocation that just finished.
    pub fn end_invocation(&self) {
        self.client.end_invocation();
    }

    /// One frame out, one frame back. The correlators are the shared client's
    /// business; this maps its answer onto the store's own vocabulary.
    fn ask(&self, op: wire::HostCallOp) -> Result<wire::HostCallValue, KvError> {
        match self.client.ask(op) {
            Ok(value) => Ok(value),
            Err(crate::bridge::CallError::Refused(refusal)) => Err(match refusal {
                wire::HostCallRefusal::QuotaExceeded => KvError::QuotaExceeded,
                wire::HostCallRefusal::BudgetExhausted => KvError::HostCallBudgetExhausted,
                wire::HostCallRefusal::Unavailable => KvError::Unavailable,
                wire::HostCallRefusal::Failed => {
                    KvError::Backend("the parent's store failed".to_string())
                }
                // The KV verbs cannot produce the egress or credential
                // refusals; a parent that answers one has a bug, and a bug is
                // a backend failure rather than a store decision.
                other => KvError::Backend(format!("unexpected host refusal {other:?}")),
            }),
            Err(crate::bridge::CallError::NotBound) => Err(KvError::Unavailable),
            Err(crate::bridge::CallError::Transport(detail)) => Err(KvError::Backend(detail)),
        }
    }
}

/// The value a reply must carry for one verb, or the transport error that says
/// the parent answered the wrong shape for the call that was made.
fn shape_mismatch(verb: &str, value: &wire::HostCallValue) -> KvError {
    KvError::Backend(format!(
        "the parent answered {verb} with {}",
        match value {
            wire::HostCallValue::Unit => "unit",
            wire::HostCallValue::Absent => "an absent value",
            wire::HostCallValue::Value(_) => "a value",
            wire::HostCallValue::Existed(_) => "an existence flag",
            wire::HostCallValue::Page { .. } => "a page",
            wire::HostCallValue::Http(_) => "an http response",
        }
    ))
}

impl KvStore for BridgeKvStore {
    fn get(
        &self,
        _org_id: Uuid,
        _install_id: Uuid,
        namespace: &str,
        key: &str,
    ) -> Result<Option<Vec<u8>>, KvError> {
        // The two identities are the CALLER's and stay in this process: the
        // frame has no field for them, and the parent composes them from the
        // invocation it dispatched. That is the whole reason a worker may hold
        // this store and a worker may not hold a connection.
        match self.ask(wire::HostCallOp::KvGet {
            namespace: namespace.to_string(),
            key: key.to_string(),
        })? {
            wire::HostCallValue::Absent => Ok(None),
            wire::HostCallValue::Value(value) => Ok(Some(value)),
            other => Err(shape_mismatch("kv_get", &other)),
        }
    }

    fn put(
        &self,
        _org_id: Uuid,
        _install_id: Uuid,
        namespace: &str,
        key: &str,
        value: &[u8],
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(), KvError> {
        match self.ask(wire::HostCallOp::KvPut {
            namespace: namespace.to_string(),
            key: key.to_string(),
            value: value.to_vec(),
            expires_at,
        })? {
            wire::HostCallValue::Unit => Ok(()),
            other => Err(shape_mismatch("kv_put", &other)),
        }
    }

    fn delete(
        &self,
        _org_id: Uuid,
        _install_id: Uuid,
        namespace: &str,
        key: &str,
    ) -> Result<bool, KvError> {
        match self.ask(wire::HostCallOp::KvDelete {
            namespace: namespace.to_string(),
            key: key.to_string(),
        })? {
            wire::HostCallValue::Existed(existed) => Ok(existed),
            other => Err(shape_mismatch("kv_delete", &other)),
        }
    }

    fn list(
        &self,
        _org_id: Uuid,
        _install_id: Uuid,
        namespace: &str,
        prefix: &str,
        cursor: Option<&str>,
    ) -> Result<KvKeyPage, KvError> {
        match self.ask(wire::HostCallOp::KvList {
            namespace: namespace.to_string(),
            prefix: prefix.to_string(),
            cursor: cursor.map(str::to_string),
        })? {
            wire::HostCallValue::Page { keys, cursor } => Ok(KvKeyPage {
                keys,
                next_cursor: cursor,
            }),
            other => Err(shape_mismatch("kv_list", &other)),
        }
    }
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

    // -----------------------------------------------------------------------
    // The bridge, driven by a channel a test controls
    // -----------------------------------------------------------------------

    type Answer =
        Box<dyn Fn(&wire::HostCallFrame) -> Result<wire::HostReplyFrame, String> + Send + Sync>;

    struct CannedChannel {
        calls: Mutex<Vec<wire::HostCallFrame>>,
        answer: Answer,
    }

    impl CannedChannel {
        fn new(
            answer: impl Fn(&wire::HostCallFrame) -> Result<wire::HostReplyFrame, String>
                + Send
                + Sync
                + 'static,
        ) -> Arc<Self> {
            Arc::new(CannedChannel {
                calls: Mutex::new(Vec::new()),
                answer: Box::new(answer),
            })
        }

        fn calls(&self) -> Vec<wire::HostCallFrame> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    impl HostCallChannel for CannedChannel {
        fn exchange(&self, frame: &wire::HostCallFrame) -> Result<wire::HostReplyFrame, String> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(frame.clone());
            (self.answer)(frame)
        }
    }

    /// A channel that answers every call with `outcome`, echoing both
    /// correlators the way the parent does.
    fn replying(
        outcome: impl Fn(&wire::HostCallFrame) -> wire::HostCallOutcome + Send + Sync + 'static,
    ) -> Arc<CannedChannel> {
        CannedChannel::new(move |frame| {
            Ok(wire::HostReplyFrame {
                invocation_id: frame.invocation_id,
                call_id: frame.call_id,
                outcome: outcome(frame),
            })
        })
    }

    /// A store bound to the harness's invocation, and the channel behind it.
    fn bridged(invocation_id: Uuid, channel: Arc<CannedChannel>) -> Arc<BridgeKvStore> {
        let store = Arc::new(BridgeKvStore::new(channel));
        store.begin_invocation(invocation_id);
        store
    }

    #[test]
    fn a_bridge_names_only_the_invocation_and_the_call_it_is_making() {
        let channel = replying(|frame| match &frame.op {
            wire::HostCallOp::KvGet { .. } => wire::HostCallValue::Value(b"stored".to_vec()).into(),
            wire::HostCallOp::KvPut { value, .. } => {
                assert_eq!(value, b"new value");
                wire::HostCallValue::Unit.into()
            }
            wire::HostCallOp::KvDelete { .. } => wire::HostCallValue::Existed(true).into(),
            // This canned parent answers KV only; a KV unit test never sends
            // this verb, and `Unavailable` is what a parent with no transport
            // would say.
            wire::HostCallOp::HttpRequest { .. } => {
                wire::HostCallOutcome::Refused(wire::HostCallRefusal::Unavailable)
            }
            wire::HostCallOp::KvList { cursor, .. } => wire::HostCallValue::Page {
                keys: vec!["a1".into(), "a2".into()],
                cursor: cursor.as_ref().map(|_| "a3".to_string()),
            }
            .into(),
        });
        let store = bridged(Uuid::now_v7(), Arc::clone(&channel));
        let invocation_id = store
            .client
            .bound_invocation()
            .expect("the test bound an invocation");
        with_store(store, || {
            let mut harness = Harness::new(state());
            let (ns, ns_len) = harness.put_str("cache");
            let (k, k_len) = harness.put_str("k");
            let args = [ns as i32, ns_len as i32, k as i32, k_len as i32];

            assert_eq!(
                payload(harness.call("kv_get", &args)),
                Ok(b"stored".to_vec())
            );
            assert_eq!(
                put_call(&mut harness, "cache", "k", b"new value"),
                Ok(HostReturn::Status(0))
            );
            assert_eq!(harness.call("kv_delete", &args), Ok(HostReturn::Status(0)));
            let (prefix, prefix_len) = harness.put_str("a");
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
            assert!(String::from_utf8_lossy(&page).contains("a1"));
        });

        let calls = channel.calls();
        assert_eq!(calls.len(), 4, "one frame per verb");
        for (index, frame) in calls.iter().enumerate() {
            assert_eq!(
                frame.invocation_id, invocation_id,
                "every frame names the invocation the worker is running"
            );
            assert_eq!(
                frame.call_id, index as u64,
                "the correlator is the worker's own count, in order"
            );
        }
        assert_eq!(calls[0].op.verb(), wire::HostVerb::KvGet);
        assert_eq!(calls[3].op.verb(), wire::HostVerb::KvList);
    }

    #[test]
    fn every_parent_refusal_maps_onto_an_existing_abi_code_and_never_invents_one() {
        for (refusal, expected) in [
            (
                wire::HostCallRefusal::QuotaExceeded,
                Refusal::KvQuotaExceeded,
            ),
            (
                wire::HostCallRefusal::BudgetExhausted,
                Refusal::FuelExhausted,
            ),
            (wire::HostCallRefusal::Unavailable, Refusal::NotImplemented),
            (wire::HostCallRefusal::Failed, Refusal::WorkerFailed),
        ] {
            let channel = replying(move |_| wire::HostCallOutcome::Refused(refusal));
            let store = bridged(Uuid::now_v7(), channel);
            with_store(store, || {
                let mut harness = Harness::new(state());
                assert_eq!(
                    get_call(&mut harness, "cache", "k"),
                    Err(expected),
                    "{refusal:?} must map onto a code the ABI already has"
                );
            });
        }
    }

    #[test]
    fn a_reply_to_another_call_is_a_transport_failure_not_an_answer() {
        // The parent echoes both correlators. A reply that answers a different
        // call must never reach the guest as if it answered this one.
        let channel = CannedChannel::new(|frame| {
            Ok(wire::HostReplyFrame {
                invocation_id: frame.invocation_id,
                call_id: frame.call_id + 1,
                outcome: wire::HostCallOutcome::Ok(wire::HostCallValue::Value(b"x".to_vec())),
            })
        });
        let store = bridged(Uuid::now_v7(), channel);
        with_store(store, || {
            let mut harness = Harness::new(state());
            assert_eq!(
                get_call(&mut harness, "cache", "k"),
                Err(Refusal::WorkerFailed)
            );
        });
    }

    #[test]
    fn a_reply_of_the_wrong_shape_for_the_verb_is_a_transport_failure() {
        // A page where a value belongs would otherwise be handed to the guest
        // as a `kv_get` answer.
        let channel = replying(|_| {
            wire::HostCallValue::Page {
                keys: vec!["a".into()],
                cursor: None,
            }
            .into()
        });
        let store = bridged(Uuid::now_v7(), channel);
        with_store(store, || {
            let mut harness = Harness::new(state());
            assert_eq!(
                get_call(&mut harness, "cache", "k"),
                Err(Refusal::WorkerFailed)
            );
        });
    }

    #[test]
    fn a_bridge_with_no_invocation_bound_is_unavailable_without_writing_a_frame() {
        let channel = replying(|_| wire::HostCallValue::Unit.into());
        let store: Arc<dyn KvStore> = Arc::new(BridgeKvStore::new(
            Arc::clone(&channel) as Arc<dyn HostCallChannel>
        ));
        with_store(store, || {
            let mut harness = Harness::new(state());
            assert_eq!(
                get_call(&mut harness, "cache", "k"),
                Err(Refusal::NotImplemented)
            );
            assert_eq!(
                put_call(&mut harness, "cache", "k", b"v"),
                Err(Refusal::NotImplemented)
            );
        });
        assert!(
            channel.calls().is_empty(),
            "an unbound bridge must not invent an invocation id"
        );
    }

    #[test]
    fn the_call_counter_restarts_for_each_invocation() {
        let channel = replying(|_| wire::HostCallValue::Absent.into());
        let store = bridged(Uuid::now_v7(), Arc::clone(&channel));
        with_store(store.clone(), || {
            let mut harness = Harness::new(state());
            let _ = get_call(&mut harness, "cache", "a");
            let _ = get_call(&mut harness, "cache", "b");
        });
        store.begin_invocation(Uuid::now_v7());
        with_store(store, || {
            let mut harness = Harness::new(state());
            let _ = get_call(&mut harness, "cache", "c");
        });
        let ids: Vec<u64> = channel.calls().iter().map(|frame| frame.call_id).collect();
        assert_eq!(ids, vec![0, 1, 0], "the parent counts per invocation");
    }
}
