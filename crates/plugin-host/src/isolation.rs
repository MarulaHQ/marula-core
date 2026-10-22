//! The per-invocation sandbox: fuel, wall clock, memory, output, and what a
//! runaway costs (ADR 0023 Decisions 1 and 4).
//!
//! This module is the only place a guest module is compiled and called. It owns
//! the four limits that bound one invocation and nothing else:
//!
//! | Limit | Mechanism | Refusal |
//! |---|---|---|
//! | fuel | `Store::set_fuel` from [`abi::FUEL_PER_INVOCATION`] | [`Refusal::FuelExhausted`] |
//! | wall clock | epoch deadline, [`abi::EPOCH_TICK_MS`] ticks | [`Refusal::WallClockExceeded`] |
//! | linear memory | `StoreLimits`, trap on a failed grow | [`Refusal::GuestTrap`] |
//! | output | `marula_entry`'s packed length, checked before it is read | [`Refusal::OutputTooLarge`] |
//!
//! ## What it refuses before it runs anything
//!
//! A module is compiled once per digest and checked against the ABI before it
//! is cached: every import must be in the `marula` namespace with the signature
//! the registry declares, and the three exports plus `memory` must exist with
//! the types [`crate::abi`] fixes. A module that does not match is
//! [`LoadError::Rejected`] — refused whole, never repaired — and the caller
//! turns that into [`Refusal::FrameInvalid`]. There is no WASI: the linker
//! defines the `marula` namespace and nothing else, so an import of anything
//! else fails at instantiation even if a future engine change stopped reporting
//! it at load time.
//!
//! ## Why a fresh `Store` and `Instance` per invocation
//!
//! A store is the whole of a guest's mutable state. Creating one per invocation
//! means a guest cannot carry linear memory, a table entry or a global from one
//! tenant's call into the next, and every limit is exact instead of
//! cumulative: the fuel budget is not shared, the 64 MiB ceiling is per call,
//! and an epoch deadline set for one call cannot leak into another. The cost is
//! the ADR's measured 4.38 µs instantiation, which is inside the pool's
//! 9.38 µs total.
//!
//! ## What this module does NOT do
//!
//! It does not authorize anything. [`InvocationState::from_request`] is built
//! from a frame the parent already decided, and every host call runs the
//! registry's own gate against that state. A capability the parent refused
//! cannot appear in the worker, because nothing here can add one.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use wasmtime::{
    Caller, Config, Engine, ExternType, FuncType, Linker, Memory, Module, Store, StoreLimits,
    StoreLimitsBuilder, Trap, ValType,
};

use crate::abi::{self, InvocationRequest, Refusal};
use crate::host_fns::{
    AbiParam, AbiReturn, GuestMemory, HostReturn, InvocationState, LogLine, Registry,
};

/// The guest's allocation export: the host allocates the input and every
/// data-returning host reply through it.
pub const ALLOC_EXPORT: &str = "marula_alloc";
/// The guest's entry point: one call per invocation.
pub const ENTRY_EXPORT: &str = "marula_entry";
/// The guest's release export.
pub const FREE_EXPORT: &str = "marula_free";
/// The guest's exported linear memory. Without it the host cannot hand the
/// guest an argument at all, so a module that omits it is rejected.
pub const MEMORY_EXPORT: &str = "memory";

/// Compiled modules one worker keeps. An install pins one digest, so this is
/// quiet in practice; the cap is what stops a churning digest from growing the
/// worker without bound (ADR 0023 Decision 4's resident ceiling).
pub const MAX_CACHED_MODULES: usize = 32;

/// The per-invocation limits, as numbers. [`Default`] is ADR 0023 Decision 4 and
/// is what ships; a test or a benchmark may narrow one, and nothing else may.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SandboxLimits {
    /// Fuel one invocation may consume.
    pub fuel: u64,
    /// Wall clock one invocation may run for, enforced by the epoch ticker.
    pub wall_clock_ms: u64,
    /// How often the ticker increments the engine epoch.
    pub epoch_tick_ms: u64,
    /// Linear memory one store may reach.
    pub instance_memory_bytes: usize,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        SandboxLimits {
            fuel: abi::FUEL_PER_INVOCATION,
            wall_clock_ms: abi::WALL_CLOCK_MS,
            epoch_tick_ms: abi::EPOCH_TICK_MS,
            instance_memory_bytes: abi::INSTANCE_MEMORY_MAX_BYTES as usize,
        }
    }
}

impl SandboxLimits {
    /// Epoch ticks from now until the deadline. At least one, so a limit below
    /// one tick still stops the invocation instead of disabling the deadline.
    fn epoch_ticks(&self) -> u64 {
        self.wall_clock_ms
            .div_ceil(self.epoch_tick_ms.max(1))
            .max(1)
    }
}

/// One signed artifact, as the distribution layer handed it over.
///
/// `digest` is the sha256 hex the artifact was admitted under. The worker
/// recomputes it from `bytes` on every load: a worker that runs bytes the
/// parent did not name would be a second, weaker admission path, and this check
/// is what keeps the parent's pin meaningful.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleArtifact {
    pub digest: String,
    pub bytes: Vec<u8>,
}

