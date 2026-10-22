//! The request path: turning an authorized invocation into an effect (W53-5).
//!
//! One function, [`invoke_surface`], is the whole path: a surface entry the
//! registry knows about, the install that owns it, the capability the surface
//! requires, the module the install pinned, the worker that runs it, and the
//! audit row that records the attempt. Every surface goes through it —
//! [`SurfaceKind`] is the closed list of six — so a widget, a report render, a
//! field validator, an action, a trigger, a read node and a workflow node are
//! callers of one mechanism, not six mechanisms.
//!
//! ## What may and may not enter the request
//!
//! The [`abi::InvocationRequest`] is built from
//! [`crate::authorize_invocation`]'s own answer. `granted` is
//! `install.granted_capabilities` and nothing else: not the payload, not the
//! entry definition, not the worker's frame, not anything a guest returned.
//! The payload, the entry key and the ticket id come from the caller; the grant
//! set, the declared event types and the egress profile do not. The pool
//! re-validates every grant key against the closed vocabulary, so a caller bug
//! there fails closed. `host_fns::InvocationState::grant()` is reachable only
//! from the worker's own frame parsing and from tests; this module never calls
//! it.
//!
//! ## The audit row (ADR 0023 Decision 7)
//!
//! One `audit_events` row per attempt, written here, by the parent, on the
//! caller's connection, awaited before the caller answers. The shape is
//! [`audit_event`]: `actor = Actor::Plugin(<install>)`, `action =
//! plugin.invoke`, `entity_type = plugin_entry`, `entity_id = <entry key>`, and
//! a `detail` carrying exactly `outcome`, `reason`, `capability`,
//! `invocation_id`, `fuel_used`, `duration_us` and `worker`.
//!
//! - A refusal decided before dispatch is the same row with `outcome =
//!   "refused"` and `invocation_id = null`.
//! - A dead worker is `outcome = "worker_failed"`, `reason =
//!   "plugin_worker_failed"`, `fuel_used = null`.
//! - A refusal that cannot name an install — the registry row is gone, so no
//!   install is resolvable — is attributed to `system` with a null actor id.
//!   The entry key is caller-supplied, so it may never choose the actor.
//!
//! ## The transaction
//!
//! The row and the effect share the caller's transaction: a rollback drops
//! both. The limit is ADR 0023's and is stated rather than hidden — a core-api
//! crash between the worker's reply and the insert loses the row and the effect
//! together. The pool's concurrency slots (one invocation per org, one per
//! install) bound how many request transactions sit open across a worker call.
//!
//! ## The pool, and the declarative path
//!
//! The caller's [`PoolSlot`] starts the pool once, on the first module-bearing
//! invocation, and reuses it; the slot belongs to the process's `AppState`, not
//! to a `static`, so it can be shut down and two states never share one.
//! `MARULA_PLUGIN_ISOLATION=disabled` turns every such invocation into a
//! `plugin_runtime_disabled` refusal WITH a row, which is what a deployment
//! without the worker binary gets; nothing falls back to running the effect
//! without the module.
//!
//! ## The guest's own host calls
//!
//! A guest's `kv_get` reaches this module as a host-call frame on the worker's
//! pipe, and [`TransactionHostCalls`] answers it on the transaction this
//! function already holds. That is the only place the two identities a KV row
//! needs can come from: the install row this invocation was authorized against,
//! never the frame (which has no field for them). See
//! [`TransactionHostCalls`] for the connection choice and its price.
//!
//! ## A fault is not a refusal
//!
//! [`SurfaceOutcome::is_fault`] is the distinction callers act on. A refusal is
//! the plugin saying no: policy, consent, validation, a schema, the guest
//! declining its own effect. The mechanism worked, so the node is a skip. A
//! FAULT is the mechanism failing — the worker died, the transport broke, the
//! reply never came, the pool could not start — and a caller must fail the node
//! rather than record an outage as a decision.
//!
//! The attempt's audit row is still written here, once, for either. What the
//! caller's refusal does to it is ADR 0023's documented transaction rule: the
//! row and the effect commit or roll back together, so a fault — which the
//! caller answers with a 5xx — takes its row with it and the retry writes its
//! own. The durable record of a worker death does not depend on that row: the
//! pool's health counter is written outside the request's transaction, on
//! purpose, so a rolled-back request cannot roll back the fact that the worker
//! died.
//!
//! An install with no module pin is a declarative install: `enable` allows it
//! (a surface/connector/settings plugin has no module to run), and the
//! consented effect is applied by the parent exactly as core did before this
//! lane. That path is the old behaviour, kept reachable and now audited.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use core_db::plugin_surface::{self, SurfaceEntryRow};
use core_domain::entities::plugins::{PluginInstall, PluginManifestV2, SurfaceActionEffect};
use core_domain::entities::{Actor, AuditEvent, TicketComment};
use core_domain::status::NoteKind;
use core_store::ObjectStore;
use serde_json::{json, Value};
use sqlx::PgConnection;
use tokio::sync::OnceCell;
use uuid::Uuid;

use crate::abi::{self, InvocationOutcome, InvocationRequest};
use crate::lifecycle::{self, ArtifactError};
use crate::runtime::{HostCallContext, HostCallService, PluginPool, PoolConfig, PoolError};
use crate::surface::{self, SurfaceKind};
use crate::wire::{HostCallOp, HostCallOutcome, HostCallRefusal, HostCallValue};

/// The `audit_events.action` every invocation writes.
pub const AUDIT_ACTION: &str = "plugin.invoke";
/// The `audit_events.entity_type` every invocation writes.
pub const AUDIT_ENTITY: &str = "plugin_entry";
/// `detail.worker` on the declarative path: the parent applied the effect, so
/// no worker was reached.
pub const WORKER_DECLARATIVE: &str = "declarative";

/// The body cap the hardcoded effect enforced before this lane, kept so a
/// comment that used to be refused is still refused.
pub const MAX_EFFECT_BODY_BYTES: usize = 4000;

/// The static reasons this path decides before `authorize_invocation` can
/// answer. Same posture as [`crate::denial_reasons`]: a stable string an audit
/// row and an API envelope carry, never prose to parse.
pub mod refusal_reasons {
    /// The registry row is absent: the install is disabled or revoked, the
    /// grant was withdrawn, or the entry itself is disabled.
    pub const ENTRY_UNAVAILABLE: &str = "plugin_entry_unavailable";
    /// The registry row named an install the org no longer has.
    pub const INSTALL_UNAVAILABLE: &str = "plugin_install_unavailable";
    /// The catalog entry or the pinned version row is gone.
    pub const MANIFEST_UNAVAILABLE: &str = "plugin_manifest_unavailable";
    /// The entry's own `params_schema` is not a schema the manifest subset
    /// accepts, so the entry cannot be run safely.
    pub const ENTRY_SCHEMA_INVALID: &str = "plugin_entry_schema_invalid";
    /// The caller's params do not satisfy the entry's `params_schema`, or the
    /// effect's own body bounds.
    pub const PARAMS_REFUSED: &str = "plugin_params_refused";
    /// The entry declares no effect, or one outside the closed vocabulary.
    pub const EFFECT_NOT_CONSENTED: &str = "plugin_effect_not_consented";
    /// The guest answered that it did not accept the call.
    pub const EFFECT_DECLINED: &str = "plugin_effect_declined";
    /// The effect needs a ticket and the call named none.
    pub const EFFECT_TARGET_MISSING: &str = "plugin_effect_target_missing";
    /// Defensive: an attempt that finished without applying an effect a caller
    /// expected one from. The action path cannot reach it.
    pub const NO_EFFECT: &str = "plugin_effect_not_applied";
}

