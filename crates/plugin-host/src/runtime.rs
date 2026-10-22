//! The supervised plugin pool: worker processes, crash containment, quarantine,
//! and the four concurrency gates (ADR 0023 Decisions 2, 4 and 7).
//!
//! ## The shape
//!
//! One child process per worker, spawned by whatever process owns the pool
//! (`all-in-one`: one worker inside core-api's container; `split`: two, born the
//! same way). A worker answers one invocation at a time, so pool capacity is the
//! worker count and queueing is the pool's own. A worker that dies, hangs, or
//! answers a frame the parent cannot trust is killed and replaced; the
//! invocation that found it gets a well-formed
//! [`abi::InvocationReply::worker_failed`] instead of a hang or an error the
//! caller has to interpret.
//!
//! ## What the parent decides, and what the worker may not
//!
//! The parent has already run [`crate::authorize_invocation`] and resolved the
//! capability, the declared event types and the consented egress profile into
//! the [`abi::InvocationRequest`] it sends. The worker performs NO
//! authorization: it re-validates the frame into an
//! [`crate::host_fns::InvocationState`] that starts empty, refuses a frame that
//! does not validate whole, and can only ever run the module whose digest the
//! parent named. A bug in the worker can run a module it should not have; it
//! cannot decide that a host call is allowed.
//!
//! ## What this module does not do
//!
//! It does not write the audit row. Decision 7 puts that at the call site that
//! owns the transaction, which is W53-5's `plugin.action` route: every
//! [`InvocationResult`] this module returns is audit-ready (`outcome`,
//! `reason`, `fuel_used`, `duration_us`) and carries the `worker` label.
//!
//! ## The gates, outermost first
//!
//! 1. `enabled()` — an admin-disabled runtime refuses everything.
//! 2. the frame validates, and the digest is a sha256 hex string;
//! 3. quarantine — an install that keeps killing workers is refused until an
//!    operator clears it;
//! 4. the per-install rate bucket (300/min, burst 60);
//! 5. the per-org slot (1 concurrent invocation per org);
//! 6. the per-install slot (1 — a plugin's own read-modify-write on KV cannot
//!    interleave with itself);
//! 7. a free worker (1 concurrent invocation per worker).
//!
//! Gates 3 and 4 live in the database and in this process respectively; gates
//! 5-7 are process-local. See [`PluginPool`] for what that costs in `split`.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

use crate::abi::{self, InvocationOutcome, InvocationReply, InvocationRequest, Refusal};
use crate::host_fns::{register_all, LogLine};
use crate::isolation::{ModuleArtifact, Sandbox, SandboxLimits};
use crate::wire::{self, HostCallOp, HostCallOutcome, HostReplyFrame, InvokeFrame, ReplyFrame};
use crate::{InvocationResult, PluginRuntime};

/// The row W53-8's operator surface renders, re-exported so a caller does not
/// have to reach into `core-db` for the type it already receives.
pub use core_db::plugin_health::InstallHealth;

/// `pool` (default), `in_process` or `disabled` (ADR 0023 Decision 2).
pub const ENV_ISOLATION: &str = "MARULA_PLUGIN_ISOLATION";
/// Worker count override. The profile default is 1 for `all-in-one`, 2 for
/// `split`.
pub const ENV_WORKERS: &str = "MARULA_PLUGIN_WORKERS";
/// Path to the worker executable, when it is not the sibling of this binary.
pub const ENV_WORKER_BIN: &str = "MARULA_PLUGIN_WORKER_BIN";
/// The profile flag core-api already reads (`crate::config`'s `all_in_one`).
pub const ENV_ALL_IN_ONE: &str = "MARULA_ALL_IN_ONE";

/// The worker binary's name, both as a `cargo` target and as a file.
pub const WORKER_BIN_NAME: &str = "marula-plugin-worker";

/// Consecutive worker failures one install may cause before quarantine.
///
/// Three, because one is a flake, two is still a coincidence (a container under
/// memory pressure can take two children), and three in a row from the same
/// install is that install's artifact taking its worker down every time. A
/// deterministic crash-on-input bug reaches three within three requests, well
/// inside one rate bucket's worth of traffic, so the window an attacker gets is
/// tiny; and a plugin that fails once an hour is never quarantined because a
/// successful invocation resets the counter.
pub const QUARANTINE_AFTER_FAILURES: u32 = 3;

/// How long a health decision read from the database is trusted. It bounds two
/// things: how long an operator's quarantine clear takes to reach every replica,
/// and how often the invocation path performs that read (once per install per
/// window, not once per call).
pub const HEALTH_CACHE_TTL: Duration = Duration::from_secs(30);

/// How long a caller waits for a slot before it is refused. One invocation's
/// own wall-clock budget: a request that cannot start inside the time an
/// invocation is allowed to take is refused rather than queued behind an
/// unbounded line.
pub const CONCURRENCY_WAIT: Duration = Duration::from_millis(abi::WALL_CLOCK_MS);

/// How long the parent waits for one invocation's reply. Twice the guest's own
/// deadline: the epoch should stop the guest at 250 ms, and a worker that has
/// not answered by 500 ms has already ignored it once (Decision 4's restart
/// threshold).
pub const INVOCATION_TIMEOUT: Duration = Duration::from_millis(abi::WORKER_RESTART_THRESHOLD_MS);

/// How long the parent waits for a reply to a frame that carries module bytes.
/// Compiling a module is host work, not guest work, so it is not charged against
/// the invocation's wall clock.
pub const LOAD_TIMEOUT: Duration = Duration::from_millis(abi::WORKER_RESTART_THRESHOLD_MS * 4);

/// The ceiling on `MARULA_PLUGIN_WORKERS`. Each worker is its own resident set,
/// so a typo must not be able to ask for a hundred of them.
pub const MAX_WORKERS: usize = 8;

/// How many install entries the in-process gate table may hold before idle,
/// full buckets are pruned. An entry with a drained bucket is never pruned: its
/// state IS the rate limit.
const GATE_TABLE_SOFT_LIMIT: usize = 4096;

/// How often the worker measures its own resident set. Amortised so the
/// measurement is not on the per-invocation path (5-10 µs per read).
pub const RESIDENT_CHECK_EVERY: u64 = 32;