impl ModuleArtifact {
    /// The sha256 hex of `bytes` — the same digest `distribution::sha256_hex`
    /// computes at admission.
    pub fn digest_of(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    /// A digest that cannot match any artifact.
    pub fn mismatched_digest() -> String {
        "0".repeat(64)
    }
}

/// Why a module was not loaded. Both variants are the parent's problem: the
/// worker answers [`Refusal::FrameInvalid`] and logs the detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// The bytes are not what the digest names.
    DigestMismatch { declared: String, actual: String },
    /// The bytes are not a core module, or its ABI surface is not the one
    /// ADR 0023 fixes. The message is for a log line, never for an audit row.
    Rejected(String),
    /// The caller asked for a digest the sandbox does not hold and supplied no
    /// bytes for it. The pool never does this — a fresh member has an empty
    /// cache and its parent sends the bytes — so it is a bug, not a condition.
    NotCached(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::DigestMismatch { declared, actual } => {
                write!(f, "artifact digest {declared} but bytes hash to {actual}")
            }
            LoadError::Rejected(reason) => write!(f, "module rejected: {reason}"),
            LoadError::NotCached(digest) => write!(f, "digest {digest} is not cached"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Why the sandbox itself could not be built. A deployment error: the process
/// refuses to serve plugins rather than running them without a limit.
#[derive(Debug)]
pub struct SandboxError(String);

impl std::fmt::Display for SandboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SandboxError {}

/// What a completed invocation produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestOutput {
    /// The payload `marula_entry` returned. `None` when it returned no bytes.
    pub output: Option<Vec<u8>>,
    /// Fuel consumed, exact and deterministic for the same input.
    pub fuel_used: u64,
    /// Lines the guest wrote through `log`, oldest first. The worker forwards
    /// them to its own log stream; they never travel in a reply frame.
    pub logs: Vec<LogLine>,
}

/// A refused invocation. `fuel_used` is always present: the store survives its
/// own trap, so the parent can write the exact figure into the audit row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestFailure {
    pub refusal: Refusal,
    pub fuel_used: u64,
    pub logs: Vec<LogLine>,
}

/// One engine, one linker, one compiled-module cache, one epoch ticker.
///
/// A worker process builds exactly one of these. `in_process` builds one per
/// pool. Both run the same code: only the transport differs.
pub struct Sandbox {
    engine: Engine,
    linker: Linker<StoreData>,
    registry: Arc<Registry>,
    limits: SandboxLimits,
    cache: Mutex<Cache>,
    /// Held, never read: the ticker runs while it exists and stops when the
    /// sandbox drops. Its `Drop` is the whole API.
    _ticker: EpochTicker,
}

impl std::fmt::Debug for Sandbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sandbox")
            .field("limits", &self.limits)
            .field("cached_modules", &self.cached_count())
            .finish()
    }
}

struct Cache {
    entries: Vec<CacheEntry>,
    clock: u64,
}

struct CacheEntry {
    digest: String,
    module: Arc<Module>,
    used: u64,
}

/// The state a store carries: what a host call may read, and the memory limits
/// the store enforces. One per invocation, discarded with it.
struct StoreData {
    invocation: InvocationState,
    limits: StoreLimits,
    /// The fuel this invocation started with, so the used figure is a
    /// subtraction rather than a second copy of the constant.
    fuel_budget: u64,
}

impl Sandbox {
    /// Builds the sandbox with ADR 0023 Decision 4's limits.
    pub fn new(registry: Arc<Registry>) -> Result<Self, SandboxError> {
        Sandbox::with_limits(registry, SandboxLimits::default())
    }

    /// Builds the sandbox with narrowed limits. The pool and the worker always
    /// pass [`SandboxLimits::default`]; only tests narrow one.
    pub fn with_limits(
        registry: Arc<Registry>,
        limits: SandboxLimits,
    ) -> Result<Self, SandboxError> {
        let mut config = Config::new();
        // Fuel is the exact limit and the epoch is the hard stop that does not
        // depend on the guest reaching it (Decision 4).
        config.consume_fuel(true);
        config.epoch_interruption(true);
        // A guest has no threads, no shared memory and no components in v1.
        // Setting them off rather than relying on defaults means a module that
        // needs one is refused by the engine, not silently given it.
        config.wasm_threads(false);
        config.wasm_component_model(false);
        let engine = Engine::new(&config)
            .map_err(|error| SandboxError(format!("engine configuration: {error}")))?;

        let linker = build_linker(&engine, Arc::clone(&registry))
            .map_err(|error| SandboxError(format!("linker: {error}")))?;

        let ticker = EpochTicker::start(engine.clone(), limits.epoch_tick_ms)?;

        Ok(Sandbox {
            _ticker: ticker,
            engine,
            linker,
            registry,
            limits,
            cache: Mutex::new(Cache {
                entries: Vec::new(),
                clock: 0,
            }),
        })
    }

    /// The limits this sandbox enforces.
    pub fn limits(&self) -> SandboxLimits {
        self.limits
    }

    /// Loads an artifact: verifies its digest, compiles it, checks its ABI
    /// surface, and caches it by digest. A cache hit still verifies the digest
    /// of the bytes supplied, so two different bodies can never share a name.
    pub fn load(&self, artifact: &ModuleArtifact) -> Result<Arc<Module>, LoadError> {
        self.load_with_eviction(artifact).map(|(module, _)| module)
    }

    /// [`Sandbox::load`] plus the digest this load pushed out of the cache, if
    /// any. The worker reports it back so the parent stops believing the evicted
    /// module is still resident: a parent that guessed would refuse the next
    /// honest invocation for that digest.
    pub fn load_with_eviction(
        &self,
        artifact: &ModuleArtifact,
    ) -> Result<(Arc<Module>, Option<String>), LoadError> {
        let actual = ModuleArtifact::digest_of(&artifact.bytes);
        if actual != artifact.digest {
            return Err(LoadError::DigestMismatch {
                declared: artifact.digest.clone(),
                actual,
            });
        }
        if let Some(module) = self.get(&artifact.digest) {
            return Ok((module, None));
        }

        let module = Module::new(&self.engine, &artifact.bytes)
            .map_err(|error| LoadError::Rejected(format!("compile: {error}")))?;
        verify_module(&module, &self.registry)?;
        Ok(self.cache_module(artifact.digest.clone(), module))
    }

    /// The compiled module for `digest`, if this sandbox holds it.
    pub fn get(&self, digest: &str) -> Option<Arc<Module>> {
        let mut cache = self.lock_cache();
        cache.clock += 1;
        let clock = cache.clock;
        let entry = cache
            .entries
            .iter_mut()
            .find(|entry| entry.digest == digest)?;
        entry.used = clock;
        Some(Arc::clone(&entry.module))
    }

    /// Whether `digest` is cached. The worker reports this back in its reply so
    /// the parent knows when it may stop sending the bytes.
    pub fn contains(&self, digest: &str) -> bool {
        self.lock_cache()
            .entries
            .iter()
            .any(|entry| entry.digest == digest)
    }

