//! In-process reference cache for low-change, org-scoped reads.
//!
//! Why this exists, in the shape `state.rs:25` uses for the second pool:
//! `crates/core-api/src/state.rs:1-5` states that core-api never publishes to
//! Redis directly, and that rule is about *publishing events* — the outbox is
//! the only writer. It has been read as though it also bans caching, so
//! reference data with a low change rate was re-read per request. The principal
//! alone cost two indexed queries on every authenticated request
//! (`crates/core-api/src/auth/middleware.rs:192`). This module is the cache that
//! position said was missing, and it is deliberately in-process: the
//! invalidation channel it needs (the `marula_events` listener) is already
//! running inside this process, so a cache here adds no network hop and no new
//! infrastructure.
//!
//! The invalidation contract, which callers must honour:
//!
//! 1. A cached value must be invalidatable by a contract event, or it does not
//!    belong here. `crate::cache_invalidation` maps event types to keys.
//! 2. The TTL is a backstop for a missed invalidation, never the mechanism.
//! 3. A value whose shape depends on the *caller's* permissions is not cacheable
//!    under a per-org key. Those reads stay uncached.
//!
//! Shape: a process-wide singleton per namespace, reached by function call,
//! matching `realtime::hub()` and `realtime::presence()`. It is not a field on
//! `AppState` because that struct has 151 construction sites, nearly all of them
//! test rigs, and none of them would gain anything from carrying it.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Default TTL when config has not initialised the cache (tests, direct use).
pub const DEFAULT_TTL_SECS: u64 = 30;
/// Default ceiling on entries per namespace.
pub const DEFAULT_MAX_ENTRIES: usize = 4096;

/// Monotonic tick used for LRU ordering. Process-wide and starts at zero; only
/// comparisons matter.
static TICK: AtomicU64 = AtomicU64::new(1);

fn tick() -> u64 {
    TICK.fetch_add(1, Ordering::Relaxed)
}

struct Entry<V> {
    value: V,
    inserted: Instant,
    /// Last read tick, so eviction can drop the least recently used entry
    /// rather than the oldest insert. Kept atomic because reads hold only the
    /// read lock and must still record the touch.
    last_used: AtomicU64,
}

/// A bounded, TTL'd map. `V` is cloned on read, which for these value types is
/// cheaper than the query it replaces.
pub struct RefCache<K, V> {
    entries: RwLock<HashMap<K, Entry<V>>>,
    ttl: Duration,
    capacity: usize,
}