/// How many host calls one invocation may make through the parent bridge.
///
/// The arithmetic, against the ADR's own numbers:
///
/// - A host call is one pipe write, one parent store round trip on a connection
///   the invocation already holds, and one pipe read. On a local pipe the two
///   frames are tens of microseconds; the store round trip is the cost, and it
///   is one statement on an open connection.
/// - The guest's own wall clock is 250 ms (`abi::WALL_CLOCK_MS`) and the parent
///   kills a worker that has not answered in 500 ms
///   ([`INVOCATION_TIMEOUT`]). Sixty-four calls at a generous 1 ms each is
///   64 ms, a quarter of the guest's budget and an eighth of the parent's —
///   the cap refuses before the timeout, which matters because a timeout kills
///   the worker (`worker_failed`, and a strike towards quarantine) while this
///   refusal is a clean answer the guest can read and stop on.
/// - It covers the real shapes: a widget or a cache warm loading 64 keys, and a
///   `kv_list` walk of 64 pages is 32,000 keys (`abi::KV_LIST_PAGE`), which no
///   250 ms invocation can consume anyway.
///
/// A guest that keeps going past the cap is answered `BudgetExhausted` on every
/// later call, with no store work behind it. It cannot extend the invocation:
/// the same [`INVOCATION_TIMEOUT`] bounds the whole exchange, and the worker's
/// own fuel and epoch still bound the guest between calls.
pub const MAX_HOST_CALLS_PER_INVOCATION: u64 = 64;

/// The transport one deployment chose. `in_process` exists for a box that
/// cannot spare a worker's resident set; it is one knob, not a tier, and it
/// changes nothing about what a plugin may do (ADR 0023 Decision 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationMode {
    Pool,
    InProcess,
    Disabled,
}

impl IsolationMode {
    /// Parses `MARULA_PLUGIN_ISOLATION`. `in-process` is accepted beside the
    /// documented `in_process` because an operator will type it.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "pool" | "" => Some(IsolationMode::Pool),
            "in_process" | "in-process" => Some(IsolationMode::InProcess),
            "disabled" | "off" | "none" => Some(IsolationMode::Disabled),
            _ => None,
        }
    }
}

/// Everything [`PluginPool::start`] needs. The defaults are the shipped ones.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    pub mode: IsolationMode,
    pub workers: usize,
    /// Explicit worker executable. `None` resolves the sibling of the running
    /// binary, then `PATH`.
    pub worker_bin: Option<PathBuf>,
    /// The app-role pool the quarantine rows are written through. `None` keeps
    /// health in this process only, which a deployment should not ship: an
    /// in-memory counter is reset by a restart.
    pub health: Option<sqlx::PgPool>,
    pub sandbox_limits: SandboxLimits,
}

impl Default for PoolConfig {
    fn default() -> Self {
        PoolConfig {
            mode: IsolationMode::Pool,
            workers: 1,
            worker_bin: None,
            health: None,
            sandbox_limits: SandboxLimits::default(),
        }
    }
}

impl PoolConfig {
    /// Reads the environment. An unknown isolation mode is an error rather than
    /// a silent `pool`: a typo in a security knob must fail a boot, not choose
    /// for the operator.
    pub fn from_env() -> Result<Self, PoolError> {
        let mode = match std::env::var(ENV_ISOLATION) {
            Ok(value) => IsolationMode::parse(&value).ok_or_else(|| {
                PoolError::Config(format!(
                    "{ENV_ISOLATION}={value:?} is not one of pool, in_process, disabled"
                ))
            })?,
            Err(_) => IsolationMode::Pool,
        };
        let all_in_one = std::env::var(ENV_ALL_IN_ONE).as_deref() == Ok("1");
        let mut workers = if all_in_one { 1 } else { 2 };
        if let Ok(value) = std::env::var(ENV_WORKERS) {
            let parsed: usize = value.trim().parse().map_err(|_| {
                PoolError::Config(format!("{ENV_WORKERS}={value:?} is not a number"))
            })?;
            workers = parsed.clamp(1, MAX_WORKERS);
        }
        let worker_bin = std::env::var(ENV_WORKER_BIN)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from);
        Ok(PoolConfig {
            mode,
            workers,
            worker_bin,
            health: None,
            sandbox_limits: SandboxLimits::default(),
        })
    }

    pub fn with_workers(mut self, workers: usize) -> Self {
        self.workers = workers.clamp(1, MAX_WORKERS);
        self
    }

    pub fn with_worker_bin(mut self, path: impl Into<PathBuf>) -> Self {
        self.worker_bin = Some(path.into());
        self
    }

    pub fn with_health(mut self, pool: sqlx::PgPool) -> Self {
        self.health = Some(pool);
        self
    }
}

/// Why the pool could not be built. Every variant refuses the boot: a runtime
/// that half-starts would run plugins without one of Decision 4's limits.
#[derive(Debug)]
pub enum PoolError {
    Config(String),
    WorkerUnavailable { bin: String, detail: String },
    Sandbox(String),
    Health(String),
}

impl std::fmt::Display for PoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PoolError::Config(detail) => write!(f, "plugin pool configuration: {detail}"),
            PoolError::WorkerUnavailable { bin, detail } => {
                write!(f, "plugin worker {bin} could not start: {detail}")
            }
            PoolError::Sandbox(detail) => write!(f, "plugin sandbox: {detail}"),
            PoolError::Health(detail) => write!(f, "plugin health store: {detail}"),
        }
    }
}

impl std::error::Error for PoolError {}

/// The invocation a host call belongs to, as the PARENT knows it.
///
/// This is the answer to "which tenant is this call for", and it comes from the
/// [`InvocationRequest`] the parent dispatched — never from the frame the worker
/// sent. The frame cannot disagree with it because the frame has no field to
/// disagree in (`wire`'s header), and a service that is handed a context its own
/// record contradicts must refuse (`exec`'s implementation does).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostCallContext {
    pub invocation_id: Uuid,
    pub org_id: Uuid,
    pub install_id: Uuid,
}

/// The parent's side of the bridge: it answers a worker's host call.
///
/// One method, one call, one answer: the pool owns the framing, the correlators,
/// the cap and the timeout, so an implementation is only "do this verb for this
/// invocation". `plugin-host` cannot implement it against a database — the pool
/// has no connection — so the executor does (`crate::exec`), on the transaction
/// the invocation already holds.
pub trait HostCallService: Send {
    fn serve<'a>(
        &'a mut self,
        context: HostCallContext,
        op: HostCallOp,
    ) -> Pin<Box<dyn Future<Output = HostCallOutcome> + Send + 'a>>;
}

/// The service a pool with no store is handed.
///
/// It answers every host call `Unavailable`, which the worker turns into
/// `plugin_not_implemented`: the honest answer for a path that genuinely cannot
/// serve plugin state. `in_process` and the bare [`PluginRuntime::invoke`] entry
/// point use it today.
pub struct NoHostCalls;

impl HostCallService for NoHostCalls {
    fn serve<'a>(
        &'a mut self,
        _context: HostCallContext,
        _op: HostCallOp,
    ) -> Pin<Box<dyn Future<Output = HostCallOutcome> + Send + 'a>> {
        Box::pin(async move { HostCallOutcome::Refused(wire::HostCallRefusal::Unavailable) })
    }
}