    pub fn cached_count(&self) -> usize {
        self.lock_cache().entries.len()
    }

    fn lock_cache(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn cache_module(&self, digest: String, module: Module) -> (Arc<Module>, Option<String>) {
        let module = Arc::new(module);
        let mut cache = self.lock_cache();
        cache.clock += 1;
        let used = cache.clock;
        let mut evicted = None;
        if cache.entries.len() >= MAX_CACHED_MODULES {
            if let Some((index, _)) = cache
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.used)
            {
                evicted = Some(cache.entries.remove(index).digest);
            }
        }
        cache.entries.push(CacheEntry {
            digest,
            module: Arc::clone(&module),
            used,
        });
        (module, evicted)
    }

    /// Runs one authorized invocation. The request is the parent's resolved
    /// frame; this function refuses it whole if it does not validate, and never
    /// adds a grant.
    pub fn run(
        &self,
        module: &Module,
        request: &InvocationRequest,
    ) -> Result<GuestOutput, GuestFailure> {
        let invocation = match InvocationState::from_request(request) {
            Ok(state) => state,
            Err(refusal) => {
                return Err(GuestFailure {
                    refusal,
                    fuel_used: 0,
                    logs: Vec::new(),
                })
            }
        };

        let mut store = Store::new(&self.engine, StoreData::new(invocation, self.limits));
        store.limiter(|data| &mut data.limits);
        // Both of these are configuration invariants, not guest outcomes: the
        // engine has fuel metering and epoch interruption on, so a failure here
        // is a bug in this module and is reported as a trap rather than a panic.
        if store.set_fuel(self.limits.fuel).is_err() {
            return Err(GuestFailure {
                refusal: Refusal::GuestTrap,
                fuel_used: 0,
                logs: Vec::new(),
            });
        }
        store.set_epoch_deadline(self.limits.epoch_ticks());

        let instance = match self.linker.instantiate(&mut store, module) {
            Ok(instance) => instance,
            Err(error) => {
                let refusal = classify(&error, &store);
                return Err(self.failure(&mut store, refusal));
            }
        };

        let memory = match instance.get_memory(&mut store, MEMORY_EXPORT) {
            Some(memory) => memory,
            None => return Err(self.failure(&mut store, Refusal::FrameInvalid)),
        };
        let alloc = match instance.get_typed_func::<i32, i32>(&mut store, ALLOC_EXPORT) {
            Ok(func) => func,
            Err(_) => return Err(self.failure(&mut store, Refusal::FrameInvalid)),
        };
        let entry = match instance.get_typed_func::<(i32, i32), i64>(&mut store, ENTRY_EXPORT) {
            Ok(func) => func,
            Err(_) => return Err(self.failure(&mut store, Refusal::FrameInvalid)),
        };

        let payload = request.payload.as_slice();
        // The input goes through the guest's own allocator, exactly as the PDK
        // documents: the host writes it before `marula_entry` is called.
        let ptr = match alloc.call(&mut store, payload.len().max(1) as i32) {
            Ok(ptr) if ptr >= 0 => ptr,
            Ok(_) => return Err(self.failure(&mut store, Refusal::FrameInvalid)),
            Err(error) => {
                let refusal = classify(&error, &store);
                return Err(self.failure(&mut store, refusal));
            }
        };
        if memory.write(&mut store, ptr as usize, payload).is_err() {
            return Err(self.failure(&mut store, Refusal::FrameInvalid));
        }

        let packed = match entry.call(&mut store, (ptr, payload.len() as i32)) {
            Ok(packed) => packed,
            Err(error) => {
                let refusal = classify(&error, &store);
                return Err(self.failure(&mut store, refusal));
            }
        };

        // The output cap is checked against the length the guest CLAIMED,
        // before a byte of it is copied: a guest that returns `(0, u32::MAX)`
        // is refused without the host allocating anything.
        let (out_ptr, out_len) = abi::unpack(packed);
        if out_len as usize > abi::MAX_OUTPUT_BYTES {
            return Err(self.failure(&mut store, Refusal::OutputTooLarge));
        }
        let output = {
            let data = memory.data(&store);
            let start = out_ptr as usize;
            let end = start.saturating_add(out_len as usize);
            match data.get(start..end) {
                Some(bytes) => bytes.to_vec(),
                None => return Err(self.failure(&mut store, Refusal::FrameInvalid)),
            }
        };

        // The answer is already valid, so a trap in `marula_free` cannot change
        // it: the store is about to be dropped and the PDK's arena never
        // returns memory to the guest anyway.
        if let Ok(free) = instance.get_typed_func::<(i32, i32), ()>(&mut store, FREE_EXPORT) {
            let _ = free.call(&mut store, (ptr, payload.len() as i32));
        }

        let fuel_used = fuel_used(&store);
        Ok(GuestOutput {
            output: if output.is_empty() {
                None
            } else {
                Some(output)
            },
            fuel_used,
            logs: store.data_mut().invocation.take_logs(),
        })
    }

    fn failure(&self, store: &mut Store<StoreData>, refusal: Refusal) -> GuestFailure {
        GuestFailure {
            refusal,
            // The store survives its own trap, so the exact figure is available
            // even for a refused invocation (ADR 0023 Decision 7).
            fuel_used: fuel_used(store),
            // A guest that logged before it trapped did log: the lines are
            // drained here too, so a refused invocation is as debuggable as a
            // successful one.
            logs: store.data_mut().invocation.take_logs(),
        }
    }
}

impl StoreData {
    fn new(invocation: InvocationState, limits: SandboxLimits) -> Self {
        StoreData {
            invocation,
            fuel_budget: limits.fuel,
            limits: StoreLimitsBuilder::new()
                .memory_size(limits.instance_memory_bytes)
                .instances(1)
                .memories(1)
                .tables(1)
                // A failed grow is a trap, not a `-1` the guest can ignore.
                .trap_on_grow_failure(true)
                .build(),
        }
    }
}