impl<K, V> RefCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            ttl,
            capacity: capacity.max(1),
        }
    }

    /// A fresh value, or `None` when absent or past its TTL. A hit records the
    /// access so eviction sees it as recently used.
    pub fn get(&self, key: &K) -> Option<V> {
        let guard = self.entries.read().ok()?;
        let entry = guard.get(key)?;
        if entry.inserted.elapsed() >= self.ttl {
            return None;
        }
        entry.last_used.store(tick(), Ordering::Relaxed);
        Some(entry.value.clone())
    }

    pub fn insert(&self, key: K, value: V) {
        let Ok(mut guard) = self.entries.write() else {
            return;
        };
        if guard.len() >= self.capacity && !guard.contains_key(&key) {
            if let Some(victim) = guard
                .iter()
                .min_by_key(|(_, e)| e.last_used.load(Ordering::Relaxed))
                .map(|(k, _)| k.clone())
            {
                guard.remove(&victim);
            }
        }
        guard.insert(
            key,
            Entry {
                value,
                inserted: Instant::now(),
                last_used: AtomicU64::new(tick()),
            },
        );
    }

    pub fn invalidate(&self, key: &K) {
        if let Ok(mut guard) = self.entries.write() {
            guard.remove(key);
        }
    }

    /// Drops every entry whose key matches. This is how an org-scoped flush
    /// works: a `role.*` event names a role, not its holders.
    pub fn invalidate_where(&self, predicate: impl Fn(&K) -> bool) {
        if let Ok(mut guard) = self.entries.write() {
            guard.retain(|k, _| !predicate(k));
        }
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.entries.write() {
            guard.clear();
        }
    }

    /// Drops expired entries. The TTL check on read already hides them; this
    /// keeps an idle process from holding memory it will never serve.
    pub fn sweep(&self) {
        if let Ok(mut guard) = self.entries.write() {
            let ttl = self.ttl;
            guard.retain(|_, e| e.inserted.elapsed() < ttl);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.read().map(|g| g.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }
}

/// The namespaces this process caches. Each is keyed at the narrowest scope
/// that makes the value caller-independent.
pub struct Caches {
    /// `(org_id, user_id)` → the resolved principal.
    pub principal: RefCache<(Uuid, Uuid), core_domain::entities::Principal>,
    /// Generic org-scoped reference values, behind one type-erased map.
    ///
    /// Keys are `"<namespace>|<org>"` or `"<namespace>|<org>|<discriminator>"`
    /// (see [`Caches::ref_key`]), so invalidating one family for one org is a
    /// prefix match and every family shares one bounded map instead of each
    /// growing its own.
    refs: RefCache<String, Arc<dyn std::any::Any + Send + Sync>>,
    /// Whether callers should consult the cache at all.
    ///
    /// True only after [`init`], which boot calls. An uninitialised process —
    /// every test binary, and anything that constructs state by hand — reads
    /// straight through, so a process that never starts the `marula_events`
    /// listener can never serve a value it has no way to invalidate. That is
    /// the rule in this module's header, enforced mechanically rather than by
    /// convention.
    ///
    /// Flippable rather than fixed at construction, because the singleton is
    /// reachable from both an explicit boot call and the lazy accessor, and
    /// whichever runs first must not decide the question for the other.
    enabled: std::sync::atomic::AtomicBool,
}

impl Caches {
    fn new(ttl: Duration, capacity: usize, enabled: bool) -> Self {
        Self {
            principal: RefCache::new(ttl, capacity),
            refs: RefCache::new(ttl, capacity),
            enabled: std::sync::atomic::AtomicBool::new(enabled),
        }
    }

    /// Whether reads may be served from the cache. False until [`init`].
    pub fn enabled(&self) -> bool {
        self.enabled.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Namespace prefixes, one per cached family. Invalidation matches on these.
    pub const NS_ORG_SETTINGS: &'static str = "org-settings";
    pub const NS_BOARD: &'static str = "board";
    pub const NS_CUSTOM_FIELD: &'static str = "custom-field";
    pub const NS_SLA: &'static str = "sla";
    pub const NS_AI: &'static str = "ai";

    /// `"<namespace>|<org>"`.
    pub fn ref_key(namespace: &str, org_id: Uuid) -> String {
        format!("{namespace}|{org_id}")
    }

    /// `"<namespace>|<org>|<discriminator>"`, for values scoped finer than the
    /// org (a board id, a scope enum).
    pub fn ref_key_scoped(namespace: &str, org_id: Uuid, discriminator: &str) -> String {
        format!("{namespace}|{org_id}|{discriminator}")
    }

    /// A cached reference value, or `None` when absent, expired, or the cache
    /// is off. A type mismatch is also a miss rather than a panic: the
    /// namespace string is what identifies the type, and a collision there is a
    /// programming error the miss surfaces instead of a wrong value.
    pub fn get_ref<T: Clone + Send + Sync + 'static>(&self, key: &str) -> Option<T> {
        if !self.enabled() {
            return None;
        }
        let any = self.refs.get(&key.to_string())?;
        any.downcast_ref::<T>().cloned()
    }

    /// Caches one reference value. A no-op when the cache is off.
    pub fn put_ref<T: Clone + Send + Sync + 'static>(&self, key: String, value: T) {
        if !self.enabled() {
            return;
        }
        self.refs.insert(key, Arc::new(value));
    }

    /// Drops every reference entry whose key starts with `prefix`. Callers pass
    /// [`Caches::ref_key`]'s output, so one family for one org clears in one
    /// pass.
    pub fn invalidate_refs(&self, prefix: &str) {
        self.refs.invalidate_where(|k| k.starts_with(prefix));
    }

    /// Drops every reference entry, all families and all orgs.
    pub fn clear_refs(&self) {
        self.refs.clear();
    }

    /// Drops every cached principal in one org. Used when an event names
    /// something whose holders are not on the frame.
    pub fn invalidate_org(&self, org_id: Uuid) {
        self.principal.invalidate_where(|(org, _)| *org == org_id);
    }

    pub fn sweep(&self) {
        self.principal.sweep();
        self.refs.sweep();
    }
}

static CACHES: OnceLock<Arc<Caches>> = OnceLock::new();

/// Builds the process-wide cache from config and turns it on. Called once at
/// boot; a later call is ignored, because the first initialisation is the one
/// the listener and the middleware already hold a reference to.
pub fn init(ttl_secs: u64, max_entries: usize) -> Arc<Caches> {
    CACHES
        .get_or_init(|| {
            Arc::new(Caches::new(
                Duration::from_secs(ttl_secs.max(1)),
                max_entries.max(1),
                true,
            ))
        })
        .clone()
}

/// The process-wide cache. Built with the defaults and left DISABLED when boot
/// has not initialised it, which is the path unit tests and direct callers
/// take: without a listener there is nothing to invalidate with.
pub fn caches() -> Arc<Caches> {
    CACHES
        .get_or_init(|| {
            Arc::new(Caches::new(
                Duration::from_secs(DEFAULT_TTL_SECS),
                DEFAULT_MAX_ENTRIES,
                false,
            ))
        })
        .clone()
}