/// The process's plugin pool: started on the first module-bearing invocation
/// and reused.
///
/// **The slot belongs to an `AppState`, not to the process** (W53-11 commit 2).
/// It used to be a `static OnceCell`, which meant a pool could never be shut
/// down and every test in a binary shared one. A state owns its slot, so a test
/// can start and stop a pool, and two states never share one.
///
/// A failed start is not cached: `tokio::sync::OnceCell::get_or_try_init`
/// retries on the next call, which lets a deployment that booted before the
/// worker binary existed recover without a restart. Until it does, each attempt
/// is a typed, audited refusal.
#[derive(Clone, Default)]
pub struct PoolSlot {
    cell: Arc<OnceCell<Arc<PluginPool>>>,
}

impl PoolSlot {
    pub fn new() -> Self {
        PoolSlot::default()
    }

    /// The pool, started once from the environment with `health` as its
    /// quarantine store. `PluginPool::start` is expensive (it spawns and
    /// handshakes its workers), so it must not run per request.
    pub async fn get(&self, health: sqlx::PgPool) -> Result<Arc<PluginPool>, PoolError> {
        self.get_with(|| async move {
            PluginPool::start(PoolConfig::from_env()?.with_health(health)).await
        })
        .await
    }

    /// [`PoolSlot::get`] with an explicit start, which is how a test proves the
    /// lazy retry (a failed start is not cached) without touching the
    /// environment.
    pub async fn get_with<F, Fut>(&self, start: F) -> Result<Arc<PluginPool>, PoolError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Arc<PluginPool>, PoolError>>,
    {
        self.cell.get_or_try_init(start).await.cloned()
    }

    /// Whether a pool has been started in this slot.
    pub fn is_initialized(&self) -> bool {
        self.cell.initialized()
    }

    /// Whether `other` is this same slot. Two `AppState`s answering `false`
    /// have two pools, which is the property this type exists for.
    pub fn is_same_slot(&self, other: &PoolSlot) -> bool {
        Arc::ptr_eq(&self.cell, &other.cell)
    }
}

/// One surface invocation, as a caller asks for it.
///
/// Everything here is the caller's decision about WHAT to invoke. Nothing here
/// is authority: the capability comes from `surface`, and the grant set comes
/// from the install row.
pub struct SurfaceCall<'a> {
    pub org_id: Uuid,
    /// The registry key (`plugin.<slug>.<id>`) the caller wants to run.
    pub entry_key: &'a str,
    pub surface: SurfaceKind,
    /// The task input handed to the guest, and the params the entry's schema
    /// validates. JSON by convention.
    pub params: &'a Value,
    /// The ticket an effect acts on. `None` on a surface that writes nothing.
    pub ticket_id: Option<Uuid>,
}

/// What one attempt produced, for the caller to act on.
///
/// `outcome` is the audit row's word, so a caller that renders a skip and an
/// operator who reads the trail say the same thing.
#[derive(Debug, Clone)]
pub struct SurfaceOutcome {
    pub entry_key: String,
    pub outcome: InvocationOutcome,
    /// The static reason code when the attempt did not apply its effect.
    pub reason: Option<&'static str>,
    /// The internal comment an `add_internal_comment` effect inserted, in the
    /// caller's transaction. `None` when no effect was applied.
    pub comment: Option<TicketComment>,
    /// The guest's own bytes, when it returned a payload. Untrusted: a caller
    /// that renders them validates them first.
    pub output: Option<Vec<u8>>,
}

impl SurfaceOutcome {
    /// The typed reason a workflow node renders as `skipped`. Always present on
    /// a refused attempt; [`refusal_reasons::NO_EFFECT`] is the defensive
    /// answer for a caller that asks about an attempt that applied nothing.
    pub fn skip_reason(&self) -> &'static str {
        self.reason.unwrap_or(refusal_reasons::NO_EFFECT)
    }

    /// Whether this attempt is a FAULT rather than an answer.
    ///
    /// A refusal is the plugin saying no (policy, consent, a schema, the guest
    /// declining its own effect): the mechanism worked, the answer is "no", and
    /// a caller skips the node. A fault is the mechanism failing (the worker
    /// died, the transport broke, the reply never came, the pool could not
    /// start): the effect did not happen because something is broken, and
    /// skipping it would record a broken thing as a decision. Callers must fail
    /// on a fault and skip on a refusal.
    ///
    /// `WorkerFailed` is never produced for a guest that trapped or refused —
    /// those are `Refused` with their own code — so this predicate cannot
    /// mistake a guest's answer for an outage.
    pub fn is_fault(&self) -> bool {
        self.outcome == InvocationOutcome::WorkerFailed
    }
}

/// Runs one surface invocation end to end and writes its audit row.
///
/// Order, and where each gate sits:
///
/// 1. the registry row (`plugin_surface::get_entry_by_key`) — a missing row is
///    a refusal, because the row exists only while the install is enabled, the
///    grant stands and the entry is enabled;
/// 2. the install row, then the catalog entry and the pinned version — the
///    inputs `authorize_invocation` needs;
/// 3. [`crate::authorize_invocation`] — interface version, registry
///    revocation, admin-enabled, grant, matrix resolution;
/// 4. the entry's `params_schema` and, for an action, the consented effect and
///    its ticket;
/// 5. `lifecycle::load_module` — the pin, re-hashed; an unpinned install takes
///    the declarative path instead;
/// 6. [`PluginPool::invoke_with`] — the worker's own gates (enabled, frame, digest,
///    quarantine, rate, concurrency) run there;
/// 7. the effect, from the CONSENTED definition and never from the guest's
///    output, in the caller's transaction;
/// 8. the audit row, once, whatever happened above.
///
/// A database failure is returned as an error (the caller's transaction rolls
/// back); a refusal is a written row and an `Ok` outcome.
pub async fn invoke_surface(
    tx: &mut PgConnection,
    db: &sqlx::PgPool,
    store: &dyn ObjectStore,
    plugins: &PoolSlot,
    call: &SurfaceCall<'_>,
) -> core_db::DbResult<SurfaceOutcome> {
    // No transport: a caller that wants `http_request` to work passes one to
    // `invoke_surface_with`. A guest that asks for the network here is answered
    // `plugin_not_implemented`, which is what a process with nowhere to send
    // owes it.
    let mut none = crate::egress::NoHttpEgress;
    invoke_surface_with(tx, db, store, plugins, &mut none, call).await
}