/// The plugin runtime: a pool of worker processes plus the `in_process`
/// transport.
///
/// ## What an operator gets, and where it stops
///
/// - Quarantine is durable: the counter and the quarantine live in
///   `plugin_install_health` (migration 0154), so a restart does not clear it.
/// - The rate bucket and the three concurrency slots are per PROCESS. With two
///   core-api replicas the effective install rate is two buckets and the org
///   slot is two slots — honest and worth saying out loud. Making either global
///   costs a round trip per invocation (Valkey or Postgres), which Decision 4's
///   budget does not have room for; it belongs to whoever operates more than one
///   replica, with a measurement behind it.
/// - `in_process` cannot contain a panic or a hard hang: the guest shares the
///   API's address space, and a blocking thread that never returns cannot be
///   killed from outside. Its limits still apply (fuel, epoch, memory, output),
///   because they are the sandbox's and not the transport's.
pub struct PluginPool {
    id: Uuid,
    name: String,
    inner: Arc<Inner>,
}

impl std::fmt::Debug for PluginPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginPool")
            .field("id", &self.id)
            .field("mode", &self.inner.config.mode)
            .field("workers", &self.inner.config.workers)
            .field("idle", &self.inner.idle_count())
            .finish()
    }
}

struct Inner {
    config: PoolConfig,
    sandbox: Option<Arc<Sandbox>>,
    worker_bin: PathBuf,
    permits: Arc<Semaphore>,
    idle: Mutex<Vec<Member>>,
    gates: Mutex<Gates>,
    shutting_down: AtomicBool,
}