fn fuel_used(store: &Store<StoreData>) -> u64 {
    store
        .get_fuel()
        .map(|remaining| store.data().fuel_budget.saturating_sub(remaining))
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// The linker: the `marula` namespace, one entry per registry row
// ---------------------------------------------------------------------------

/// What a host call produced, before it is turned into the guest's return word.
enum HostValue {
    Unit,
    Word(u32),
    Int(i64),
}

fn entry_returns(registry: &Registry, name: &'static str) -> wasmtime::Result<AbiReturn> {
    registry
        .get(name)
        .map(|entry| entry.returns)
        .ok_or_else(|| wasmtime::Error::msg(format!("host entry {name} is not registered")))
}

/// Builds the linker. The `marula` namespace is the ONLY namespace defined, so
/// every other import — WASI included — fails at instantiation.
fn build_linker(engine: &Engine, registry: Arc<Registry>) -> wasmtime::Result<Linker<StoreData>> {
    let mut linker: Linker<StoreData> = Linker::new(engine);
    let namespace = abi::HOST_NAMESPACE;

    let returns = entry_returns(&registry, "log")?;
    let host = Arc::clone(&registry);
    linker.func_wrap(
        namespace,
        "log",
        move |mut caller: Caller<'_, StoreData>, level: u32, ptr: i32, len: i32| {
            dispatch(
                &host,
                "log",
                returns,
                &mut caller,
                &[level as i32, ptr, len],
            )?;
            Ok(())
        },
    )?;

    let returns = entry_returns(&registry, "kv_get")?;
    let host = Arc::clone(&registry);
    linker.func_wrap(
        namespace,
        "kv_get",
        move |mut caller: Caller<'_, StoreData>,
              ns_ptr: i32,
              ns_len: i32,
              k_ptr: i32,
              k_len: i32| {
            as_int(dispatch(
                &host,
                "kv_get",
                returns,
                &mut caller,
                &[ns_ptr, ns_len, k_ptr, k_len],
            )?)
        },
    )?;

    let returns = entry_returns(&registry, "kv_put")?;
    let host = Arc::clone(&registry);
    linker.func_wrap(
        namespace,
        "kv_put",
        move |mut caller: Caller<'_, StoreData>,
              ns_ptr: i32,
              ns_len: i32,
              key_ptr: i32,
              key_len: i32,
              value_ptr: i32,
              value_len: i32| {
            as_word(dispatch(
                &host,
                "kv_put",
                returns,
                &mut caller,
                &[ns_ptr, ns_len, key_ptr, key_len, value_ptr, value_len],
            )?)
        },
    )?;

    let returns = entry_returns(&registry, "kv_delete")?;
    let host = Arc::clone(&registry);
    linker.func_wrap(
        namespace,
        "kv_delete",
        move |mut caller: Caller<'_, StoreData>,
              ns_ptr: i32,
              ns_len: i32,
              k_ptr: i32,
              k_len: i32| {
            as_word(dispatch(
                &host,
                "kv_delete",
                returns,
                &mut caller,
                &[ns_ptr, ns_len, k_ptr, k_len],
            )?)
        },
    )?;

    let returns = entry_returns(&registry, "kv_list")?;
    let host = Arc::clone(&registry);
    linker.func_wrap(
        namespace,
        "kv_list",
        move |mut caller: Caller<'_, StoreData>,
              ns_ptr: i32,
              ns_len: i32,
              prefix_ptr: i32,
              prefix_len: i32,
              cursor_ptr: i32,
              cursor_len: i32| {
            as_int(dispatch(
                &host,
                "kv_list",
                returns,
                &mut caller,
                &[
                    ns_ptr, ns_len, prefix_ptr, prefix_len, cursor_ptr, cursor_len,
                ],
            )?)
        },
    )?;

    let returns = entry_returns(&registry, "http_request")?;
    let host = Arc::clone(&registry);
    linker.func_wrap(
        namespace,
        "http_request",
        move |mut caller: Caller<'_, StoreData>,
              profile_ptr: i32,
              profile_len: i32,
              req_ptr: i32,
              req_len: i32| {
            as_int(dispatch(
                &host,
                "http_request",
                returns,
                &mut caller,
                &[profile_ptr, profile_len, req_ptr, req_len],
            )?)
        },
    )?;

    let returns = entry_returns(&registry, "emit_event")?;
    let host = Arc::clone(&registry);
    linker.func_wrap(
        namespace,
        "emit_event",
        move |mut caller: Caller<'_, StoreData>,
              type_ptr: i32,
              type_len: i32,
              payload_ptr: i32,
              payload_len: i32| {
            as_word(dispatch(
                &host,
                "emit_event",
                returns,
                &mut caller,
                &[type_ptr, type_len, payload_ptr, payload_len],
            )?)
        },
    )?;

    let returns = entry_returns(&registry, "read_model_query")?;
    let host = Arc::clone(&registry);
    linker.func_wrap(
        namespace,
        "read_model_query",
        move |mut caller: Caller<'_, StoreData>, query: u32, params_ptr: i32, params_len: i32| {
            as_int(dispatch(
                &host,
                "read_model_query",
                returns,
                &mut caller,
                &[query as i32, params_ptr, params_len],
            )?)
        },
    )?;

    let returns = entry_returns(&registry, "now")?;
    let host = Arc::clone(&registry);
    linker.func_wrap(
        namespace,
        "now",
        move |mut caller: Caller<'_, StoreData>| -> wasmtime::Result<i64> {
            as_int(dispatch(&host, "now", returns, &mut caller, &[])?)
        },
    )?;

    Ok(linker)
}

fn as_int(value: HostValue) -> wasmtime::Result<i64> {
    match value {
        HostValue::Int(value) => Ok(value),
        other => Err(wasmtime::Error::msg(format!(
            "host entry returned the wrong shape: {}",
            shape_name(&other)
        ))),
    }
}