/// [`invoke_surface`], with the parent-side egress transport the caller owns.
///
/// `http` is what performs a guest's `http_request`: it holds the secret key, the
/// client and the DNS pinning, and it lives in core-api rather than here so this
/// crate — which the worker binary also links — stays free of a network client.
/// The seam is a trait object because the executor cannot name the type that
/// implements it without depending on it.
pub async fn invoke_surface_with(
    tx: &mut PgConnection,
    db: &sqlx::PgPool,
    store: &dyn ObjectStore,
    plugins: &PoolSlot,
    http: &mut dyn crate::egress::HttpEgress,
    call: &SurfaceCall<'_>,
) -> core_db::DbResult<SurfaceOutcome> {
    let started = Instant::now();

    let entry = match plugin_surface::get_entry_by_key(
        &mut *tx,
        call.org_id,
        call.surface.as_str(),
        call.entry_key,
    )
    .await?
    {
        Some(entry) => entry,
        None => return refuse(tx, call, None, refusal_reasons::ENTRY_UNAVAILABLE, started).await,
    };

    let install = match core_db::plugins::get_install(&mut *tx, call.org_id, entry.install_id)
        .await
        .map_err(|error| error.normalized())
    {
        Ok(install) => install,
        Err(core_db::DbError::NotFound) => {
            return refuse(
                tx,
                call,
                None,
                refusal_reasons::INSTALL_UNAVAILABLE,
                started,
            )
            .await
        }
        Err(error) => return Err(error),
    };

    let catalog = match core_db::plugins::get_catalog_entry(&mut *tx, &install.slug)
        .await
        .map_err(|error| error.normalized())
    {
        Ok(catalog) => catalog,
        Err(core_db::DbError::NotFound) => {
            return refuse(
                tx,
                call,
                Some(install.id),
                refusal_reasons::MANIFEST_UNAVAILABLE,
                started,
            )
            .await
        }
        Err(error) => return Err(error),
    };
    let version =
        match core_db::plugins::get_version(&mut *tx, install.catalog_id, &install.version)
            .await
            .map_err(|error| error.normalized())
        {
            Ok(version) => version,
            Err(core_db::DbError::NotFound) => {
                return refuse(
                    tx,
                    call,
                    Some(install.id),
                    refusal_reasons::MANIFEST_UNAVAILABLE,
                    started,
                )
                .await
            }
            Err(error) => return Err(error),
        };

    // The authorization gate. `entry_interface_version` is the version the
    // install ADOPTED (its pinned manifest), not the catalog's latest: an
    // install keeps running the version it consented to.
    if let Err(reason) = crate::authorize_invocation(
        &install,
        catalog.revoked_at.is_some(),
        version.manifest.interface_version,
        call.surface.capability(),
    ) {
        return refuse(tx, call, Some(install.id), reason, started).await;
    }

    match surface::params_schema(&entry.definition) {
        surface::DeclaredSchema::Absent => {}
        surface::DeclaredSchema::Valid(schema) => {
            if schema.validate_config(call.params).is_err() {
                return refuse(
                    tx,
                    call,
                    Some(install.id),
                    refusal_reasons::PARAMS_REFUSED,
                    started,
                )
                .await;
            }
        }
        surface::DeclaredSchema::Invalid => {
            return refuse(
                tx,
                call,
                Some(install.id),
                refusal_reasons::ENTRY_SCHEMA_INVALID,
                started,
            )
            .await
        }
    }

    // The effect is the MANIFEST's declaration, resolved before dispatch: a
    // module is not worth running for an effect nobody consented to, and an
    // effect that needs a ticket must have one before a worker is spent.
    let consented = if call.surface.applies_effect() {
        let Some(effect) = surface::consented_effect(&entry.definition) else {
            return refuse(
                tx,
                call,
                Some(install.id),
                refusal_reasons::EFFECT_NOT_CONSENTED,
                started,
            )
            .await;
        };
        if call.ticket_id.is_none() {
            return refuse(
                tx,
                call,
                Some(install.id),
                refusal_reasons::EFFECT_TARGET_MISSING,
                started,
            )
            .await;
        }
        Some(effect)
    } else {
        None
    };

    let artifact = match lifecycle::load_module(store, &install).await {
        Ok(artifact) => artifact,
        // A declarative install (no module pin) applies its consented effect in
        // the parent. Only an effectful surface has anything to apply; a data
        // surface on such an install has nothing to run.
        Err(ArtifactError::Unpinned) if consented.is_some() => {
            return declarative_attempt(tx, call, &install, &entry, started).await
        }
        Err(error) => {
            return refuse(tx, call, Some(install.id), error.as_str(), started).await;
        }
    };

    let invocation_id = Uuid::now_v7();
    let payload = serde_json::to_vec(call.params).unwrap_or_default();
    let request = build_request(invocation_id, &install, &version.manifest, &entry, payload);
    if let Err(refusal) = request.validate() {
        return refuse(tx, call, Some(install.id), refusal.as_str(), started).await;
    }

    let plugins = match plugins.get(db.clone()).await {
        Ok(plugins) => plugins,
        Err(error) => {
            tracing::warn!(
                entry = call.entry_key,
                error = %error,
                "plugin pool unavailable; the invocation is refused and recorded"
            );
            return worker_failed_attempt(tx, call, Some(install.id), started).await;
        }
    };

    let mut host_calls = TransactionHostCalls {
        tx: &mut *tx,
        org_id: install.org_id,
        install_id: install.id,
        http,
        scopes: request.egress.clone(),
        // The install's own grant set, resolved onto the matrix. The worker
        // already gated the call; the parent re-derives the same answer from
        // its own record because the worker is what it isolates.
        granted: install.granted_permissions(),
        transport_budget: crate::runtime::TRANSPORT_BUDGET,
    };
    let result = plugins
        .invoke_with(&request, &artifact, &mut host_calls)
        .await;
    let mut outcome = result.reply.outcome;
    let mut reason = result.reply.reason.map(|refusal| refusal.as_str());
    let mut comment = None;

    if outcome == InvocationOutcome::Ok {
        if let Some(consented) = consented {
            match effect_verdict(result.reply.output.as_deref(), consented) {
                EffectVerdict::Apply => match apply_consented_effect(tx, call, consented).await? {
                    EffectOutcome::Applied(inserted) => comment = Some(inserted),
                    EffectOutcome::Refused(refusal) => {
                        outcome = InvocationOutcome::Refused;
                        reason = Some(refusal);
                    }
                },
                EffectVerdict::Declined => {
                    outcome = InvocationOutcome::Refused;
                    reason = Some(refusal_reasons::EFFECT_DECLINED);
                }
                EffectVerdict::Contradicted => {
                    outcome = InvocationOutcome::Refused;
                    reason = Some(refusal_reasons::EFFECT_NOT_CONSENTED);
                }
            }
        }
    }

    let worker = result.worker.clone();
    let attempt = Attempt {
        org_id: call.org_id,
        install_id: Some(install.id),
        entry_key: call.entry_key,
        capability: call.surface.capability().as_str(),
        outcome,
        reason,
        invocation_id: Some(result.reply.invocation_id),
        fuel_used: result.reply.fuel_used,
        duration_us: result.reply.duration_us,
        worker: worker.as_deref(),
    };
    write_audit(tx, &attempt).await?;

    Ok(SurfaceOutcome {
        entry_key: call.entry_key.to_string(),
        outcome,
        reason,
        comment,
        output: result.reply.output,
    })
}

/// The declarative attempt: an install with no module pin, whose consented
/// effect the parent applies itself. This is the behaviour core had before the
/// executor existed, kept reachable for the declarative plugins `enable`
/// explicitly allows — and now recorded like every other attempt.
async fn declarative_attempt(
    tx: &mut PgConnection,
    call: &SurfaceCall<'_>,
    install: &PluginInstall,
    entry: &SurfaceEntryRow,
    started: Instant,
) -> core_db::DbResult<SurfaceOutcome> {
    let Some(consented) = surface::consented_effect(&entry.definition) else {
        return refuse(
            tx,
            call,
            Some(install.id),
            refusal_reasons::EFFECT_NOT_CONSENTED,
            started,
        )
        .await;
    };
    let (outcome, reason, comment) = match apply_consented_effect(tx, call, consented).await? {
        EffectOutcome::Applied(comment) => (InvocationOutcome::Ok, None, Some(comment)),
        EffectOutcome::Refused(refusal) => (InvocationOutcome::Refused, Some(refusal), None),
    };
    let attempt = Attempt {
        org_id: call.org_id,
        install_id: Some(install.id),
        entry_key: call.entry_key,
        capability: call.surface.capability().as_str(),
        outcome,
        reason,
        invocation_id: None,
        fuel_used: None,
        duration_us: elapsed_us(started),
        worker: Some(WORKER_DECLARATIVE),
    };
    write_audit(tx, &attempt).await?;
    Ok(SurfaceOutcome {
        entry_key: call.entry_key.to_string(),
        outcome,
        reason,
        comment,
        output: None,
    })
}