impl Inner {
    fn idle_count(&self) -> usize {
        self.idle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

impl PluginPool {
    /// Builds the pool and, in `pool` mode, its workers. A missing worker binary
    /// or one that does not speak this protocol fails the boot: that is a
    /// deployment error, not a per-request surprise.
    pub async fn start(config: PoolConfig) -> Result<Arc<Self>, PoolError> {
        let registry = Arc::new(register_all());
        let sandbox = match config.mode {
            IsolationMode::InProcess => Some(Arc::new(
                Sandbox::with_limits(Arc::clone(&registry), config.sandbox_limits)
                    .map_err(|error| PoolError::Sandbox(error.to_string()))?,
            )),
            IsolationMode::Pool | IsolationMode::Disabled => None,
        };
        let worker_bin = if config.mode == IsolationMode::Pool {
            resolve_worker_binary(&config)?
        } else {
            PathBuf::from(WORKER_BIN_NAME)
        };

        let inner = Arc::new(Inner {
            permits: Arc::new(Semaphore::new(config.workers.max(1))),
            config,
            sandbox,
            worker_bin,
            idle: Mutex::new(Vec::new()),
            gates: Mutex::new(Gates::default()),
            shutting_down: AtomicBool::new(false),
        });

        if inner.config.mode == IsolationMode::Pool {
            let mut members = Vec::with_capacity(inner.config.workers);
            for _ in 0..inner.config.workers {
                members.push(spawn_worker(&inner.worker_bin).await?);
            }
            *inner
                .idle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = members;
        }

        Ok(Arc::new(PluginPool {
            id: Uuid::now_v7(),
            name: "wasm-plugin-pool".into(),
            inner,
        }))
    }

    /// The worker count this pool was built with. Capacity equals it, because a
    /// worker answers one invocation at a time.
    pub fn worker_count(&self) -> usize {
        self.inner.config.workers
    }

    /// The transport this pool uses.
    pub fn mode(&self) -> IsolationMode {
        self.inner.config.mode
    }

    /// Stops every idle worker. An invocation already in flight finishes and its
    /// worker is then killed rather than returned to the pool.
    pub async fn shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);
        let members: Vec<Member> = {
            let mut idle = self
                .inner
                .idle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *idle)
        };
        for member in members {
            member.kill().await;
        }
    }

    /// The one entry point W53-5 calls.
    ///
    /// The caller has already authorized the invocation; this method never
    /// adds a grant, and it always answers with a validated reply. What it
    /// expects of the caller:
    ///
    /// * an [`InvocationRequest`] built from `authorize_invocation`'s own
    ///   resolution (the install's grants, the manifest's declared events, the
    ///   consented egress profile) — not from anything the guest or a worker
    ///   supplied;
    /// * the artifact's bytes and the digest they were admitted under;
    /// * that the caller writes the audit row (Decision 7) — including the
    ///   refusal rows this method returns before any worker was reached.
    ///
    /// This form serves no host calls: a `kv_get` from the guest is answered
    /// `plugin_not_implemented`. A caller that can serve them — anything with a
    /// transaction, which in this deployment is [`crate::exec`] — calls
    /// [`PluginPool::invoke_with`].
    pub async fn invoke(
        &self,
        request: &InvocationRequest,
        artifact: &ModuleArtifact,
    ) -> InvocationResult {
        self.run_invocation(request, artifact, &mut NoHostCalls)
            .await
    }

    /// [`PluginPool::invoke`] with a service for the guest's host calls.
    ///
    /// The service is the parent's half of the bridge: `kv_get` and its three
    /// siblings are one frame out of the worker and one call into this trait,
    /// and the trait answers from the invocation's own transaction. See
    /// [`HostCallService`] and [`MAX_HOST_CALLS_PER_INVOCATION`].
    pub async fn invoke_with(
        &self,
        request: &InvocationRequest,
        artifact: &ModuleArtifact,
        service: &mut dyn HostCallService,
    ) -> InvocationResult {
        self.run_invocation(request, artifact, service).await
    }

    /// Clears an install's quarantine. The admin route owns the transaction's
    /// `app.org_id` scope; this opens its own, because quarantine is written
    /// outside the request's effect transaction on purpose (a rolled-back
    /// request must not roll back the fact that the worker died).
    pub async fn clear_quarantine(
        &self,
        org_id: Uuid,
        install_id: Uuid,
        cleared_by: Option<Uuid>,
    ) -> Result<bool, PoolError> {
        let Some(pool) = self.inner.config.health.clone() else {
            // Memory-only deployment: forget the in-process health so the next
            // call re-reads the database (which does not exist) and starts clean.
            self.forget_health(install_id);
            return Ok(true);
        };
        let mut tx = pool.begin().await.map_err(health_error)?;
        core_db::set_org(&mut tx, org_id)
            .await
            .map_err(|error| PoolError::Health(error.to_string()))?;
        let cleared = core_db::plugin_health::clear_quarantine(&mut tx, install_id, cleared_by)
            .await
            .map_err(|error| PoolError::Health(error.to_string()))?;
        tx.commit().await.map_err(health_error)?;
        self.forget_health(install_id);
        Ok(cleared)
    }

    /// The quarantine state an operator surface renders (W53-8). `None` means
    /// no worker failure has ever been recorded for the install.
    pub async fn quarantine_state(
        &self,
        org_id: Uuid,
        install_id: Uuid,
    ) -> Result<Option<core_db::plugin_health::InstallHealth>, PoolError> {
        let Some(pool) = self.inner.config.health.clone() else {
            return Ok(None);
        };
        let mut tx = pool.begin().await.map_err(health_error)?;
        core_db::set_org(&mut tx, org_id)
            .await
            .map_err(|error| PoolError::Health(error.to_string()))?;
        let row = core_db::plugin_health::load(&mut tx, install_id)
            .await
            .map_err(|error| PoolError::Health(error.to_string()))?;
        tx.commit().await.map_err(health_error)?;
        Ok(row)
    }

    async fn run_invocation(
        &self,
        request: &InvocationRequest,
        artifact: &ModuleArtifact,
        service: &mut dyn HostCallService,
    ) -> InvocationResult {
        let started = Instant::now();
        let invocation_id = request.invocation_id;

        // The outermost gate: admin-disabled means nothing executes.
        if !self.enabled() {
            return InvocationResult::refused(
                invocation_id,
                Refusal::RuntimeDisabled,
                elapsed_us(started),
            );
        }
        if let Err(refusal) = request.validate() {
            return InvocationResult::refused(invocation_id, refusal, elapsed_us(started));
        }
        if !is_sha256_hex(&artifact.digest) {
            return InvocationResult::refused(
                invocation_id,
                Refusal::FrameInvalid,
                elapsed_us(started),
            );
        }

        let org_id = request.org_id;
        let install_id = request.install_id;

        if self.inner.quarantined(org_id, install_id).await {
            return InvocationResult::refused(
                invocation_id,
                Refusal::Quarantined,
                elapsed_us(started),
            );
        }
        if !self.inner.take_rate_token(install_id) {
            return InvocationResult::refused(
                invocation_id,
                Refusal::RateLimited,
                elapsed_us(started),
            );
        }

        let org_slot = self.inner.org_slot(org_id);
        let Some(_org_permit) = acquire(org_slot).await else {
            return InvocationResult::refused(
                invocation_id,
                Refusal::ConcurrencyLimited,
                elapsed_us(started),
            );
        };
        let install_slot = self.inner.install_slot(install_id);
        let Some(_install_permit) = acquire(install_slot).await else {
            return InvocationResult::refused(
                invocation_id,
                Refusal::ConcurrencyLimited,
                elapsed_us(started),
            );
        };

        let result = match self.inner.config.mode {
            IsolationMode::InProcess => {
                self.invoke_in_process(request, artifact, invocation_id, started)
                    .await
            }
            _ => {
                self.invoke_in_pool(request, artifact, invocation_id, started, service)
                    .await
            }
        };

        // The health counter is about WORKER HEALTH, not about the plugin's
        // answer: a guest trap or a refusal means the worker did its job and
        // survived it, and only a dead or lying worker counts against the
        // install.
        if result.reply.outcome == InvocationOutcome::WorkerFailed {
            self.inner.note_worker_failure(org_id, install_id).await;
        } else {
            self.inner.note_worker_survived(org_id, install_id).await;
        }
        result
    }

    async fn invoke_in_pool(
        &self,
        request: &InvocationRequest,
        artifact: &ModuleArtifact,
        invocation_id: Uuid,
        started: Instant,
        service: &mut dyn HostCallService,
    ) -> InvocationResult {
        let permit = match tokio::time::timeout(
            CONCURRENCY_WAIT,
            Arc::clone(&self.inner.permits).acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            _ => {
                return InvocationResult::refused(
                    invocation_id,
                    Refusal::ConcurrencyLimited,
                    elapsed_us(started),
                )
            }
        };

        let mut member = match self.inner.checkout().await {
            Ok(member) => member,
            Err(error) => {
                tracing::warn!(error = %error, "plugin worker unavailable");
                return worker_failed(invocation_id, None, started);
            }
        };
        let worker = member.label();
        let outcome = member.invoke(request, artifact, service).await;
        drop(permit);

        match outcome {
            Ok(frame) => {
                let result = InvocationResult {
                    reply: frame.reply,
                    worker: Some(worker),
                };
                if frame.retire {
                    // The worker answered, then said it is over a ceiling. Its
                    // answer stands; the member does not.
                    member.kill().await;
                } else {
                    self.inner.checkin(member);
                }
                result
            }
            Err(error) => {
                tracing::warn!(
                    worker = %worker,
                    detail = %error,
                    "plugin worker failed; the invocation is recorded as worker_failed and the worker is replaced"
                );
                member.kill().await;
                worker_failed(invocation_id, Some(worker), started)
            }
        }
    }

    async fn invoke_in_process(
        &self,
        request: &InvocationRequest,
        artifact: &ModuleArtifact,
        invocation_id: Uuid,
        started: Instant,
    ) -> InvocationResult {
        let Some(sandbox) = self.inner.sandbox.clone() else {
            return worker_failed(invocation_id, None, started);
        };
        let log_context = (request.org_id, request.install_id, request.invocation_id);
        let request = request.clone();
        let artifact = artifact.clone();
        let worker = "in-process".to_string();

        // wasmtime execution is synchronous: running it on the reactor would
        // stop every other request for the duration of one guest.
        let join = tokio::task::spawn_blocking(move || {
            let module = sandbox
                .load(&artifact)
                .map_err(|error| (Refusal::FrameInvalid, 0, Some(error.to_string())))?;
            sandbox
                .run(&module, &request)
                .map_err(|failure| (failure.refusal, failure.fuel_used, None))
        });

        match tokio::time::timeout(INVOCATION_TIMEOUT, join).await {
            Ok(Ok(Ok(output))) => {
                forward_logs(log_context, &output.logs);
                InvocationResult {
                    reply: InvocationReply::ok(
                        invocation_id,
                        output.output,
                        output.fuel_used,
                        elapsed_us(started),
                    ),
                    worker: Some(worker),
                }
            }
            Ok(Ok(Err((refusal, fuel_used, detail)))) => {
                if let Some(detail) = detail {
                    tracing::warn!(detail, "in-process plugin module rejected");
                }
                InvocationResult {
                    reply: InvocationReply::refused(
                        invocation_id,
                        refusal,
                        fuel_used,
                        elapsed_us(started),
                    ),
                    worker: Some(worker),
                }
            }
            Ok(Err(join_error)) => {
                tracing::warn!(error = %join_error, "in-process plugin task panicked");
                worker_failed(invocation_id, Some(worker), started)
            }
            Err(_) => {
                // The blocking thread cannot be cancelled. It is still bounded
                // by the sandbox's own epoch deadline; this is the backstop for
                // a sandbox bug, and it leaks a thread until the guest returns.
                tracing::warn!("in-process plugin invocation exceeded the restart threshold");
                worker_failed(invocation_id, Some(worker), started)
            }
        }
    }

    fn forget_health(&self, install_id: Uuid) {
        let mut gates = self
            .inner
            .gates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(gate) = gates.installs.get_mut(&install_id) {
            gate.quarantined = false;
            gate.failures = 0;
            gate.health_loaded_at = None;
        }
    }
}