fn as_word(value: HostValue) -> wasmtime::Result<u32> {
    match value {
        HostValue::Word(word) => Ok(word),
        other => Err(wasmtime::Error::msg(format!(
            "host entry returned the wrong shape: {}",
            shape_name(&other)
        ))),
    }
}

fn shape_name(value: &HostValue) -> &'static str {
    match value {
        HostValue::Unit => "unit",
        HostValue::Word(_) => "word",
        HostValue::Int(_) => "int",
    }
}

/// The one dispatch path: the registry resolves the gate before it calls the
/// handler, and this function only maps the handler's answer onto the guest's
/// return word.
fn dispatch(
    registry: &Registry,
    name: &'static str,
    returns: AbiReturn,
    caller: &mut Caller<'_, StoreData>,
    args: &[i32],
) -> wasmtime::Result<HostValue> {
    let memory = caller
        .get_export(MEMORY_EXPORT)
        .and_then(|export| export.into_memory())
        .ok_or_else(|| wasmtime::Error::msg("guest exports no memory"))?;

    // `data_and_store_mut` splits the store borrow so the host function sees an
    // immutable view of linear memory and a mutable view of the invocation
    // state at the same time. Nothing here is unsafe in wasmtime's terms: the
    // memory bytes and the store data are disjoint.
    let outcome = {
        let (bytes, data) = memory.data_and_store_mut(&mut *caller);
        let view = SliceMemory { bytes };
        registry.invoke(name, &mut data.invocation, &view, args)
    };

    match outcome {
        Ok(HostReturn::None) => Ok(HostValue::Unit),
        Ok(HostReturn::Status(word)) => Ok(HostValue::Word(word)),
        Ok(HostReturn::Int(value)) => Ok(HostValue::Int(value)),
        Ok(HostReturn::Data(payload)) => {
            let (ptr, len) = write_guest(caller, memory, &payload)?;
            Ok(HostValue::Int(abi::pack(ptr, len)))
        }
        Err(refusal) => match returns {
            // A data-returning call hands its refusal back in-band as a tagged
            // frame, which is how the PDK turns it into `Err(Refusal)`.
            AbiReturn::Data => {
                let frame = abi::err_frame(refusal);
                let (ptr, len) = write_guest(caller, memory, &frame)?;
                Ok(HostValue::Int(abi::pack(ptr, len)))
            }
            AbiReturn::Status => Ok(HostValue::Word(refusal.code())),
            // `log` returns nothing and `now` carries no gate, so a refusal
            // here has no in-band channel; the worker's log stream is where it
            // surfaces. It is never silently a success.
            AbiReturn::None | AbiReturn::Int => {
                tracing::debug!(
                    host_function = name,
                    reason = refusal.as_str(),
                    "host call refused with no in-band channel"
                );
                Ok(match returns {
                    AbiReturn::Int => HostValue::Int(0),
                    _ => HostValue::Unit,
                })
            }
        },
    }
}

/// Writes `bytes` into guest memory through the guest's own allocator and
/// returns the `(pointer, length)` pair.
fn write_guest(
    caller: &mut Caller<'_, StoreData>,
    memory: Memory,
    bytes: &[u8],
) -> wasmtime::Result<(u32, u32)> {
    let alloc = caller
        .get_export(ALLOC_EXPORT)
        .and_then(|export| export.into_func())
        .ok_or_else(|| wasmtime::Error::msg("guest exports no marula_alloc"))?;
    let alloc = alloc.typed::<i32, i32>(&*caller)?;
    let ptr = alloc.call(&mut *caller, bytes.len().max(1) as i32)?;
    if ptr < 0 {
        return Err(wasmtime::Error::msg(
            "marula_alloc returned a negative pointer",
        ));
    }
    memory.write(&mut *caller, ptr as usize, bytes)?;
    Ok((ptr as u32, bytes.len() as u32))
}

/// [`GuestMemory`] over a borrowed slice of linear memory. `len == 0` is the
/// empty range whatever the pointer holds, matching the registry's convention.
struct SliceMemory<'a> {
    bytes: &'a [u8],
}

impl GuestMemory for SliceMemory<'_> {
    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn read(&self, ptr: u32, len: u32) -> Result<Vec<u8>, Refusal> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let start = ptr as usize;
        let end = start
            .checked_add(len as usize)
            .ok_or(Refusal::FrameInvalid)?;
        self.bytes
            .get(start..end)
            .map(<[u8]>::to_vec)
            .ok_or(Refusal::FrameInvalid)
    }
}

// ---------------------------------------------------------------------------
// Load-time ABI verification
// ---------------------------------------------------------------------------

fn expected_func_type(engine: &Engine, params: &[AbiParam], returns: AbiReturn) -> FuncType {
    let params: Vec<ValType> = params
        .iter()
        .map(|param| match param {
            AbiParam::I32 | AbiParam::U32 => ValType::I32,
        })
        .collect();
    let results: &[ValType] = match returns {
        AbiReturn::None => &[],
        AbiReturn::Status => &[ValType::I32],
        AbiReturn::Data | AbiReturn::Int => &[ValType::I64],
    };
    FuncType::new(engine, params, results.iter().cloned())
}

fn expect_export(
    module: &Module,
    name: &str,
    params: &[ValType],
    results: &[ValType],
) -> Result<(), LoadError> {
    let expected = FuncType::new(
        module.engine(),
        params.iter().cloned(),
        results.iter().cloned(),
    );
    match module.get_export(name) {
        Some(ExternType::Func(actual)) if actual.matches(&expected) => Ok(()),
        Some(ExternType::Func(actual)) => Err(LoadError::Rejected(format!(
            "export `{name}` is {actual:?}, the ABI fixes {expected:?}"
        ))),
        Some(_) => Err(LoadError::Rejected(format!(
            "export `{name}` is not a function"
        ))),
        None => Err(LoadError::Rejected(format!("no `{name}` export"))),
    }
}