/// The row a dead or absent worker leaves: `worker_failed`, no fuel figure and
/// no invocation id, because no frame was answered.
async fn worker_failed_attempt(
    tx: &mut PgConnection,
    call: &SurfaceCall<'_>,
    install_id: Option<Uuid>,
    started: Instant,
) -> core_db::DbResult<SurfaceOutcome> {
    let reason = abi::Refusal::WorkerFailed.as_str();
    let attempt = Attempt {
        org_id: call.org_id,
        install_id,
        entry_key: call.entry_key,
        capability: call.surface.capability().as_str(),
        outcome: InvocationOutcome::WorkerFailed,
        reason: Some(reason),
        invocation_id: None,
        fuel_used: None,
        duration_us: elapsed_us(started),
        worker: None,
    };
    write_audit(tx, &attempt).await?;
    Ok(SurfaceOutcome {
        entry_key: call.entry_key.to_string(),
        outcome: InvocationOutcome::WorkerFailed,
        reason: Some(reason),
        comment: None,
        output: None,
    })
}

/// A pre-dispatch refusal: write the row, answer the caller, write nothing
/// else.
async fn refuse(
    tx: &mut PgConnection,
    call: &SurfaceCall<'_>,
    install_id: Option<Uuid>,
    reason: &'static str,
    started: Instant,
) -> core_db::DbResult<SurfaceOutcome> {
    let attempt = Attempt {
        org_id: call.org_id,
        install_id,
        entry_key: call.entry_key,
        capability: call.surface.capability().as_str(),
        outcome: InvocationOutcome::Refused,
        reason: Some(reason),
        invocation_id: None,
        fuel_used: None,
        duration_us: elapsed_us(started),
        worker: None,
    };
    write_audit(tx, &attempt).await?;
    Ok(SurfaceOutcome {
        entry_key: call.entry_key.to_string(),
        outcome: InvocationOutcome::Refused,
        reason: Some(reason),
        comment: None,
        output: None,
    })
}

// ---------------------------------------------------------------------------
// The host-call service
// ---------------------------------------------------------------------------

/// The ABI's limits, in the shape `core_db::plugin_kv` takes them.
///
/// Constructed from `abi` and `kv` rather than restated, the way
/// `core_db::plugin_health` takes its quarantine threshold as an argument. A
/// copy here would be a second place for a limit to live, and the first one to
/// drift would be the value cap the host and the store both enforce.
const KV_LIMITS: core_db::plugin_kv::KvLimits = core_db::plugin_kv::KvLimits::new(
    abi::KV_VALUE_MAX_BYTES,
    crate::kv::KV_KEYS_PER_INSTALL,
    crate::kv::KV_BYTES_PER_INSTALL,
    crate::kv::KV_LIST_PAGE,
);

/// Answers a worker's host calls from the invocation's own transaction.
///
/// ## Where the tenant comes from
///
/// `org_id` and `install_id` are set from the install row this invocation was
/// authorized against, and they are what every query below binds. The frame
/// cannot name a tenant because it has no field for one (`wire`'s header), and
/// [`TransactionHostCalls::serve`] additionally refuses a [`HostCallContext`]
/// that disagrees with its own record rather than resolving the disagreement in
/// the frame's favour.
///
/// ## Why the invocation's transaction, not a new connection
///
/// The caller already holds one, opened for the request that caused the
/// invocation, with `app.org_id` bound. Using it costs no connection and no
/// `BEGIN` per host call, which is what keeps a `kv_get` to one statement
/// (`plugin_kv::get` is a primary-key read). It also puts a KV write in the same
/// transaction as the audit row and the surface's effect: a rolled-back request
/// rolls back its plugin's writes with it. The price is stated rather than
/// hidden — a genuine SQL failure inside that transaction (a deadlock, a
/// statement timeout) aborts it, exactly as a failed core query would, and the
/// invocation's audit row is lost with it. A `SAVEPOINT` per host call would
/// keep the row and cost two more round trips on every call, which a 250 ms
/// guest budget cannot pay for a case the store only reaches when the database
/// is already unwell. The quota refusal — the one a guest can actually cause —
/// is decided in Rust beside the counter and leaves the transaction usable.
struct TransactionHostCalls<'a> {
    tx: &'a mut PgConnection,
    org_id: Uuid,
    install_id: Uuid,
    /// The parent-side transport for `http_request` (W53-12). The executor
    /// cannot name its type — core-api owns it — so it is a seam.
    http: &'a mut dyn crate::egress::HttpEgress,
    /// The install's consented egress profiles, from the PINNED manifest and
    /// the parent's own copy. A frame cannot add to it: `HostCallOp` has no
    /// field for an allowlist.
    scopes: abi::EgressScopes,
    /// The install's grants, resolved onto the existing permission matrix. Used
    /// only to re-check the capability a call needs.
    granted: Vec<core_domain::policy::Permission>,
    /// How much wall clock this invocation's transport may still spend. Every
    /// HTTP call is clamped to what is left, so 64 calls cannot each take the
    /// full budget.
    transport_budget: Duration,
}

impl HostCallService for TransactionHostCalls<'_> {
    fn serve<'a>(
        &'a mut self,
        context: HostCallContext,
        op: HostCallOp,
    ) -> Pin<Box<dyn Future<Output = HostCallOutcome> + Send + 'a>> {
        Box::pin(async move {
            if !context_is_our_own(&context, self.org_id, self.install_id) {
                tracing::warn!(
                    invocation_id = %context.invocation_id,
                    "plugin host call arrived outside the invocation it was dispatched for"
                );
                return HostCallOutcome::Refused(HostCallRefusal::Failed);
            }
            match op {
                HostCallOp::KvGet { namespace, key } => {
                    if !labels_are_legal(&namespace, &key) {
                        return refused_illegal_label("kv_get");
                    }
                    match core_db::plugin_kv::get(
                        &mut *self.tx,
                        self.org_id,
                        self.install_id,
                        &namespace,
                        &key,
                    )
                    .await
                    {
                        Ok(Some(value)) => HostCallOutcome::Ok(HostCallValue::Value(value)),
                        Ok(None) => HostCallOutcome::Ok(HostCallValue::Absent),
                        Err(error) => store_failure("kv_get", error),
                    }
                }
                HostCallOp::KvPut {
                    namespace,
                    key,
                    value,
                    expires_at,
                } => {
                    if !labels_are_legal(&namespace, &key) {
                        return refused_illegal_label("kv_put");
                    }
                    match core_db::plugin_kv::put(
                        &mut *self.tx,
                        self.org_id,
                        self.install_id,
                        &namespace,
                        &key,
                        &value,
                        expires_at,
                        &KV_LIMITS,
                    )
                    .await
                    {
                        Ok(()) => HostCallOutcome::Ok(HostCallValue::Unit),
                        Err(error) => store_failure("kv_put", error),
                    }
                }
                HostCallOp::KvDelete { namespace, key } => {
                    if !labels_are_legal(&namespace, &key) {
                        return refused_illegal_label("kv_delete");
                    }
                    match core_db::plugin_kv::delete(
                        &mut *self.tx,
                        self.org_id,
                        self.install_id,
                        &namespace,
                        &key,
                    )
                    .await
                    {
                        Ok(existed) => HostCallOutcome::Ok(HostCallValue::Existed(existed)),
                        Err(error) => store_failure("kv_delete", error),
                    }
                }
                HostCallOp::HttpRequest { profile, request } => {
                    self.exchange_http(&profile, &request).await
                }
                HostCallOp::KvList {
                    namespace,
                    prefix,
                    cursor,
                } => {
                    // A prefix and a cursor are keys, so they answer to the same
                    // rule; a page is bounded by `KV_LIMITS.page` either way.
                    if !labels_are_legal(&namespace, &prefix)
                        || !cursor.as_deref().is_none_or(key_slot_is_legal)
                    {
                        return refused_illegal_label("kv_list");
                    }
                    match core_db::plugin_kv::list_keys(
                        &mut *self.tx,
                        self.org_id,
                        self.install_id,
                        &namespace,
                        &prefix,
                        cursor.as_deref(),
                        &KV_LIMITS,
                    )
                    .await
                    {
                        Ok(page) => HostCallOutcome::Ok(HostCallValue::Page {
                            keys: page.keys,
                            cursor: page.next_cursor,
                        }),
                        Err(error) => store_failure("kv_list", error),
                    }
                }
            }
        })
    }
}