#[cfg(test)]
pub fn __reset_for_tests() {
    // Both maps: a test that reuses a key across cases must not see the
    // previous case's value, whichever namespace it wrote to.
    caches().principal.clear();
    caches().clear_refs();
}

/// Turns the cache on for a test binary that needs the caching path exercised.
/// Flips the flag on the singleton rather than re-running [`init`], so the call
/// order between this and the lazy accessor cannot decide the outcome.
#[cfg(test)]
pub fn __enable_for_tests() -> Arc<Caches> {
    let caches = caches();
    caches
        .enabled
        .store(true, std::sync::atomic::Ordering::Relaxed);
    caches
}

/// Serializes every unit test that drives the process-wide cache.
///
/// The cache is global on purpose — a per-request cache is not a cache — but
/// that means tests in different modules share one singleton, and a reset in
/// one module wipes a key another module just inserted. A `static` per module
/// serializes that module against itself and nothing else, which is exactly
/// the race this replaces: `boards` and `sla_admin` asserted a hit while
/// `settings` reset the map. This is the single lock they all take.
#[cfg(test)]
pub fn only_one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Drops expired entries on a timer so an idle process does not hold memory it
/// will never serve. The read path already hides expired entries, so this is
/// housekeeping rather than correctness, and a slow tick is fine.
pub async fn sweep_loop(caches: Arc<Caches>) {
    let period = caches.principal.ttl();
    let mut ticker = tokio::time::interval(period.max(Duration::from_secs(1)));
    loop {
        ticker.tick().await;
        caches.sweep();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn short_cache() -> RefCache<i32, String> {
        RefCache::new(Duration::from_millis(40), 4)
    }

    #[test]
    fn a_value_survives_a_read_then_expires() {
        let c: RefCache<i32, String> = RefCache::new(Duration::from_millis(40), 4);
        c.insert(1, "one".into());
        assert_eq!(c.get(&1).as_deref(), Some("one"));
        std::thread::sleep(Duration::from_millis(60));
        assert!(c.get(&1).is_none(), "a past-TTL entry must never be served");
    }

    #[test]
    fn a_miss_returns_none() {
        let c = short_cache();
        assert!(c.get(&7).is_none());
    }

    #[test]
    fn invalidation_drops_exactly_one_key() {
        let c = short_cache();
        c.insert(1, "one".into());
        c.insert(2, "two".into());
        c.invalidate(&1);
        assert!(c.get(&1).is_none());
        assert_eq!(c.get(&2).as_deref(), Some("two"));
    }

    #[test]
    fn invalidate_where_is_an_org_scoped_flush() {
        let c: RefCache<i32, String> = RefCache::new(Duration::from_secs(30), 8);
        for k in [10, 11, 20] {
            c.insert(k, format!("v{k}"));
        }
        c.invalidate_where(|k| *k < 20);
        assert!(c.get(&10).is_none() && c.get(&11).is_none());
        assert!(c.get(&20).is_some());
    }

    #[test]
    fn capacity_evicts_the_least_recently_used() {
        let c: RefCache<i32, String> = RefCache::new(Duration::from_secs(30), 2);
        c.insert(1, "one".into());
        c.insert(2, "two".into());
        // Touch 1 so 2 is the least recently used, then overflow.
        assert!(c.get(&1).is_some());
        c.insert(3, "three".into());
        assert_eq!(c.len(), 2, "the ceiling holds");
        assert_eq!(c.get(&1).as_deref(), Some("one"), "the touched entry stays");
        assert!(c.get(&2).is_none(), "the untouched entry was evicted");
        assert!(c.get(&3).is_some());
    }

    #[test]
    fn reinserting_a_live_key_does_not_evict() {
        let c: RefCache<i32, String> = RefCache::new(Duration::from_secs(30), 1);
        c.insert(1, "one".into());
        c.insert(1, "one-again".into());
        assert_eq!(c.get(&1).as_deref(), Some("one-again"));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn sweep_drops_expired_entries() {
        let c: RefCache<i32, String> = RefCache::new(Duration::from_millis(20), 8);
        c.insert(1, "one".into());
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(c.len(), 1, "hidden by the TTL but still resident");
        c.sweep();
        assert!(c.is_empty());
    }

    #[test]
    fn a_shorter_ttl_is_honoured() {
        let c: RefCache<i32, String> = RefCache::new(Duration::from_millis(1), 4);
        c.insert(1, "one".into());
        std::thread::sleep(Duration::from_millis(10));
        assert!(c.get(&1).is_none());
    }
}