impl PluginRuntime for PluginPool {
    const KIND: &'static str = "wasm";

    fn id(&self) -> Uuid {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn enabled(&self) -> bool {
        self.inner.config.mode != IsolationMode::Disabled
    }

    async fn invoke(
        &self,
        request: &InvocationRequest,
        artifact: &ModuleArtifact,
    ) -> InvocationResult {
        self.run_invocation(request, artifact, &mut NoHostCalls)
            .await
    }
}

/// Writes a guest's captured `log` lines into this process's log stream. The
/// worker process calls this too, so a line looks the same whichever transport
/// ran the invocation. The lines never travel in a reply frame: 100 lines of
/// 4 KiB would be 400 KiB of frame for an audit row that carries none of it.
pub fn forward_logs(context: (Uuid, Uuid, Uuid), lines: &[LogLine]) {
    let (org_id, install_id, invocation_id) = context;
    for line in lines {
        let message = line.message.as_str();
        match line.level {
            1 => tracing::error!(
                %org_id, %install_id, %invocation_id, message,
                "plugin log"
            ),
            2 => tracing::warn!(
                %org_id, %install_id, %invocation_id, message,
                "plugin log"
            ),
            3 => tracing::info!(
                %org_id, %install_id, %invocation_id, message,
                "plugin log"
            ),
            _ => tracing::debug!(
                %org_id, %install_id, %invocation_id, message,
                "plugin log"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// One worker process
// ---------------------------------------------------------------------------

/// A live worker: its pipes, and what this parent has already sent it.
struct Member {
    pid: u32,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    /// Digests this member has acknowledged caching. Cleared when the member is
    /// replaced, which is what makes the next frame carry the bytes again.
    loaded: HashSet<String>,
}

impl Member {
    /// The audit row's `worker` detail: stable for the life of the member.
    fn label(&self) -> String {
        format!("{WORKER_BIN_NAME}:{}", self.pid)
    }

    /// One request, one reply — alternating with the guest's host calls.
    ///
    /// The parent writes the invocation frame, then reads: a host-call frame is
    /// served and answered, and the loop continues until the invocation's reply
    /// arrives. Any I/O error, protocol error, echo mismatch or reply that fails
    /// [`InvocationReply::validate`] is `Err`: the caller kills this member and
    /// answers the invocation with `worker_failed`.
    ///
    /// Both host-call correlators are checked against this invocation before a
    /// byte of store work happens: the invocation id must be the one just
    /// dispatched, and the call number must be the next one owed. A duplicate,
    /// a replay or a frame from another invocation therefore fails the
    /// invocation instead of being answered.
    async fn invoke(
        &mut self,
        request: &InvocationRequest,
        artifact: &ModuleArtifact,
        service: &mut dyn HostCallService,
    ) -> Result<ReplyFrame, String> {
        let send_module = !self.loaded.contains(&artifact.digest);
        let frame = InvokeFrame {
            digest: artifact.digest.clone(),
            module: send_module.then(|| artifact.bytes.clone()),
            request: request.clone(),
        };
        let bytes = wire::encode_invoke(&frame).map_err(|error| format!("encode: {error}"))?;
        // A frame that carries a module also pays for a compile, which is host
        // work and is not charged against the invocation's own clock.
        let deadline = if send_module {
            LOAD_TIMEOUT
        } else {
            INVOCATION_TIMEOUT
        };
        let context = HostCallContext {
            invocation_id: request.invocation_id,
            // The tenant comes from the frame the PARENT built, never from the
            // host-call frames the worker sends back.
            org_id: request.org_id,
            install_id: request.install_id,
        };

        let exchange = async {
            self.stdin
                .write_all(&bytes)
                .await
                .map_err(|error| format!("write: {error}"))?;
            self.stdin
                .flush()
                .await
                .map_err(|error| format!("flush: {error}"))?;

            let mut served: u64 = 0;
            loop {
                let body = read_body(&mut self.stdout)
                    .await
                    .map_err(|error| format!("read: {error}"))?
                    .ok_or_else(|| "worker closed its stdout".to_string())?;
                match wire::frame_kind(&body).map_err(|error| format!("frame: {error}"))? {
                    wire::KIND_REPLY => {
                        return wire::decode_reply(&body)
                            .map_err(|error| format!("decode: {error}"))
                    }
                    wire::KIND_HOST_CALL => {
                        let call = wire::decode_host_call(&body)
                            .map_err(|error| format!("host call: {error}"))?;
                        if call.invocation_id != context.invocation_id {
                            return Err("host call names another invocation".to_string());
                        }
                        if call.call_id != served {
                            return Err(format!(
                                "host call {served} expected, {} arrived",
                                call.call_id
                            ));
                        }
                        let outcome = if served >= MAX_HOST_CALLS_PER_INVOCATION {
                            HostCallOutcome::Refused(wire::HostCallRefusal::BudgetExhausted)
                        } else {
                            service.serve(context, call.op).await
                        };
                        served += 1;
                        let reply = wire::encode_host_reply(&HostReplyFrame {
                            invocation_id: context.invocation_id,
                            call_id: call.call_id,
                            outcome,
                        })
                        .map_err(|error| format!("host reply: {error}"))?;
                        self.stdin
                            .write_all(&reply)
                            .await
                            .map_err(|error| format!("host reply write: {error}"))?;
                        self.stdin
                            .flush()
                            .await
                            .map_err(|error| format!("host reply flush: {error}"))?;
                    }
                    other => {
                        return Err(format!("frame kind {other} mid-invocation"));
                    }
                }
            }
        };

        let reply = match tokio::time::timeout(deadline, exchange).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(detail)) => return Err(detail),
            Err(_) => return Err(format!("no reply within {} ms", deadline.as_millis())),
        };

        // The reply must be the answer to THIS invocation: a stale frame from a
        // previous call would otherwise be written into another install's audit
        // row.
        if reply.reply.invocation_id != request.invocation_id {
            return Err("reply echoes another invocation id".to_string());
        }
        if let Err(refusal) = reply.reply.validate() {
            return Err(format!(
                "reply contradicts its outcome ({})",
                refusal.as_str()
            ));
        }
        if reply.reply.outcome == InvocationOutcome::WorkerFailed {
            // Only the parent writes a worker_failed outcome. A worker that
            // sends one is not reporting its own death, it is lying about it.
            return Err("worker sent a worker_failed outcome".to_string());
        }
        if reply.module_cached {
            self.loaded.insert(artifact.digest.clone());
        } else {
            self.loaded.remove(&artifact.digest);
        }
        // The worker told us what its cache pushed out. Forgetting it here is
        // what keeps the next invocation for that digest carrying its bytes
        // instead of being refused for a module the worker no longer has.
        if let Some(evicted) = &reply.evicted {
            self.loaded.remove(evicted);
        }
        Ok(reply)
    }

    async fn kill(mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

/// Reads one length-prefixed body. `Ok(None)` is a clean EOF between frames — a
/// worker that exited or was retired, which is not an error in itself.
async fn read_body<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<Option<Vec<u8>>, String> {
    let mut len = [0u8; 4];
    match reader.read_exact(&mut len).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(format!("length prefix: {error}")),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > wire::MAX_FRAME_BYTES {
        return Err(format!(
            "frame of {len} bytes is above the {}-byte ceiling",
            wire::MAX_FRAME_BYTES
        ));
    }
    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|error| format!("body: {error}"))?;
    Ok(Some(body))
}

fn resolve_worker_binary(config: &PoolConfig) -> Result<PathBuf, PoolError> {
    if let Some(path) = &config.worker_bin {
        if path.is_file() {
            return Ok(path.clone());
        }
        return Err(PoolError::WorkerUnavailable {
            bin: path.display().to_string(),
            detail: "no such file".to_string(),
        });
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(format!("{WORKER_BIN_NAME}{}", std::env::consts::EXE_SUFFIX));
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    // Last resort: PATH. The image copies every role into one bin directory, so
    // the sibling lookup above already resolves there.
    Ok(PathBuf::from(WORKER_BIN_NAME))
}

/// Spawns one worker and proves it speaks this protocol before it is handed any
/// work.
async fn spawn_worker(bin: &Path) -> Result<Member, PoolError> {
    let mut child = Command::new(bin)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        // The worker writes its diagnostics to stderr and they belong in the
        // deployment's log. Inheriting avoids a pipe nobody drains, which would
        // eventually block the worker mid-log-line.
        .stderr(std::process::Stdio::inherit())
        // A parent that panics must not leave an orphan running guest code.
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| PoolError::WorkerUnavailable {
            bin: bin.display().to_string(),
            detail: error.to_string(),
        })?;

    let pid = child.id().unwrap_or_default();
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| PoolError::WorkerUnavailable {
            bin: bin.display().to_string(),
            detail: "no stdin pipe".to_string(),
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| PoolError::WorkerUnavailable {
            bin: bin.display().to_string(),
            detail: "no stdout pipe".to_string(),
        })?;
    let mut member = Member {
        pid,
        child,
        stdin,
        stdout: BufReader::new(stdout),
        loaded: HashSet::new(),
    };

    // The worker announces itself first. A binary that is not ours (an old
    // build, a shell script, the wrong file) fails here at boot rather than on
    // the first user request.
    let hello = tokio::time::timeout(LOAD_TIMEOUT, read_body(&mut member.stdout)).await;
    match hello {
        Ok(Ok(Some(body))) => match wire::decode_hello(&body) {
            Ok(worker_pid) => {
                if worker_pid != pid {
                    tracing::debug!(
                        reported = worker_pid,
                        actual = pid,
                        "plugin worker reported another pid"
                    );
                }
                Ok(member)
            }
            Err(error) => Err(PoolError::WorkerUnavailable {
                bin: bin.display().to_string(),
                detail: format!(
                    "does not speak protocol v{}: {error}",
                    wire::PROTOCOL_VERSION
                ),
            }),
        },
        Ok(Ok(None)) => Err(PoolError::WorkerUnavailable {
            bin: bin.display().to_string(),
            detail: "exited before its hello frame".to_string(),
        }),
        Ok(Err(detail)) => Err(PoolError::WorkerUnavailable {
            bin: bin.display().to_string(),
            detail,
        }),
        Err(_) => Err(PoolError::WorkerUnavailable {
            bin: bin.display().to_string(),
            detail: "no hello frame".to_string(),
        }),
    }
}

impl Inner {
    async fn checkout(&self) -> Result<Member, PoolError> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(PoolError::Config("the pool is shutting down".to_string()));
        }
        let idle = {
            let mut idle = self
                .idle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            idle.pop()
        };
        match idle {
            Some(member) => Ok(member),
            None => spawn_worker(&self.worker_bin).await,
        }
    }

    fn checkin(&self, member: Member) {
        if self.shutting_down.load(Ordering::SeqCst) {
            // A shutdown raced this invocation. The child dies with the Member's
            // drop; do not park it in a pool nobody will use again.
            drop(member);
            return;
        }
        let mut idle = self
            .idle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Never exceed the configured count: a retiring member frees its slot.
        if idle.len() < self.config.workers {
            idle.push(member);
        }
    }
}

// ---------------------------------------------------------------------------
// The gates
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Gates {
    installs: HashMap<Uuid, InstallGate>,
    orgs: HashMap<Uuid, Arc<Semaphore>>,
}

/// One install's gate row, created on first sight. One constructor, so a new
/// field cannot be added to two of the three call sites and forgotten in the
/// third.
fn install_entry(gates: &mut Gates, install_id: Uuid) -> &mut InstallGate {
    gates
        .installs
        .entry(install_id)
        .or_insert_with(|| InstallGate {
            bucket: Bucket::new(),
            // One invocation per install (Decision 4): a plugin's own
            // read-modify-write on its KV cannot interleave with itself.
            semaphore: Arc::new(Semaphore::new(1)),
            failures: 0,
            quarantined: false,
            health_loaded_at: None,
        })
}

struct InstallGate {
    bucket: Bucket,
    semaphore: Arc<Semaphore>,
    /// Consecutive worker failures seen by THIS process since the last
    /// successful invocation. Mirrors the database row between reloads.
    failures: u32,
    quarantined: bool,
    health_loaded_at: Option<Instant>,
}

/// A token bucket: 300 per minute means a refill of 5 per second, and a burst of
/// 60 means a script that fires 60 calls at once is served rather than throttled
/// for no reason.
struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn new() -> Self {
        Bucket {
            tokens: abi::INVOCATION_BURST_PER_INSTALL as f64,
            at: Instant::now(),
        }
    }

    fn refill_per_second() -> f64 {
        abi::INVOCATIONS_PER_MINUTE_PER_INSTALL as f64 / 60.0
    }

    fn take(&mut self) -> bool {
        let capacity = abi::INVOCATION_BURST_PER_INSTALL as f64;
        let elapsed = self.at.elapsed().as_secs_f64();
        self.tokens = (self.tokens + elapsed * Self::refill_per_second()).min(capacity);
        self.at = Instant::now();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn is_full(&self) -> bool {
        self.tokens >= abi::INVOCATION_BURST_PER_INSTALL as f64
    }
}

impl Inner {
    fn install_slot(&self, install_id: Uuid) -> Arc<Semaphore> {
        let mut gates = self
            .gates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Opportunistic pruning, and only of buckets that are full: a drained
        // bucket IS the rate limit, so evicting it would hand the install a new
        // burst.
        if gates.installs.len() > GATE_TABLE_SOFT_LIMIT {
            gates
                .installs
                .retain(|_, gate| Arc::strong_count(&gate.semaphore) > 1 || !gate.bucket.is_full());
        }
        Arc::clone(&install_entry(&mut gates, install_id).semaphore)
    }

    fn org_slot(&self, org_id: Uuid) -> Arc<Semaphore> {
        let mut gates = self
            .gates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if gates.orgs.len() > GATE_TABLE_SOFT_LIMIT {
            gates.orgs.retain(|_, semaphore| {
                Arc::strong_count(semaphore) > 1 || semaphore.available_permits() > 0
            });
        }
        Arc::clone(
            gates
                .orgs
                .entry(org_id)
                // One invocation per org (Decision 4): a second org always has
                // a worker.
                .or_insert_with(|| Arc::new(Semaphore::new(1))),
        )
    }

    fn take_rate_token(&self, install_id: Uuid) -> bool {
        let mut gates = self
            .gates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        install_entry(&mut gates, install_id).bucket.take()
    }

    /// Whether the install is quarantined, reading the database at most once per
    /// [`HEALTH_CACHE_TTL`] per install. Without a database the answer comes
    /// from this process's own counter, which a restart clears — stated in
    /// [`PluginPool`]'s documentation rather than hidden.
    async fn quarantined(&self, org_id: Uuid, install_id: Uuid) -> bool {
        let stale = {
            let gates = self
                .gates
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match gates.installs.get(&install_id) {
                Some(gate) => match gate.health_loaded_at {
                    Some(at) => at.elapsed() >= HEALTH_CACHE_TTL,
                    None => true,
                },
                None => true,
            }
        };
        if stale {
            self.reload_health(org_id, install_id).await;
        }
        let gates = self
            .gates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        gates
            .installs
            .get(&install_id)
            .map(|gate| gate.quarantined || gate.failures >= QUARANTINE_AFTER_FAILURES)
            .unwrap_or(false)
    }

    async fn reload_health(&self, org_id: Uuid, install_id: Uuid) {
        let Some(pool) = self.config.health.clone() else {
            self.mark_health_loaded(install_id, None);
            return;
        };
        let row = match load_health(&pool, org_id, install_id).await {
            Ok(row) => row,
            Err(error) => {
                // The database is unreachable; core-api is failing more than
                // plugin calls. Keep the in-process view and say so.
                tracing::warn!(error = %error, "plugin health read failed; using the in-process counter");
                self.mark_health_loaded(install_id, None);
                return;
            }
        };
        self.mark_health_loaded(install_id, row);
    }

    fn mark_health_loaded(
        &self,
        install_id: Uuid,
        row: Option<core_db::plugin_health::InstallHealth>,
    ) {
        let mut gates = self
            .gates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let gate = install_entry(&mut gates, install_id);
        gate.health_loaded_at = Some(Instant::now());
        if let Some(row) = row {
            gate.failures = u32::try_from(row.consecutive_failures).unwrap_or(0);
            // `quarantined_at` is the durable fact; a clear sets it back to NULL.
            gate.quarantined = row.is_quarantined();
        }
    }

    async fn note_worker_failure(&self, org_id: Uuid, install_id: Uuid) {
        let failures = {
            let mut gates = self
                .gates
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let gate = install_entry(&mut gates, install_id);
            gate.failures = gate.failures.saturating_add(1);
            gate.failures
        };
        if failures >= QUARANTINE_AFTER_FAILURES {
            tracing::error!(
                %org_id,
                %install_id,
                failures,
                "plugin install quarantined after repeated worker failures"
            );
        }

        let Some(pool) = self.config.health.clone() else {
            return;
        };
        // The failure path, not the happy path: one transaction, one row write.
        let result = async {
            let mut tx = pool.begin().await.map_err(health_error)?;
            core_db::set_org(&mut tx, org_id)
                .await
                .map_err(|error| PoolError::Health(error.to_string()))?;
            let row = core_db::plugin_health::record_failure(
                &mut tx,
                install_id,
                Refusal::WorkerFailed.as_str(),
                QUARANTINE_AFTER_FAILURES as i32,
            )
            .await
            .map_err(|error| PoolError::Health(error.to_string()))?;
            tx.commit().await.map_err(health_error)?;
            Ok::<_, PoolError>(row)
        }
        .await;

        match result {
            Ok(Some(row)) => {
                let mut gates = self
                    .gates
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if let Some(gate) = gates.installs.get_mut(&install_id) {
                    gate.quarantined = row.is_quarantined();
                    gate.failures =
                        u32::try_from(row.consecutive_failures).unwrap_or(gate.failures);
                }
            }
            Ok(None) => {
                tracing::warn!(
                    %install_id,
                    "worker failure not recorded: the install is not visible in this tenant scope"
                );
            }
            Err(error) => {
                tracing::warn!(error = %error, "plugin worker failure was not persisted");
            }
        }
    }

    async fn note_worker_survived(&self, org_id: Uuid, install_id: Uuid) {
        let had_failures = {
            let mut gates = self
                .gates
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match gates.installs.get_mut(&install_id) {
                Some(gate) if gate.failures > 0 => {
                    gate.failures = 0;
                    true
                }
                _ => false,
            }
        };
        // No write on the happy path: the reset is persisted only when there was
        // something to reset.
        if !had_failures {
            return;
        }
        let Some(pool) = self.config.health.clone() else {
            return;
        };
        let result = async {
            let mut tx = pool.begin().await.map_err(health_error)?;
            core_db::set_org(&mut tx, org_id)
                .await
                .map_err(|error| PoolError::Health(error.to_string()))?;
            core_db::plugin_health::record_success(&mut tx, install_id)
                .await
                .map_err(|error| PoolError::Health(error.to_string()))?;
            tx.commit().await.map_err(health_error)?;
            Ok::<_, PoolError>(())
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(error = %error, "plugin worker success was not persisted");
        }
    }
}

async fn acquire(semaphore: Arc<Semaphore>) -> Option<OwnedSemaphorePermit> {
    match tokio::time::timeout(CONCURRENCY_WAIT, semaphore.acquire_owned()).await {
        Ok(Ok(permit)) => Some(permit),
        _ => None,
    }
}

async fn load_health(
    pool: &sqlx::PgPool,
    org_id: Uuid,
    install_id: Uuid,
) -> Result<Option<core_db::plugin_health::InstallHealth>, PoolError> {
    let mut tx = pool.begin().await.map_err(health_error)?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(|error| PoolError::Health(error.to_string()))?;
    let row = core_db::plugin_health::load(&mut tx, install_id)
        .await
        .map_err(|error| PoolError::Health(error.to_string()))?;
    tx.commit().await.map_err(health_error)?;
    Ok(row)
}

fn health_error(error: sqlx::Error) -> PoolError {
    PoolError::Health(error.to_string())
}

fn elapsed_us(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

fn worker_failed(
    invocation_id: Uuid,
    worker: Option<String>,
    started: Instant,
) -> InvocationResult {
    InvocationResult {
        reply: InvocationReply::worker_failed(invocation_id, elapsed_us(started)),
        worker,
    }
}

/// A digest is the sha256 hex `distribution::sha256_hex` produces. Refusing
/// anything else before dispatch means a caller cannot smuggle a path, a URL or
/// an empty string past the artifact lookup.
fn is_sha256_hex(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The worker's own resident set, in bytes, when the platform exposes it.
///
/// Linux only: `/proc/self/status` is one small file and every shipped
/// deployment is a Linux container. On macOS the function answers `None` and the
/// resident ceiling is not enforced — the ADR's ceiling is a container concern,
/// and the sandbox's own `StoreLimits` still bounds the instance.
#[cfg(target_os = "linux")]
pub fn resident_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kilobytes: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kilobytes.saturating_mul(1024));
        }
    }
    None
}