/// Refuses a module whose surface is not the ABI, before it is cached. Every
/// import must be `marula::<entry>` with the signature the registry declares;
/// the three exports and `memory` must exist with the fixed types.
fn verify_module(module: &Module, registry: &Registry) -> Result<(), LoadError> {
    for import in module.imports() {
        if import.module() != abi::HOST_NAMESPACE {
            return Err(LoadError::Rejected(format!(
                "imports {}::{}; `{}` is the only importable namespace",
                import.module(),
                import.name(),
                abi::HOST_NAMESPACE
            )));
        }
        let entry = registry.get(import.name()).ok_or_else(|| {
            LoadError::Rejected(format!(
                "imports marula::{}, which the ABI does not declare",
                import.name()
            ))
        })?;
        let expected = expected_func_type(module.engine(), entry.params, entry.returns);
        match import.ty() {
            ExternType::Func(actual) if actual.matches(&expected) => {}
            ExternType::Func(actual) => {
                return Err(LoadError::Rejected(format!(
                    "marula::{} is {actual:?}, the ABI declares {expected:?}",
                    import.name()
                )))
            }
            other => {
                return Err(LoadError::Rejected(format!(
                    "marula::{} is imported as {other:?}, not a function",
                    import.name()
                )))
            }
        }
    }

    expect_export(module, ALLOC_EXPORT, &[ValType::I32], &[ValType::I32])?;
    expect_export(
        module,
        ENTRY_EXPORT,
        &[ValType::I32, ValType::I32],
        &[ValType::I64],
    )?;
    expect_export(module, FREE_EXPORT, &[ValType::I32, ValType::I32], &[])?;
    match module.get_export(MEMORY_EXPORT) {
        Some(ExternType::Memory(_)) => Ok(()),
        _ => Err(LoadError::Rejected(format!("no `{MEMORY_EXPORT}` export"))),
    }
}

// ---------------------------------------------------------------------------
// Trap classification
// ---------------------------------------------------------------------------

/// Turns a wasmtime error into the audit row's reason. Fuel and the epoch have
/// precise trap variants; everything else a guest can do to itself is a trap.
fn classify(error: &wasmtime::Error, store: &Store<StoreData>) -> Refusal {
    if let Some(trap) = error.downcast_ref::<Trap>() {
        return match trap {
            Trap::OutOfFuel => Refusal::FuelExhausted,
            Trap::Interrupt => Refusal::WallClockExceeded,
            _ => Refusal::GuestTrap,
        };
    }
    // A host-side error raised after the fuel ran out (an allocation inside a
    // host function, for instance) carries no trap of its own.
    if store.get_fuel().map(|left| left == 0).unwrap_or(false) {
        return Refusal::FuelExhausted;
    }
    Refusal::GuestTrap
}

// ---------------------------------------------------------------------------
// The epoch ticker
// ---------------------------------------------------------------------------

/// Increments the engine epoch every tick. A store's deadline is expressed in
/// ticks, so this thread is the wall-clock limit: without it, `set_epoch_deadline`
/// never fires. It parks on a condvar, so dropping a sandbox stops it at once
/// instead of waiting out a tick.
struct EpochTicker {
    signal: Arc<(Mutex<bool>, Condvar)>,
    handle: Option<JoinHandle<()>>,
}