impl TransactionHostCalls<'_> {
    /// One `http_request`, in the order that makes each refusal mean something.
    ///
    /// 1. **The capability**, re-derived here from the install's own grant set.
    ///    The worker's gate already did this; a worker that skipped its gate
    ///    must not be able to spend a capability the install does not hold.
    /// 2. **The profile and the URL**, screened against the parent's own copy of
    ///    the pinned manifest's `outbound_scopes` — the same
    ///    `core_domain::egress` functions the worker used, called again because
    ///    this is the process that actually sends.
    /// 3. **The binding**, resolved on the invocation's transaction and
    ///    re-authorised against the vault's category grant. This is the call
    ///    that turns a name into sealed bytes, and the one that must refuse when
    ///    an operator's read was withdrawn after consent. A profile with NO
    ///    binding is not refused: it goes out uncredentialed, which is what an
    ///    allowlisted `http_request` meant before this lane. A profile whose
    ///    binding exists but cannot be used refuses, because sending it anyway
    ///    would downgrade an authenticated connector to an anonymous one
    ///    without anybody saying so.
    /// 4. **The send**, clamped to the transport budget that is left.
    ///
    /// The credential never reaches this function in plaintext: `resolve`
    /// returns ciphertext, and the transport decrypts it in core-api, in the
    /// process that owns the key. Nothing about the credential is returned to
    /// the caller — the guest sees status, headers and body.
    async fn exchange_http(
        &mut self,
        profile: &str,
        request: &abi::HttpRequest,
    ) -> HostCallOutcome {
        let needed = if request.is_write() {
            core_domain::entities::PluginCapability::AutomationActions
        } else {
            core_domain::entities::PluginCapability::AutomationReads
        };
        if !core_domain::policy::check(&self.granted, needed.permission()).is_allowed() {
            return refused_host(HostCallRefusal::CapabilityNotGranted);
        }
        let target = match abi::authorize_egress_target(&self.scopes, profile, &request.url) {
            Ok(target) => target,
            Err(refusal) => return refused_host(host_refusal(refusal)),
        };

        if self.transport_budget.is_zero() {
            return refused_host(HostCallRefusal::BudgetExhausted);
        }

        let credential = match core_db::plugin_credentials::resolve(
            &mut *self.tx,
            self.org_id,
            self.install_id,
            profile,
        )
        .await
        {
            Ok(core_db::plugin_credentials::ResolveOutcome::Granted(credential)) => {
                // Moved out of the box rather than copied: the sealed bytes are
                // the largest thing this path carries and there is exactly one
                // owner.
                let core_db::plugin_credentials::ResolvedCredential {
                    org_id,
                    record_id,
                    field_key,
                    header_name,
                    sealed,
                    ..
                } = *credential;
                Some(crate::egress::SealedCredential {
                    org_id,
                    record_id,
                    field_key,
                    header_name,
                    sealed,
                })
            }
            // NO BINDING IS NOT A REFUSAL. The profile is consented (the gate
            // checked it against the manifest), and an uncredentialed request
            // to an allowlisted host is exactly what Wave 53 shipped. The
            // binding's EXISTENCE is what makes a credential required: a
            // profile nobody bound sends anonymously, and a profile whose
            // binding cannot be used refuses below rather than silently
            // downgrading an authenticated connector to an anonymous one.
            Ok(core_db::plugin_credentials::ResolveOutcome::NotBound) => None,
            Ok(core_db::plugin_credentials::ResolveOutcome::GrantRevoked) => {
                tracing::info!(
                    install_id = %self.install_id,
                    profile,
                    cause = "grant_revoked",
                    "the vault category grant no longer covers this binding"
                );
                return refused_host(HostCallRefusal::CredentialRefused);
            }
            Ok(core_db::plugin_credentials::ResolveOutcome::FieldMissing) => {
                tracing::info!(
                    install_id = %self.install_id,
                    profile,
                    cause = "field_missing",
                    "the bound record has no material field"
                );
                return refused_host(HostCallRefusal::CredentialRefused);
            }
            Err(error) => {
                // The store is the parent's own, so a failure here is the
                // mechanism breaking, not a decision about the plugin.
                tracing::warn!(error = %error, "credential resolve failed");
                return refused_host(HostCallRefusal::Failed);
            }
        };

        let started = Instant::now();
        let sent = self
            .http
            .send(&target, request, credential.as_ref(), self.transport_budget)
            .await;
        self.transport_budget = self.transport_budget.saturating_sub(started.elapsed());
        match sent {
            Ok(response) => HostCallOutcome::Ok(HostCallValue::Http(response)),
            Err(refusal) => refused_host(host_refusal(refusal)),
        }
    }
}

/// A refusal the parent decided, in the shape the bridge carries.
fn refused_host(refusal: HostCallRefusal) -> HostCallOutcome {
    HostCallOutcome::Refused(refusal)
}

/// An ABI refusal from the transport or the screen, mapped onto the bridge's
/// closed vocabulary.
///
/// The transport can only produce the codes below; anything else is the
/// mechanism failing, and [`HostCallRefusal::Failed`] is what the guest's
/// `plugin_worker_failed` means. `Refusal` is `non_exhaustive`, so the wildcard
/// is the fail-closed arm rather than an oversight.
fn host_refusal(refusal: abi::Refusal) -> HostCallRefusal {
    use abi::Refusal;
    match refusal {
        Refusal::CapabilityNotGranted => HostCallRefusal::CapabilityNotGranted,
        Refusal::EgressProfileMissing => HostCallRefusal::EgressProfileMissing,
        Refusal::EgressHostDenied => HostCallRefusal::EgressHostDenied,
        Refusal::EgressSchemeDenied => HostCallRefusal::EgressSchemeDenied,
        Refusal::EgressResponseTooLarge => HostCallRefusal::EgressResponseTooLarge,
        Refusal::FrameInvalid => HostCallRefusal::FrameInvalid,
        Refusal::CredentialRefused => HostCallRefusal::CredentialRefused,
        Refusal::NotImplemented => HostCallRefusal::Unavailable,
        _ => HostCallRefusal::Failed,
    }
}

/// Whether the context the pool passed is the invocation this executor
/// dispatched. Both values are the parent's own; the check exists so that the
/// day one of them is wired wrong, the store refuses instead of serving.
fn context_is_our_own(context: &HostCallContext, org_id: Uuid, install_id: Uuid) -> bool {
    context.org_id == org_id && context.install_id == install_id
}

/// The ABI's label rules, applied to what a frame carried.
///
/// A namespace is never allowed to be empty; a key, a prefix and a cursor are
/// (the ABI's empty prefix and empty cursor mean "everything under here" and
/// "the first page"). A non-empty one answers to the same rule `kv_get`'s key
/// does.
fn labels_are_legal(namespace: &str, key: &str) -> bool {
    crate::host_fns::kv::namespace_is_legal(namespace) && key_slot_is_legal(key)
}