/// See the Linux implementation. Other platforms answer `None`.
#[cfg(not(target_os = "linux"))]
pub fn resident_bytes() -> Option<u64> {
    None
}

/// Whether a worker should retire itself: its resident set is above ADR 0023
/// Decision 4's ceiling. The worker answers the invocation in flight and then
/// exits, so no call is lost to the recycle.
pub fn over_resident_ceiling() -> bool {
    resident_bytes()
        .map(|bytes| bytes > abi::WORKER_RESIDENT_MAX_BYTES)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_inner() -> Inner {
        Inner {
            config: PoolConfig::default(),
            sandbox: None,
            worker_bin: PathBuf::from(WORKER_BIN_NAME),
            permits: Arc::new(Semaphore::new(1)),
            idle: Mutex::new(Vec::new()),
            gates: Mutex::new(Gates::default()),
            shutting_down: AtomicBool::new(false),
        }
    }

    #[test]
    fn the_isolation_knob_parses_the_documented_values_and_refuses_the_rest() {
        assert_eq!(IsolationMode::parse("pool"), Some(IsolationMode::Pool));
        assert_eq!(IsolationMode::parse(""), Some(IsolationMode::Pool));
        assert_eq!(
            IsolationMode::parse("in_process"),
            Some(IsolationMode::InProcess)
        );
        assert_eq!(
            IsolationMode::parse("in-process"),
            Some(IsolationMode::InProcess)
        );
        assert_eq!(
            IsolationMode::parse("disabled"),
            Some(IsolationMode::Disabled)
        );
        assert_eq!(IsolationMode::parse("Pool"), Some(IsolationMode::Pool));
        assert_eq!(IsolationMode::parse("extism"), None);
        assert_eq!(IsolationMode::parse("pool "), Some(IsolationMode::Pool));
    }

    #[test]
    fn a_bucket_serves_its_burst_then_refuses_and_refills_at_five_per_second() {
        let mut bucket = Bucket::new();
        for served in 0..abi::INVOCATION_BURST_PER_INSTALL {
            assert!(bucket.take(), "call {served} is inside the burst");
        }
        assert!(!bucket.take(), "the burst is spent");
        assert!(!bucket.is_full());

        // One second of refill is 300/60 = 5 tokens.
        bucket.at = Instant::now() - Duration::from_secs(1);
        for refilled in 0..5 {
            assert!(bucket.take(), "token {refilled} refilled in one second");
        }
        assert!(!bucket.take());
    }

    #[tokio::test]
    async fn three_consecutive_worker_failures_quarantine_without_a_database() {
        let inner = test_inner();
        let org_id = Uuid::now_v7();
        let install_id = Uuid::now_v7();

        for _ in 0..QUARANTINE_AFTER_FAILURES - 1 {
            inner.note_worker_failure(org_id, install_id).await;
        }
        assert!(
            !inner.quarantined(org_id, install_id).await,
            "two failures are still a flake"
        );

        inner.note_worker_failure(org_id, install_id).await;
        assert!(
            inner.quarantined(org_id, install_id).await,
            "the third consecutive failure quarantines"
        );

        // A worker that survives an invocation is what makes the failures
        // CONSECUTIVE rather than cumulative.
        inner.note_worker_survived(org_id, install_id).await;
        assert!(!inner.quarantined(org_id, install_id).await);
    }

    #[tokio::test]
    async fn quarantine_is_per_install_and_does_not_leak_to_another() {
        let inner = test_inner();
        let org_id = Uuid::now_v7();
        let doomed = Uuid::now_v7();
        let healthy = Uuid::now_v7();
        for _ in 0..QUARANTINE_AFTER_FAILURES {
            inner.note_worker_failure(org_id, doomed).await;
        }
        assert!(inner.quarantined(org_id, doomed).await);
        assert!(!inner.quarantined(org_id, healthy).await);
    }

    #[test]
    fn only_a_lowercase_sha256_hex_string_passes_as_a_digest() {
        assert!(is_sha256_hex(&"a".repeat(64)));
        assert!(is_sha256_hex(&"0".repeat(64)));
        assert!(!is_sha256_hex(""));
        assert!(!is_sha256_hex(&"a".repeat(63)));
        assert!(!is_sha256_hex(&"a".repeat(65)));
        assert!(!is_sha256_hex(&"A".repeat(64)));
        assert!(!is_sha256_hex(&"g".repeat(64)));
        assert!(!is_sha256_hex("../../etc/passwd"));
    }

    #[test]
    fn the_wait_budget_and_the_restart_threshold_are_the_adr_numbers() {
        assert_eq!(CONCURRENCY_WAIT, Duration::from_millis(abi::WALL_CLOCK_MS));
        assert_eq!(
            INVOCATION_TIMEOUT,
            Duration::from_millis(abi::WORKER_RESTART_THRESHOLD_MS)
        );
        assert!(LOAD_TIMEOUT > INVOCATION_TIMEOUT);
        assert_eq!(QUARANTINE_AFTER_FAILURES, 3);
    }
}