impl EpochTicker {
    fn start(engine: Engine, tick_ms: u64) -> Result<Self, SandboxError> {
        let signal = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_signal = Arc::clone(&signal);
        let tick = Duration::from_millis(tick_ms.max(1));
        let handle = std::thread::Builder::new()
            .name("plugin-epoch".into())
            .spawn(move || {
                let (lock, condvar) = &*worker_signal;
                loop {
                    {
                        let guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                        let (guard, _) = condvar
                            .wait_timeout(guard, tick)
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if *guard {
                            return;
                        }
                    }
                    engine.increment_epoch();
                }
            })
            .map_err(|error| SandboxError(format!("epoch ticker: {error}")))?;
        Ok(EpochTicker {
            signal,
            handle: Some(handle),
        })
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        {
            let (lock, condvar) = &*self.signal;
            *lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
            condvar.notify_all();
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn sandbox(limits: SandboxLimits) -> Sandbox {
        Sandbox::with_limits(Arc::new(crate::host_fns::register_all()), limits)
            .expect("the sandbox builds")
    }

    fn default_sandbox() -> Sandbox {
        sandbox(SandboxLimits::default())
    }

    fn artifact(wat_source: &str) -> ModuleArtifact {
        let bytes = wat::parse_str(wat_source).expect("valid WAT");
        ModuleArtifact {
            digest: ModuleArtifact::digest_of(&bytes),
            bytes,
        }
    }

    fn request() -> InvocationRequest {
        InvocationRequest {
            invocation_id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            install_id: Uuid::now_v7(),
            entry: "plugin.test.entry".into(),
            payload: br#"{"ticket":"01J"}"#.to_vec(),
            granted: Vec::new(),
            declared_events: Vec::new(),
            egress: abi::EgressScopes::new(),
        }
    }

    /// A guest with the three exports, a bump allocator and an entry point that
    /// returns the two bytes it logs.
    const GREETER: &str = r#"
        (module
          (import "marula" "log" (func $log (param i32 i32 i32)))
          (import "marula" "now" (func $now (result i64)))
          (memory (export "memory") 1)
          (data (i32.const 3000) "OK")
          (global $next (mut i32) (i32.const 1024))
          (func (export "marula_alloc") (param $len i32) (result i32)
            (local $ptr i32)
            (local.set $ptr (global.get $next))
            (global.set $next (i32.add (global.get $next) (i32.add (local.get $len) (i32.const 8))))
            (local.get $ptr))
          (func (export "marula_free") (param $ptr i32) (param $len i32))
          (func (export "marula_entry") (param $ptr i32) (param $len i32) (result i64)
            (call $log (i32.const 3) (i32.const 3000) (i32.const 2))
            (drop (call $now))
            (i64.or (i64.shl (i64.const 3000) (i64.const 32)) (i64.const 2))))
    "#;

    /// A guest that never stops.
    const SPINNER: &str = r#"
        (module
          (memory (export "memory") 1)
          (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
          (func (export "marula_free") (param i32) (param i32))
          (func (export "marula_entry") (param i32) (param i32) (result i64)
            (loop $spin (br $spin))
            (i64.const 0)))
    "#;

    #[test]
    fn a_module_runs_and_its_output_and_logs_come_back() {
        let sandbox = default_sandbox();
        let artifact = artifact(GREETER);
        let module = sandbox.load(&artifact).expect("loads");
        let output = sandbox.run(&module, &request()).expect("runs");
        assert_eq!(output.output.as_deref(), Some(&b"OK"[..]));
        assert_eq!(
            output.logs,
            vec![LogLine {
                level: 3,
                message: "OK".into()
            }]
        );
        assert!(output.fuel_used > 0, "a real guest burns fuel");
        assert!(
            output.fuel_used < abi::FUEL_PER_INVOCATION,
            "a healthy guest must stay well inside its budget"
        );
    }

    #[test]
    fn the_same_digest_is_compiled_once() {
        let sandbox = default_sandbox();
        let artifact = artifact(GREETER);
        let first = sandbox.load(&artifact).expect("loads");
        let second = sandbox.load(&artifact).expect("loads from cache");
        assert_eq!(sandbox.cached_count(), 1);
        assert!(Arc::ptr_eq(&first, &second));
        assert!(sandbox.contains(&artifact.digest));
    }

    fn tiny_guest(marker: i32) -> String {
        format!(
            r#"
            (module
              (memory (export "memory") 1)
              (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
              (func (export "marula_free") (param i32) (param i32))
              (func (export "marula_entry") (param i32) (param i32) (result i64)
                (i64.or (i64.shl (i64.const {marker}) (i64.const 32)) (i64.const 0))))
        "#
        )
    }

    #[test]
    fn the_module_cache_names_the_digest_it_evicted() {
        let sandbox = default_sandbox();
        let artifacts: Vec<ModuleArtifact> = (0..MAX_CACHED_MODULES)
            .map(|index| artifact(&tiny_guest(index as i32 + 1)))
            .collect();
        for artifact in &artifacts {
            sandbox.load(artifact).expect("each module loads");
        }
        assert_eq!(sandbox.cached_count(), MAX_CACHED_MODULES);

        let (_, evicted) = sandbox
            .load_with_eviction(&artifact(&tiny_guest(9_999)))
            .expect("the next module loads");
        assert_eq!(
            evicted.as_deref(),
            Some(artifacts[0].digest.as_str()),
            "the least recently used digest is the one reported"
        );
        assert_eq!(sandbox.cached_count(), MAX_CACHED_MODULES);
        assert!(!sandbox.contains(&artifacts[0].digest));
        assert!(sandbox.contains(&artifacts[1].digest));
    }

    #[test]
    fn a_digest_mismatch_is_refused_before_compiling() {
        let sandbox = default_sandbox();
        let mut artifact = artifact(GREETER);
        let honest = artifact.digest.clone();
        artifact.bytes.push(0);
        match sandbox.load(&artifact) {
            Err(LoadError::DigestMismatch { declared, actual }) => {
                assert_eq!(declared, honest);
                assert_eq!(actual, ModuleArtifact::digest_of(&artifact.bytes));
            }
            other => panic!("expected a digest mismatch, got {other:?}"),
        }
        assert_eq!(sandbox.cached_count(), 0);
    }

    #[test]
    fn fuel_exhaustion_is_refused_and_reports_the_whole_budget() {
        let limit = SandboxLimits {
            fuel: 10_000,
            ..SandboxLimits::default()
        };
        let sandbox = sandbox(limit);
        let module = sandbox.load(&artifact(SPINNER)).expect("loads");
        let failure = sandbox
            .run(&module, &request())
            .expect_err("a spin must burn the budget");
        assert_eq!(failure.refusal, Refusal::FuelExhausted);
        assert_eq!(failure.fuel_used, 10_000);
    }

    #[test]
    fn the_epoch_deadline_stops_a_guest_that_still_has_fuel() {
        let limit = SandboxLimits {
            wall_clock_ms: 20,
            epoch_tick_ms: 5,
            ..SandboxLimits::default()
        };
        let sandbox = sandbox(limit);
        let module = sandbox.load(&artifact(SPINNER)).expect("loads");
        let started = std::time::Instant::now();
        let failure = sandbox
            .run(&module, &request())
            .expect_err("the epoch must stop a spin");
        assert_eq!(failure.refusal, Refusal::WallClockExceeded);
        assert!(
            failure.fuel_used < abi::FUEL_PER_INVOCATION,
            "the deadline, not the fuel cap, is what stopped it"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_millis(2_000),
            "the deadline is 20 ms; waiting seconds means it did not fire"
        );
    }

    #[test]
    fn a_memory_growth_past_the_instance_cap_traps() {
        let limit = SandboxLimits {
            instance_memory_bytes: 128 * 1024,
            ..SandboxLimits::default()
        };
        let sandbox = sandbox(limit);
        let guest = r#"
            (module
              (memory (export "memory") 1)
              (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
              (func (export "marula_free") (param i32) (param i32))
              (func (export "marula_entry") (param i32) (param i32) (result i64)
                (drop (memory.grow (i32.const 1000)))
                (i64.const 0)))
        "#;
        let module = sandbox.load(&artifact(guest)).expect("loads");
        let failure = sandbox.run(&module, &request()).expect_err("must trap");
        assert_eq!(failure.refusal, Refusal::GuestTrap);
    }

    #[test]
    fn an_output_above_the_cap_is_refused_before_it_is_read() {
        let sandbox = default_sandbox();
        let guest = format!(
            r#"
            (module
              (memory (export "memory") 1)
              (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
              (func (export "marula_free") (param i32) (param i32))
              (func (export "marula_entry") (param i32) (param i32) (result i64)
                (i64.or (i64.shl (i64.const 1024) (i64.const 32)) (i64.const {}))))
            "#,
            abi::MAX_OUTPUT_BYTES + 1
        );
        let module = sandbox.load(&artifact(&guest)).expect("loads");
        let failure = sandbox.run(&module, &request()).expect_err("must refuse");
        assert_eq!(failure.refusal, Refusal::OutputTooLarge);
    }

    #[test]
    fn a_failed_host_call_hands_its_refusal_frame_to_the_guest() {
        let sandbox = default_sandbox();
        let guest = r#"
            (module
              (import "marula" "read_model_query" (func $query (param i32 i32 i32) (result i64)))
              (memory (export "memory") 1)
              (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
              (func (export "marula_free") (param i32) (param i32))
              (func (export "marula_entry") (param i32) (param i32) (result i64)
                (call $query (i32.const 1) (i32.const 0) (i32.const 0))))
        "#;
        let module = sandbox.load(&artifact(guest)).expect("loads");
        let output = sandbox
            .run(&module, &request())
            .expect("the guest returns the frame");
        assert_eq!(
            output.output.as_deref(),
            Some(abi::err_frame(Refusal::CapabilityNotGranted).as_slice()),
            "the guest sees the tagged refusal frame the ABI documents"
        );
    }

    #[test]
    fn the_frame_s_grants_are_what_the_host_enforces() {
        let sandbox = default_sandbox();
        let guest = r#"
            (module
              (import "marula" "emit_event" (func $emit (param i32 i32 i32 i32) (result i32)))
              (memory (export "memory") 1)
              (data (i32.const 4000) "ticket.escalated")
              (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
              (func (export "marula_free") (param i32) (param i32))
              (func (export "marula_entry") (param i32) (param i32) (result i64)
                (i32.store (i32.const 4096)
                  (call $emit (i32.const 4000) (i32.const 16) (i32.const 0) (i32.const 0)))
                (i64.or (i64.shl (i64.const 4096) (i64.const 32)) (i64.const 4))))
        "#;
        let module = sandbox.load(&artifact(guest)).expect("loads");

        // No grant in the frame: the capability gate refuses.
        let output = sandbox.run(&module, &request()).expect("runs");
        assert_eq!(
            output.output.as_deref(),
            Some(
                Refusal::CapabilityNotGranted
                    .code()
                    .to_le_bytes()
                    .as_slice()
            )
        );

        // The grant and the declaration both in the frame: the gate passes and
        // the deferred body answers `plugin_not_implemented`, which is W53-2's
        // stub refusing rather than this lane pretending.
        let mut request = request();
        request.granted = vec!["events.emit".into()];
        request.declared_events = vec!["ticket.escalated".into()];
        let output = sandbox.run(&module, &request).expect("runs");
        assert_eq!(
            output.output.as_deref(),
            Some(Refusal::NotImplemented.code().to_le_bytes().as_slice())
        );
    }

    #[test]
    fn a_grant_the_closed_vocabulary_does_not_define_refuses_the_frame() {
        let sandbox = default_sandbox();
        let module = sandbox.load(&artifact(GREETER)).expect("loads");
        let mut request = request();
        request.granted = vec!["sudo.everything".into()];
        let failure = sandbox.run(&module, &request).expect_err("must refuse");
        assert_eq!(failure.refusal, Refusal::FrameInvalid);
        assert_eq!(failure.fuel_used, 0, "nothing ran");
    }

    #[test]
    fn an_oversize_payload_is_refused_before_the_store_is_built() {
        let sandbox = default_sandbox();
        let module = sandbox.load(&artifact(GREETER)).expect("loads");
        let mut request = request();
        request.payload = vec![0; abi::MAX_INPUT_BYTES + 1];
        let failure = sandbox.run(&module, &request).expect_err("must refuse");
        assert_eq!(failure.refusal, Refusal::InputTooLarge);
    }

    fn rejected(source: &str) -> String {
        let sandbox = default_sandbox();
        match sandbox.load(&artifact(source)) {
            Err(LoadError::Rejected(reason)) => reason,
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    #[test]
    fn a_module_importing_anything_but_the_marula_namespace_is_rejected() {
        let reason = rejected(
            r#"
            (module
              (import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))
              (memory (export "memory") 1)
              (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
              (func (export "marula_free") (param i32) (param i32))
              (func (export "marula_entry") (param i32) (param i32) (result i64) (i64.const 0)))
            "#,
        );
        assert!(reason.contains("wasi_snapshot_preview1"), "{reason}");
    }

    #[test]
    fn a_module_importing_an_undeclared_marula_entry_is_rejected() {
        let reason = rejected(
            r#"
            (module
              (import "marula" "secret_get" (func $secret (param i32 i32) (result i64)))
              (memory (export "memory") 1)
              (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
              (func (export "marula_free") (param i32) (param i32))
              (func (export "marula_entry") (param i32) (param i32) (result i64) (i64.const 0)))
            "#,
        );
        assert!(reason.contains("secret_get"), "{reason}");
    }

    #[test]
    fn a_module_with_the_wrong_import_signature_is_rejected() {
        let reason = rejected(
            r#"
            (module
              (import "marula" "kv_put" (func $put (param i32) (result i32)))
              (memory (export "memory") 1)
              (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
              (func (export "marula_free") (param i32) (param i32))
              (func (export "marula_entry") (param i32) (param i32) (result i64) (i64.const 0)))
            "#,
        );
        assert!(reason.contains("kv_put"), "{reason}");
    }

    #[test]
    fn a_module_without_the_three_exports_is_rejected() {
        let reason = rejected(
            r#"
            (module
              (memory (export "memory") 1)
              (func (export "marula_alloc") (param i32) (result i32) (i32.const 1024))
              (func (export "marula_entry") (param i32) (param i32) (result i64) (i64.const 0)))
            "#,
        );
        assert!(reason.contains(FREE_EXPORT), "{reason}");
    }

    #[test]
    fn a_module_that_is_not_wasm_at_all_is_rejected() {
        let sandbox = default_sandbox();
        let artifact = ModuleArtifact {
            digest: ModuleArtifact::digest_of(b"not wasm"),
            bytes: b"not wasm".to_vec(),
        };
        assert!(matches!(
            sandbox.load(&artifact),
            Err(LoadError::Rejected(_))
        ));
    }
}