/// A key, a prefix or a cursor from a frame: empty is legal, non-empty answers
/// to the ABI's key rule.
fn key_slot_is_legal(value: &str) -> bool {
    value.is_empty() || crate::host_fns::kv::key_is_legal(value)
}

/// A label the host would have refused at the ABI edge, arriving in a frame
/// anyway: a worker bug, not a guest refusal.
fn refused_illegal_label(verb: &str) -> HostCallOutcome {
    tracing::warn!(verb, "plugin host call carried a label the ABI forbids");
    HostCallOutcome::Refused(HostCallRefusal::Failed)
}

/// Maps a store error onto the closed host-call refusal set.
///
/// The quota refusal is the store's own typed token and the one a guest can
/// cause; everything else is an infrastructure failure that the pool reports as
/// `plugin_worker_failed` after logging it here with the detail the guest must
/// never see.
fn store_failure(verb: &str, error: core_db::DbError) -> HostCallOutcome {
    match error {
        core_db::DbError::Conflict(token) if token == core_db::plugin_kv::QUOTA_EXCEEDED => {
            HostCallOutcome::Refused(HostCallRefusal::QuotaExceeded)
        }
        other => {
            tracing::warn!(verb, error = %other, "plugin store call failed");
            HostCallOutcome::Refused(HostCallRefusal::Failed)
        }
    }
}

// ---------------------------------------------------------------------------
// The request frame
// ---------------------------------------------------------------------------

/// Builds the frame the worker runs. `granted`, `declared_events` and `egress`
/// are the install's and the pinned manifest's; `payload`, `entry` and the ids
/// are the call's.
fn build_request(
    invocation_id: Uuid,
    install: &PluginInstall,
    manifest: &PluginManifestV2,
    entry: &SurfaceEntryRow,
    payload: Vec<u8>,
) -> InvocationRequest {
    InvocationRequest {
        invocation_id,
        org_id: install.org_id,
        install_id: install.id,
        entry: entry.entry_key.clone(),
        payload,
        // THE constraint: the grant set is the install's, verbatim. Nothing a
        // request body, a guest or a worker says can add to it.
        granted: install.granted_capabilities.clone(),
        declared_events: surface::declared_events(&entry.definition),
        egress: manifest
            .connector_profile
            .as_ref()
            .map(|profile| profile.outbound_scopes.clone())
            .unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// The effect
// ---------------------------------------------------------------------------

/// What the guest's answer says about the consented effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffectVerdict {
    /// Apply the consented effect.
    Apply,
    /// The guest said it did not accept the call (or its answer is not a frame
    /// this path can read). Fail closed: no effect.
    Declined,
    /// The guest named a DIFFERENT effect than the one consented. The guest
    /// does not choose the effect; the manifest does.
    Contradicted,
}

/// Reads the guest's answer against the consented effect.
///
/// No output means the module said nothing and the consented effect stands. A
/// JSON object may carry `effect` (which must be the consented one) and
/// `accepted` (which must not be `false`). Anything unreadable is a decline,
/// because an unreadable answer is not an acceptance.
fn effect_verdict(output: Option<&[u8]>, consented: SurfaceActionEffect) -> EffectVerdict {
    let Some(bytes) = output else {
        return EffectVerdict::Apply;
    };
    if bytes.is_empty() {
        return EffectVerdict::Apply;
    }
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return EffectVerdict::Declined;
    };
    let Some(object) = value.as_object() else {
        return EffectVerdict::Declined;
    };
    if let Some(named) = object.get("effect").and_then(Value::as_str) {
        if named != consented.as_str() {
            return EffectVerdict::Contradicted;
        }
    }
    if object.get("accepted").and_then(Value::as_bool) == Some(false) {
        return EffectVerdict::Declined;
    }
    EffectVerdict::Apply
}

/// The result of applying one consented effect.
enum EffectOutcome {
    Applied(TicketComment),
    Refused(&'static str),
}

/// Applies the effect the entry's manifest consented, in the caller's
/// transaction. `params` are the caller's, already schema-checked; the body
/// bounds are the effect's own and are checked here so a comment the old
/// hardcoded effect refused is still refused.
async fn apply_consented_effect(
    tx: &mut PgConnection,
    call: &SurfaceCall<'_>,
    effect: SurfaceActionEffect,
) -> core_db::DbResult<EffectOutcome> {
    match effect {
        SurfaceActionEffect::AddInternalComment => {
            let Some(ticket_id) = call.ticket_id else {
                return Ok(EffectOutcome::Refused(
                    refusal_reasons::EFFECT_TARGET_MISSING,
                ));
            };
            let Some(body) = call.params.get("body").and_then(Value::as_str) else {
                return Ok(EffectOutcome::Refused(refusal_reasons::PARAMS_REFUSED));
            };
            let body = body.trim();
            if body.is_empty() || body.len() > MAX_EFFECT_BODY_BYTES {
                return Ok(EffectOutcome::Refused(refusal_reasons::PARAMS_REFUSED));
            }
            let comment = core_db::tickets::insert_comment(
                &mut *tx,
                core_db::tickets::NewTicketComment {
                    id: Uuid::now_v7(),
                    org_id: call.org_id,
                    ticket_id,
                    // No human author: the install is the actor, and the audit
                    // row names it. The same shape the hardcoded effect wrote.
                    author_id: None,
                    kind: NoteKind::Internal,
                    body,
                    workflow_meta: None,
                },
            )
            .await?;
            Ok(EffectOutcome::Applied(comment))
        }
    }
}

// ---------------------------------------------------------------------------
// The audit row
// ---------------------------------------------------------------------------

/// Everything one attempt's row needs. `reason` is a static string, never the
/// guest's prose and never a database error message.
struct Attempt<'a> {
    org_id: Uuid,
    /// `None` only when no install was resolvable; the row is then attributed
    /// to `system`.
    install_id: Option<Uuid>,
    entry_key: &'a str,
    /// The capability the surface requires, from the closed
    /// [`SurfaceKind`] mapping — never from the entry or the caller.
    capability: &'static str,
    outcome: InvocationOutcome,
    reason: Option<&'static str>,
    invocation_id: Option<Uuid>,
    fuel_used: Option<u64>,
    duration_us: u64,
    worker: Option<&'a str>,
}

/// The `audit_events` row ADR 0023 Decision 7 fixes. Pure, so the shape is
/// testable without a database.
fn audit_event(attempt: &Attempt<'_>) -> AuditEvent {
    AuditEvent {
        id: Uuid::now_v7(),
        org_id: attempt.org_id,
        actor: match attempt.install_id {
            Some(install_id) => Actor::Plugin(install_id.to_string()),
            // The entry key is caller-supplied, so it may never choose the
            // actor. With no install resolvable, `system` is the only honest
            // attribution, and `reason` says why.
            None => Actor::System,
        },
        action: AUDIT_ACTION.to_string(),
        entity_type: Some(AUDIT_ENTITY.to_string()),
        entity_id: Some(attempt.entry_key.to_string()),
        detail: json!({
            "outcome": attempt.outcome.as_str(),
            "reason": attempt.reason,
            "capability": attempt.capability,
            "invocation_id": attempt.invocation_id,
            "fuel_used": attempt.fuel_used,
            "duration_us": attempt.duration_us,
            "worker": attempt.worker,
        }),
        ip: None,
        occurred_at: chrono::Utc::now(),
    }
}

async fn write_audit(tx: &mut PgConnection, attempt: &Attempt<'_>) -> core_db::DbResult<()> {
    core_db::audit::insert(&mut *tx, &audit_event(attempt)).await
}

fn elapsed_us(started: Instant) -> u64 {
    started.elapsed().as_micros() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every refusal the transport can produce keeps its identity across the
    /// bridge, and the one the guest must never confuse with anything else —
    /// code 25 — is unchanged in both directions.
    ///
    /// A mapping that collapsed two causes would be a guest-visible difference
    /// between "no credential" and "the URL was refused", which is the
    /// distinction an author debugs with.
    #[test]
    fn the_transport_refusals_survive_the_bridge() {
        use abi::Refusal;
        let pairs = [
            (
                Refusal::CapabilityNotGranted,
                HostCallRefusal::CapabilityNotGranted,
            ),
            (
                Refusal::EgressProfileMissing,
                HostCallRefusal::EgressProfileMissing,
            ),
            (Refusal::EgressHostDenied, HostCallRefusal::EgressHostDenied),
            (
                Refusal::EgressSchemeDenied,
                HostCallRefusal::EgressSchemeDenied,
            ),
            (
                Refusal::EgressResponseTooLarge,
                HostCallRefusal::EgressResponseTooLarge,
            ),
            (Refusal::FrameInvalid, HostCallRefusal::FrameInvalid),
            (
                Refusal::CredentialRefused,
                HostCallRefusal::CredentialRefused,
            ),
        ];
        for (refusal, host) in pairs {
            assert_eq!(host_refusal(refusal), host, "{refusal:?}");
            assert_eq!(
                host.as_refusal(),
                refusal,
                "{refusal:?} must come back out unchanged"
            );
        }
        // Anything else is the mechanism failing, which is `plugin_worker_failed`
        // and counts against the install — the loud direction, not a silent ok.
        assert_eq!(
            host_refusal(Refusal::GuestTrap),
            HostCallRefusal::Failed,
            "an unmapped refusal fails closed"
        );
        assert_eq!(Refusal::CredentialRefused.code(), 25);
    }

    use core_domain::entities::plugins::ConnectorProfile;
    use core_domain::entities::plugins::{
        PluginCapability, PluginInstallStatus, PluginSurfaceProfile, SurfaceAction,
        SurfaceActionEffect, PLUGIN_RUNTIME_INTERFACE_VERSION,
    };

    fn install(grants: &[&str]) -> PluginInstall {
        PluginInstall {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            catalog_id: Uuid::now_v7(),
            slug: "triage-note-action".into(),
            version: "1.0.1".into(),
            module_digest: String::new(),
            status: PluginInstallStatus::Enabled,
            granted_capabilities: grants.iter().map(|grant| grant.to_string()).collect(),
            installed_by: None,
            installed_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            revoked_at: None,
            revoked_by: None,
            revoked_reason: None,
            previous_version: None,
            previous_module_digest: None,
            previous_capabilities: Vec::new(),
        }
    }

    fn manifest() -> PluginManifestV2 {
        let mut outbound_scopes = std::collections::BTreeMap::new();
        outbound_scopes.insert(
            "telemetry".to_string(),
            vec!["https://api.example.com".to_string()],
        );
        PluginManifestV2 {
            name: "Triage Note Action".into(),
            version: "1.0.1".into(),
            interface_version: PLUGIN_RUNTIME_INTERFACE_VERSION,
            description: "Adds the triage note.".into(),
            capabilities: vec!["automation.actions".into()],
            settings_schema: None,
            artifact_kind: None,
            connector_profile: Some(ConnectorProfile {
                webhook_endpoints: Vec::new(),
                scopes: Vec::new(),
                routing: None,
                outbound_scopes,
            }),
            surface_profile: Some(PluginSurfaceProfile {
                actions: vec![SurfaceAction {
                    id: "triage_note".into(),
                    label: "Triage note".into(),
                    description: "Adds the note.".into(),
                    effect: SurfaceActionEffect::AddInternalComment,
                    params_schema: None,
                    output_schema: None,
                    idempotent: false,
                    emits: vec!["ticket.comment_added".into()],
                    audience: None,
                    agent_description: None,
                }],
                ..PluginSurfaceProfile::default()
            }),
        }
    }

    fn entry() -> SurfaceEntryRow {
        SurfaceEntryRow {
            id: Uuid::now_v7(),
            install_id: Uuid::now_v7(),
            surface: "automation_action".into(),
            entry_key: "plugin.triage-note-action.triage_note".into(),
            label: "Triage note".into(),
            definition: json!({
                "effect": "add_internal_comment",
                "node_contract": {"emits": ["ticket.comment_added"]},
            }),
            enabled: true,
        }
    }

    #[test]
    fn a_request_carries_only_the_installs_own_grants() {
        let install = install(&["automation.actions"]);
        let entry = entry();
        // A payload that NAMES capabilities is still only bytes: the request's
        // grant set comes from the install row and nowhere else.
        let payload = serde_json::to_vec(&json!({
            "body": "hello",
            "granted": ["tickets.write", "secrets.read"],
            "capabilities": ["tickets.write"],
            "grant": ["tickets.write"],
        }))
        .expect("payload serialises");
        let request = build_request(
            Uuid::now_v7(),
            &install,
            &manifest(),
            &entry,
            payload.clone(),
        );
        assert_eq!(request.granted, vec!["automation.actions".to_string()]);
        assert_eq!(request.payload, payload);
        assert_eq!(request.org_id, install.org_id);
        assert_eq!(request.install_id, install.id);
        assert_eq!(request.entry, entry.entry_key);
        // The frame validates: the payload's claims are not grants to reject.
        assert_eq!(request.validate(), Ok(()));
    }

    #[test]
    fn a_request_takes_declared_events_and_egress_from_the_pinned_manifest() {
        let install = install(&["automation.actions"]);
        let request = build_request(
            Uuid::now_v7(),
            &install,
            &manifest(),
            &entry(),
            b"{}".to_vec(),
        );
        assert_eq!(
            request.declared_events,
            vec!["ticket.comment_added".to_string()]
        );
        assert_eq!(
            request.egress.get("telemetry").map(Vec::as_slice),
            Some(&["https://api.example.com".to_string()][..])
        );
    }

    #[test]
    fn a_manifest_without_an_egress_declaration_consents_nothing() {
        let install = install(&["automation.actions"]);
        let mut manifest = manifest();
        manifest.connector_profile = None;
        let request = build_request(
            Uuid::now_v7(),
            &install,
            &manifest,
            &entry(),
            b"{}".to_vec(),
        );
        assert!(request.egress.is_empty());
    }

    #[test]
    fn the_audit_row_has_the_seven_decision_detail_keys_and_the_install_actor() {
        let install = install(&["automation.actions"]);
        let invocation_id = Uuid::now_v7();
        let attempt = Attempt {
            org_id: install.org_id,
            install_id: Some(install.id),
            entry_key: "plugin.triage-note-action.triage_note",
            capability: PluginCapability::AutomationActions.as_str(),
            outcome: InvocationOutcome::Ok,
            reason: None,
            invocation_id: Some(invocation_id),
            fuel_used: Some(38_909),
            duration_us: 42,
            worker: Some("marula-plugin-worker:1234"),
        };
        let event = audit_event(&attempt);
        assert_eq!(event.actor, Actor::Plugin(install.id.to_string()));
        assert_eq!(event.action, "plugin.invoke");
        assert_eq!(event.entity_type.as_deref(), Some("plugin_entry"));
        assert_eq!(
            event.entity_id.as_deref(),
            Some("plugin.triage-note-action.triage_note")
        );
        let detail = event.detail.as_object().expect("detail is an object");
        let mut keys: Vec<&str> = detail.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "capability",
                "duration_us",
                "fuel_used",
                "invocation_id",
                "outcome",
                "reason",
                "worker",
            ]
        );
        assert_eq!(detail["outcome"], "ok");
        assert_eq!(detail["reason"], Value::Null);
        assert_eq!(detail["capability"], "automation.actions");
        assert_eq!(detail["invocation_id"], invocation_id.to_string());
        assert_eq!(detail["fuel_used"], 38_909);
        assert_eq!(detail["duration_us"], 42);
        assert_eq!(detail["worker"], "marula-plugin-worker:1234");
    }

    #[test]
    fn a_refusal_without_an_install_is_attributed_to_system_never_to_the_entry_key() {
        let attempt = Attempt {
            org_id: Uuid::now_v7(),
            install_id: None,
            entry_key: "plugin.somebody-elses.triage_note",
            capability: PluginCapability::AutomationActions.as_str(),
            outcome: InvocationOutcome::Refused,
            reason: Some(refusal_reasons::ENTRY_UNAVAILABLE),
            invocation_id: None,
            fuel_used: None,
            duration_us: 3,
            worker: None,
        };
        let event = audit_event(&attempt);
        assert_eq!(event.actor, Actor::System);
        assert_eq!(event.detail["reason"], refusal_reasons::ENTRY_UNAVAILABLE);
        assert_eq!(event.detail["invocation_id"], Value::Null);
        assert_eq!(event.detail["fuel_used"], Value::Null);
        assert_eq!(event.detail["worker"], Value::Null);
    }

    #[test]
    fn a_worker_failure_row_reports_no_fuel_and_the_static_reason() {
        let attempt = Attempt {
            org_id: Uuid::now_v7(),
            install_id: Some(Uuid::now_v7()),
            entry_key: "plugin.triage-note-action.triage_note",
            capability: PluginCapability::AutomationActions.as_str(),
            outcome: InvocationOutcome::WorkerFailed,
            reason: Some(abi::Refusal::WorkerFailed.as_str()),
            invocation_id: None,
            fuel_used: None,
            duration_us: 700,
            worker: None,
        };
        let event = audit_event(&attempt);
        assert_eq!(event.detail["outcome"], "worker_failed");
        assert_eq!(event.detail["reason"], "plugin_worker_failed");
        assert_eq!(event.detail["fuel_used"], Value::Null);
    }

    #[test]
    fn the_guest_does_not_choose_the_effect() {
        let consented = SurfaceActionEffect::AddInternalComment;
        // The reference plugin's own answer.
        assert_eq!(
            effect_verdict(
                Some(br#"{"effect":"add_internal_comment","accepted":true}"#),
                consented
            ),
            EffectVerdict::Apply
        );
        // A different effect, even one the vocabulary knows.
        assert_eq!(
            effect_verdict(Some(br#"{"effect":"add_internal_comment"}"#), consented),
            EffectVerdict::Apply
        );
        assert_eq!(
            effect_verdict(Some(br#"{"effect":"not_consented"}"#), consented),
            EffectVerdict::Contradicted
        );
        // The plugin declining its own call.
        assert_eq!(
            effect_verdict(
                Some(br#"{"effect":"add_internal_comment","accepted":false,"reason":"body_required"}"#),
                consented
            ),
            EffectVerdict::Declined
        );
        // An answer this path cannot read is not an acceptance.
        assert_eq!(
            effect_verdict(Some(b"not json"), consented),
            EffectVerdict::Declined
        );
        assert_eq!(
            effect_verdict(Some(b"[1,2]"), consented),
            EffectVerdict::Declined
        );
        // A module that says nothing leaves the consented effect standing.
        assert_eq!(effect_verdict(None, consented), EffectVerdict::Apply);
        assert_eq!(effect_verdict(Some(b""), consented), EffectVerdict::Apply);
    }

    #[test]
    fn a_fault_is_reported_and_a_refusal_is_not() {
        let refused = SurfaceOutcome {
            entry_key: "plugin.triage-note-action.triage_note".into(),
            outcome: InvocationOutcome::Refused,
            reason: Some(refusal_reasons::ENTRY_UNAVAILABLE),
            comment: None,
            output: None,
        };
        assert!(!refused.is_fault(), "a refusal is an answer");

        let failed = SurfaceOutcome {
            outcome: InvocationOutcome::WorkerFailed,
            reason: Some(abi::Refusal::WorkerFailed.as_str()),
            ..refused
        };
        assert!(failed.is_fault(), "a worker that died is a fault");
        assert_eq!(failed.skip_reason(), "plugin_worker_failed");
    }

    #[test]
    fn the_declarative_label_is_not_a_worker_name() {
        assert_eq!(WORKER_DECLARATIVE, "declarative");
        assert!(!WORKER_DECLARATIVE.starts_with("marula-plugin-worker:"));
    }

    // -----------------------------------------------------------------------
    // The host-call service (W53-11)
    // -----------------------------------------------------------------------

    #[test]
    fn a_host_call_served_for_another_invocation_is_refused() {
        let org = Uuid::now_v7();
        let install = Uuid::now_v7();
        let own = HostCallContext {
            invocation_id: Uuid::now_v7(),
            org_id: org,
            install_id: install,
        };
        assert!(context_is_our_own(&own, org, install));
        // The frame cannot name a tenant, so the only way this is reached is a
        // pool wiring bug — and a bug must refuse, not serve whichever tenant
        // the frame happened to carry.
        let mut other_org = own;
        other_org.org_id = Uuid::now_v7();
        assert!(!context_is_our_own(&other_org, org, install));
        let mut other_install = own;
        other_install.install_id = Uuid::now_v7();
        assert!(!context_is_our_own(&other_install, org, install));
    }

    #[test]
    fn the_service_applies_the_abi_label_rules_to_what_a_frame_carried() {
        for good in ["cache", "a-z_0.9"] {
            assert!(labels_are_legal(good, "k"), "{good:?} is a namespace");
        }
        for bad in ["", "Cache", "ca che", "cache/slash", &"a".repeat(65)] {
            assert!(!labels_are_legal(bad, "k"), "{bad:?} must not be one");
        }
        // A key, a prefix and a cursor may be empty; a non-empty one answers to
        // the ABI's key rule.
        assert!(key_slot_is_legal(""));
        assert!(key_slot_is_legal(&"k".repeat(512)));
        assert!(!key_slot_is_legal(&"k".repeat(513)));
        assert!(!key_slot_is_legal("a\0b"));
        assert!(!labels_are_legal("cache", &"k".repeat(513)));
    }

    #[test]
    fn only_the_stores_quota_token_becomes_a_quota_refusal() {
        assert_eq!(
            store_failure(
                "kv_put",
                core_db::DbError::Conflict(core_db::plugin_kv::QUOTA_EXCEEDED.to_string())
            ),
            HostCallOutcome::Refused(HostCallRefusal::QuotaExceeded)
        );
        // A conflict that is not the quota token, a missing install and a
        // driver failure are all the same to the guest: the store failed.
        for error in [
            core_db::DbError::Conflict("plugin_something_else".to_string()),
            core_db::DbError::NotFound,
            core_db::DbError::Sql(sqlx::Error::PoolTimedOut),
        ] {
            assert_eq!(
                store_failure("kv_put", error),
                HostCallOutcome::Refused(HostCallRefusal::Failed)
            );
        }
    }

    #[test]
    fn the_store_limits_are_the_abi_and_kv_numbers_and_not_a_second_copy() {
        assert_eq!(KV_LIMITS.max_value_bytes, abi::KV_VALUE_MAX_BYTES);
        assert_eq!(KV_LIMITS.max_keys, crate::kv::KV_KEYS_PER_INSTALL);
        assert_eq!(KV_LIMITS.max_bytes, crate::kv::KV_BYTES_PER_INSTALL);
        assert_eq!(KV_LIMITS.page, crate::kv::KV_LIST_PAGE);
        assert_eq!(KV_LIMITS.page, i64::from(abi::KV_LIST_PAGE));
    }
}
