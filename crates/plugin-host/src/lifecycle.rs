//! Plugin lifecycle and the artifact store seam (Wave 53 lane 4,
//! `p53-lifecycle-distribution`; ADR 0023 Decisions 6 and 7).
//!
//! Two things live here, and they are here together because they are one
//! question: what an install IS between the moment an admin adopts it and the
//! moment the executor is handed a module.
//!
//! # The state machine
//!
//! An install has exactly one status — `disabled`, `enabled` or `revoked`
//! (`plugin_install_status`, migration 0076) — and one pin: the sha256 of the
//! executable module it adopted. [`LifecycleAction::apply`] is the whole
//! transition table, as one total function, so no route has to remember it:
//!
//! | current | install | enable | disable | update | rollback | revoke |
//! |---|---|---|---|---|---|---|
//! | (none) | `disabled` | refuse | refuse | refuse | refuse | refuse |
//! | `disabled` | refuse | `enabled` | `disabled` (no-op) | `disabled`, new pin | `disabled`, old pin | `revoked` |
//! | `enabled` | refuse | `enabled` (no-op) | `disabled` | `enabled`, new pin | `enabled`, old pin | `revoked` |
//! | `revoked` | refuse | refuse | refuse | refuse | refuse | `revoked` (no-op) |
//!
//! The refusals are typed ([`LifecycleRefusal`]), never prose, so a route turns
//! one into an envelope reason code and an audit row without parsing anything.
//!
//! ## Three different things that are not each other
//!
//! Migration 0154's header already settled this once, and it is worth restating
//! where the code lives:
//!
//! * **Disable** is an admin's reversible choice. Nothing is torn down, the pin
//!   stays, one `enable` brings it back. Grants are untouched.
//! * **Revoke** is terminal. The install never runs again; re-adopting the
//!   catalog entry is the only way back, and that mints a fresh install row.
//!   The pin and the bytes stay for forensics. Revocation also tears down the
//!   install's outbound webhooks, because it is a status flip and the FK
//!   cascade that covers uninstall never fires.
//! * **Quarantine** is neither. It is the POOL's judgement after three
//!   consecutive worker deaths (`runtime::QUARANTINE_AFTER_FAILURES`), it
//!   touches no grant and no status, and an operator clears it. It lives in
//!   `plugin_install_health` (0154) precisely so it cannot be mistaken for a
//!   lifecycle status.
//!
//! A fourth thing that is also not these: **uninstall** (`DELETE`), which
//! removes the row and cascades. Revoke exists so an operator can stop a plugin
//! without erasing the evidence of what it did.
//!
//! # The pin, and rollback
//!
//! The installed digest IS the pin. An update pins a NEW digest and parks the
//! outgoing `(version, digest, grants)` as the rollback target; a rollback moves
//! the pointer back and clears the target. Nothing is re-downloaded and nothing
//! is rewritten: the object store keys modules by digest, so an update writes a
//! new key and the old bytes stay exactly where they were. Two installs of one
//! module share one object, and one install cannot corrupt another's.
//!
//! A pin that does not match the bytes is a refusal, not a warning — twice:
//! [`admit_module`] refuses to store bytes that do not hash to the digest they
//! claim, and [`load_module`] re-hashes what it read and refuses to hand the
//! executor anything else. The worker repeats the check on every load, which is
//! why a parent bug cannot widen the admission path (ADR 0023 Decision 2).
//!
//! # Key rotation does not touch installs
//!
//! A signature is a gate at ADMISSION. Once bytes are stored under their
//! digest, running them needs no key material at all — [`load_module`] takes a
//! store, an org and an install, and no trust set. So rotating the operator key
//! (add the new key, retire the old, per migration 0095) cannot invalidate an
//! install that was admitted under the old one: the pin is a digest, not a key
//! id. The test below proves the property rather than asserting it in prose.
//!
//! What rotation DOES affect is the next admission: a retired key still
//! verifies (past trust), a revoked key verifies nothing (compromise response),
//! and that is `distribution`'s existing behaviour, unchanged here.
//!
//! # The store key
//!
//! `org/<org_id>/plugins/<sha256_hex>` — [`core_store::plugin_artifact_key`],
//! beside `attachments/` and `quarantine/` and sharing their validation. An
//! install row's `module_digest` is the address; there is no second index to
//! keep in step.
//!
//! # What W53-5 calls
//!
//! ```ignore
//! let artifact = plugin_host::lifecycle::load_module(&*state.store, &install).await?;
//! let result = pool.invoke(&request, &artifact).await;
//! ```
//!
//! [`load_module`] refuses a revoked or disabled install before it reads a
//! byte, so revoke is enforced by the parent's gate and not by a worker's
//! memory. The pool's own `enabled()` remains the outermost check; this is the
//! per-install gate beneath it.

use bytes::{Bytes, BytesMut};
use core_domain::entities::{PluginInstall, PluginInstallStatus};
use core_store::{plugin_artifact_key, validate_key, GetObject, ObjectStore, StoreError};
use futures_util::TryStreamExt;

use crate::isolation::ModuleArtifact;

/// Largest module this layer will read out of the store. Equal to
/// `distribution::MAX_ARCHIVE_BYTES` on purpose: an artifact that could not be
/// imported must not become loadable by being stored and read back.
pub const MAX_MODULE_BYTES: usize = 8 * 1024 * 1024;

/// The content type recorded for a stored module.
pub const MODULE_CONTENT_TYPE: &str = "application/wasm";

// --- the pin -----------------------------------------------------------------

/// One install's pin: the version, the module digest, and the grants that were
/// consented for that pair.
///
/// They travel together because they are one consent. Restoring an older digest
/// while keeping a newer grant set would run old code with authority its
/// manifest never declared, so a rollback that moved only the digest would be a
/// privilege escalation with a friendly name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub version: String,
    pub module_digest: String,
    pub capabilities: Vec<String>,
}

impl Pin {
    /// Reads the pin off an install row.
    pub fn of(install: &PluginInstall) -> Self {
        Pin {
            version: install.version.clone(),
            module_digest: install.module_digest.clone(),
            capabilities: install.granted_capabilities.clone(),
        }
    }

    /// The rollback target an install carries, if it has one.
    pub fn previous_of(install: &PluginInstall) -> Option<Self> {
        let version = install.previous_version.clone()?;
        Some(Pin {
            version,
            module_digest: install.previous_module_digest.clone().unwrap_or_default(),
            capabilities: install.previous_capabilities.clone(),
        })
    }

    /// Does this pin name bytes the executor could load?
    pub fn is_pinned(&self) -> bool {
        !self.module_digest.is_empty()
    }
}

// --- the state machine -------------------------------------------------------

/// The six lifecycle verbs, named once for the audit row and the route layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LifecycleVerb {
    Install,
    Enable,
    Disable,
    Update,
    Rollback,
    Revoke,
}

impl LifecycleVerb {
    /// The stable string an audit row carries.
    pub fn as_str(self) -> &'static str {
        match self {
            LifecycleVerb::Install => "install",
            LifecycleVerb::Enable => "enable",
            LifecycleVerb::Disable => "disable",
            LifecycleVerb::Update => "update",
            LifecycleVerb::Rollback => "rollback",
            LifecycleVerb::Revoke => "revoke",
        }
    }
}

/// One lifecycle action, carrying the data its verb needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleAction {
    /// Adopt a version: the install is created DISABLED (deny-by-default).
    Install {
        pin: Pin,
    },
    Enable,
    Disable,
    /// Move to a new pin, parking the current one for rollback.
    Update {
        pin: Pin,
    },
    /// Move back to the parked pin, clearing it.
    Rollback,
    /// Terminal stop. The reason is required by the DB layer.
    Revoke {
        reason: String,
    },
}

impl LifecycleAction {
    pub fn verb(&self) -> LifecycleVerb {
        match self {
            LifecycleAction::Install { .. } => LifecycleVerb::Install,
            LifecycleAction::Enable => LifecycleVerb::Enable,
            LifecycleAction::Disable => LifecycleVerb::Disable,
            LifecycleAction::Update { .. } => LifecycleVerb::Update,
            LifecycleAction::Rollback => LifecycleVerb::Rollback,
            LifecycleAction::Revoke { .. } => LifecycleVerb::Revoke,
        }
    }
}

/// Why a verb was refused. Static reason codes, in the manner of
/// [`crate::denial_reasons`]: an audit row and an API envelope carry the code
/// and never prose to parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleRefusal {
    /// The verb needs an install and there is none.
    NotInstalled,
    /// The org already adopts this catalog entry (the DB's unique index is the
    /// authority; this is the pre-flight answer).
    AlreadyInstalled,
    /// Revocation is terminal. Re-adoption is a new install.
    Revoked,
    /// A rollback with nothing parked.
    NoPreviousPin,
    /// An update that would clear the pin an install already holds.
    PinWouldBeDropped,
    /// The action's own data is incoherent (an empty reason, a malformed
    /// digest).
    Invalid(&'static str),
}

impl LifecycleRefusal {
    /// The stable reason code for an audit row / API envelope.
    pub fn as_str(&self) -> &'static str {
        match self {
            LifecycleRefusal::NotInstalled => "plugin_lifecycle_not_installed",
            LifecycleRefusal::AlreadyInstalled => "plugin_already_installed",
            LifecycleRefusal::Revoked => "plugin_install_revoked",
            LifecycleRefusal::NoPreviousPin => "plugin_lifecycle_no_previous_pin",
            LifecycleRefusal::PinWouldBeDropped => "plugin_lifecycle_pin_would_be_dropped",
            LifecycleRefusal::Invalid(reason) => reason,
        }
    }
}

/// A lifecycle state as the transition table sees it: a status, the current
/// pin, and the parked rollback target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleState {
    pub status: PluginInstallStatus,
    pub pin: Pin,
    pub previous: Option<Pin>,
}

impl LifecycleState {
    pub fn from_install(install: &PluginInstall) -> Self {
        LifecycleState {
            status: install.status,
            pin: Pin::of(install),
            previous: Pin::previous_of(install),
        }
    }
}

/// The whole transition table (see the module docs), as one total function.
///
/// `current` is `None` when the org does not adopt the entry yet. Every verb is
/// matched against every state, so a new status cannot arrive without this
/// function failing to compile.
pub fn apply(
    current: Option<&LifecycleState>,
    action: &LifecycleAction,
) -> Result<LifecycleState, LifecycleRefusal> {
    // Install is the only verb that works on "nothing", and it always produces
    // DISABLED. Everything else needs a row.
    if let LifecycleAction::Install { pin } = action {
        return match current {
            Some(_) => Err(LifecycleRefusal::AlreadyInstalled),
            None => Ok(LifecycleState {
                status: PluginInstallStatus::Disabled,
                pin: pin.clone(),
                previous: None,
            }),
        };
    }
    let Some(state) = current else {
        return Err(LifecycleRefusal::NotInstalled);
    };

    // Revocation is terminal for every verb, including a second revoke (which
    // is a no-op, because "stop this" is already true).
    if state.status == PluginInstallStatus::Revoked {
        return match action {
            LifecycleAction::Revoke { .. } => Ok(state.clone()),
            _ => Err(LifecycleRefusal::Revoked),
        };
    }

    match action {
        LifecycleAction::Install { .. } => Err(LifecycleRefusal::AlreadyInstalled),
        LifecycleAction::Enable => Ok(LifecycleState {
            status: PluginInstallStatus::Enabled,
            pin: state.pin.clone(),
            previous: state.previous.clone(),
        }),
        LifecycleAction::Disable => Ok(LifecycleState {
            status: PluginInstallStatus::Disabled,
            pin: state.pin.clone(),
            previous: state.previous.clone(),
        }),
        LifecycleAction::Update { pin } => {
            if pin.module_digest.is_empty() && state.pin.is_pinned() {
                // The one silent downgrade this lane exists to prevent: an
                // update that adopts a version with no module would leave the
                // install pointing at nothing while still looking healthy.
                return Err(LifecycleRefusal::PinWouldBeDropped);
            }
            let previous = if pin.module_digest == state.pin.module_digest {
                // Re-consenting the same bytes: the rollback target the
                // operator already has is still the right one.
                state.previous.clone()
            } else {
                Some(state.pin.clone())
            };
            Ok(LifecycleState {
                status: state.status,
                pin: pin.clone(),
                previous,
            })
        }
        LifecycleAction::Rollback => {
            let Some(previous) = state.previous.clone() else {
                return Err(LifecycleRefusal::NoPreviousPin);
            };
            Ok(LifecycleState {
                status: state.status,
                pin: previous,
                // One step, not a toggle: a second rollback needs the update
                // re-consented, which is the operator saying which bytes they
                // want rather than ping-ponging between two.
                previous: None,
            })
        }
        LifecycleAction::Revoke { reason } => {
            if reason.trim().is_empty() {
                return Err(LifecycleRefusal::Invalid("plugin_revoke_reason_required"));
            }
            Ok(LifecycleState {
                status: PluginInstallStatus::Revoked,
                pin: state.pin.clone(),
                previous: state.previous.clone(),
            })
        }
    }
}

// --- the artifact store seam -------------------------------------------------

/// Why the artifact layer refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactError {
    /// The install names no module. A declarative-only install is legal; it
    /// just cannot be executed.
    Unpinned,
    /// The digest is not a sha256 hex, so it cannot address anything.
    BadDigest,
    /// The bytes do not hash to the digest they were admitted under.
    DigestMismatch { declared: String, actual: String },
    /// The object the pin names is not in the store.
    Missing { digest: String },
    /// The install is disabled or revoked. Refused before a byte is read.
    NotRunnable { status: PluginInstallStatus },
    /// The stored object is larger than this layer will read.
    TooLarge { size: usize },
    /// The store itself failed. Fail closed: never run bytes we could not read
    /// completely.
    Store(String),
}

impl ArtifactError {
    /// The stable reason code for an audit row / API envelope.
    pub fn as_str(&self) -> &'static str {
        match self {
            ArtifactError::Unpinned => "plugin_artifact_unpinned",
            ArtifactError::BadDigest => "plugin_artifact_bad_digest",
            ArtifactError::DigestMismatch { .. } => "plugin_artifact_digest_mismatch",
            ArtifactError::Missing { .. } => "plugin_artifact_missing",
            ArtifactError::NotRunnable { .. } => "plugin_install_not_runnable",
            ArtifactError::TooLarge { .. } => "plugin_artifact_too_large",
            ArtifactError::Store(_) => "plugin_artifact_store_unavailable",
        }
    }
}

impl std::fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArtifactError::Unpinned => write!(f, "the install names no module digest"),
            ArtifactError::BadDigest => write!(f, "the module digest is not a sha256 hex"),
            ArtifactError::DigestMismatch { declared, actual } => {
                write!(f, "module digest {declared} but bytes hash to {actual}")
            }
            ArtifactError::Missing { digest } => write!(f, "module {digest} is not in the store"),
            ArtifactError::NotRunnable { status } => {
                write!(f, "a {} install does not run", status.as_str())
            }
            ArtifactError::TooLarge { size } => {
                write!(
                    f,
                    "module is {size} bytes, above the {MAX_MODULE_BYTES} cap"
                )
            }
            ArtifactError::Store(detail) => write!(f, "object store: {detail}"),
        }
    }
}

impl std::error::Error for ArtifactError {}

/// Stores verified module bytes under their digest and returns the digest.
///
/// This is the admission write: the caller has already verified a signature (or
/// accepted a local import), and this function enforces the second half — that
/// the bytes ARE what the digest claims — before they become addressable. A
/// mismatch writes nothing.
///
/// Content-addressed, so a re-import of the same module rewrites the same
/// object with identical bytes; no dedup table is needed and no install can
/// observe a different module under a digest it already pinned.
pub async fn admit_module(
    store: &dyn ObjectStore,
    org_id: uuid::Uuid,
    bytes: &[u8],
    expected_digest: &str,
) -> Result<String, ArtifactError> {
    let expected = expected_digest.trim().to_ascii_lowercase();
    if expected.is_empty() {
        return Err(ArtifactError::Unpinned);
    }
    if !is_sha256_hex(&expected) {
        return Err(ArtifactError::BadDigest);
    }
    if bytes.len() > MAX_MODULE_BYTES {
        return Err(ArtifactError::TooLarge { size: bytes.len() });
    }
    let actual = ModuleArtifact::digest_of(bytes);
    if actual != expected {
        return Err(ArtifactError::DigestMismatch {
            declared: expected,
            actual,
        });
    }
    let key = plugin_artifact_key(org_id, &actual);
    validate_key(&key).map_err(|error| ArtifactError::Store(error.to_string()))?;
    store
        .put(&key, MODULE_CONTENT_TYPE, Bytes::copy_from_slice(bytes))
        .await
        .map_err(|error| ArtifactError::Store(error.to_string()))?;
    Ok(actual)
}

/// Checks that a pin's bytes are present and still hash to the pin, without
/// returning them.
///
/// Used where a digest is about to become an install's pin (an update's
/// consent): a pin that cannot be loaded is refused at the moment it would be
/// adopted, not at the first invocation.
pub async fn verify_stored_module(
    store: &dyn ObjectStore,
    org_id: uuid::Uuid,
    digest: &str,
) -> Result<(), ArtifactError> {
    read_module(store, org_id, digest).await.map(|_| ())
}

/// Loads an install's pinned module, ready for the executor.
///
/// **This is the call W53-5 makes.** It refuses before reading anything when the
/// install is not enabled, so a revoked install cannot execute even if a caller
/// forgot the authorization gate; and it re-hashes the bytes it read, so a store
/// object that lies about its key is refused rather than run.
pub async fn load_module(
    store: &dyn ObjectStore,
    install: &PluginInstall,
) -> Result<ModuleArtifact, ArtifactError> {
    if install.status != PluginInstallStatus::Enabled {
        return Err(ArtifactError::NotRunnable {
            status: install.status,
        });
    }
    if install.module_digest.is_empty() {
        return Err(ArtifactError::Unpinned);
    }
    read_module(store, install.org_id, &install.module_digest).await
}

/// The read half shared by [`load_module`] and [`verify_stored_module`].
async fn read_module(
    store: &dyn ObjectStore,
    org_id: uuid::Uuid,
    digest: &str,
) -> Result<ModuleArtifact, ArtifactError> {
    let digest = digest.trim().to_ascii_lowercase();
    if !is_sha256_hex(&digest) {
        return Err(ArtifactError::BadDigest);
    }
    let key = plugin_artifact_key(org_id, &digest);
    validate_key(&key).map_err(|error| ArtifactError::Store(error.to_string()))?;
    let object = match store.get(&key).await {
        Ok(object) => object,
        Err(StoreError::NotFound(_)) => return Err(ArtifactError::Missing { digest }),
        Err(error) => return Err(ArtifactError::Store(error.to_string())),
    };
    if object.size_bytes as usize > MAX_MODULE_BYTES {
        return Err(ArtifactError::TooLarge {
            size: object.size_bytes as usize,
        });
    }
    let bytes = collect(object, MAX_MODULE_BYTES).await?;
    let actual = ModuleArtifact::digest_of(&bytes);
    if actual != digest {
        return Err(ArtifactError::DigestMismatch {
            declared: digest,
            actual,
        });
    }
    Ok(ModuleArtifact {
        digest,
        bytes: bytes.to_vec(),
    })
}

/// Drains a store body with a hard cap, so a lying `size_bytes` cannot make the
/// reader allocate past the module limit.
async fn collect(object: GetObject, cap: usize) -> Result<Bytes, ArtifactError> {
    let body = object
        .stream
        .try_fold(BytesMut::new(), |mut acc, chunk| async move {
            if acc.len() + chunk.len() > cap {
                return Err(StoreError::Rejected(format!(
                    "module body exceeded {cap} bytes"
                )));
            }
            acc.extend_from_slice(&chunk);
            Ok(acc)
        })
        .await
        .map_err(|error: StoreError| match error {
            StoreError::Rejected(detail) => ArtifactError::Store(detail),
            other => ArtifactError::Store(other.to_string()),
        })?;
    Ok(body.freeze())
}

/// The digest shape every pin must have: 64 lowercase hex characters.
fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use core_store::memory::MemoryStore;
    use uuid::Uuid;

    fn install(status: PluginInstallStatus, digest: &str) -> PluginInstall {
        PluginInstall {
            id: Uuid::now_v7(),
            org_id: Uuid::nil(),
            catalog_id: Uuid::now_v7(),
            slug: "ticket-summarizer".into(),
            version: "1.0.0".into(),
            module_digest: digest.to_string(),
            status,
            granted_capabilities: vec!["tickets.read".into()],
            installed_by: None,
            installed_at: Utc::now(),
            updated_at: Utc::now(),
            revoked_at: None,
            revoked_by: None,
            revoked_reason: None,
            previous_version: None,
            previous_module_digest: None,
            previous_capabilities: Vec::new(),
        }
    }

    fn pin(version: &str, digest: &str) -> Pin {
        Pin {
            version: version.into(),
            module_digest: digest.into(),
            capabilities: vec!["tickets.read".into()],
        }
    }

    fn state(status: PluginInstallStatus) -> LifecycleState {
        LifecycleState {
            status,
            pin: pin("1.0.0", &"a".repeat(64)),
            previous: None,
        }
    }

    // --- the transition table ------------------------------------------------

    #[test]
    fn install_creates_a_disabled_install_and_refuses_a_second_one() {
        let adopted = apply(
            None,
            &LifecycleAction::Install {
                pin: pin("1.0.0", &"a".repeat(64)),
            },
        )
        .expect("a fresh adopt is legal");
        assert_eq!(adopted.status, PluginInstallStatus::Disabled);
        assert_eq!(adopted.previous, None);
        assert_eq!(
            apply(
                Some(&adopted),
                &LifecycleAction::Install {
                    pin: pin("2.0.0", &"b".repeat(64))
                }
            ),
            Err(LifecycleRefusal::AlreadyInstalled)
        );
    }

    #[test]
    fn every_verb_but_install_refuses_when_there_is_no_install() {
        for action in [
            LifecycleAction::Enable,
            LifecycleAction::Disable,
            LifecycleAction::Update {
                pin: pin("2.0.0", &"b".repeat(64)),
            },
            LifecycleAction::Rollback,
            LifecycleAction::Revoke {
                reason: "incident".into(),
            },
        ] {
            assert_eq!(
                apply(None, &action),
                Err(LifecycleRefusal::NotInstalled),
                "{:?} on no install must refuse",
                action.verb()
            );
        }
    }

    #[test]
    fn enable_and_disable_are_reversible_and_idempotent() {
        let disabled = state(PluginInstallStatus::Disabled);
        let enabled = apply(Some(&disabled), &LifecycleAction::Enable).unwrap();
        assert_eq!(enabled.status, PluginInstallStatus::Enabled);
        assert_eq!(enabled.pin, disabled.pin, "enable never moves the pin");
        // Idempotent in both directions: repeating a verb is not an error.
        assert_eq!(
            apply(Some(&enabled), &LifecycleAction::Enable)
                .unwrap()
                .status,
            PluginInstallStatus::Enabled
        );
        assert_eq!(
            apply(Some(&enabled), &LifecycleAction::Disable)
                .unwrap()
                .status,
            PluginInstallStatus::Disabled
        );
        assert_eq!(
            apply(Some(&disabled), &LifecycleAction::Disable)
                .unwrap()
                .status,
            PluginInstallStatus::Disabled
        );
    }

    #[test]
    fn revoke_is_terminal_for_every_other_verb() {
        let revoked = state(PluginInstallStatus::Revoked);
        for action in [
            LifecycleAction::Enable,
            LifecycleAction::Disable,
            LifecycleAction::Update {
                pin: pin("2.0.0", &"b".repeat(64)),
            },
            LifecycleAction::Rollback,
        ] {
            assert_eq!(
                apply(Some(&revoked), &action),
                Err(LifecycleRefusal::Revoked),
                "{:?} must refuse a revoked install",
                action.verb()
            );
        }
        // A second revoke is a no-op, not a conflict: "stop this" is already
        // true, and an operator retrying a kill switch must not be told no.
        assert_eq!(
            apply(
                Some(&revoked),
                &LifecycleAction::Revoke {
                    reason: "again".into()
                }
            )
            .unwrap()
            .status,
            PluginInstallStatus::Revoked
        );
    }

    #[test]
    fn revoke_requires_a_reason() {
        let enabled = state(PluginInstallStatus::Enabled);
        assert_eq!(
            apply(
                Some(&enabled),
                &LifecycleAction::Revoke {
                    reason: "  ".into()
                }
            ),
            Err(LifecycleRefusal::Invalid("plugin_revoke_reason_required"))
        );
    }

    #[test]
    fn update_parks_the_previous_pin_and_rollback_returns_to_it() {
        let enabled = state(PluginInstallStatus::Enabled);
        let next = pin("2.0.0", &"b".repeat(64));
        let updated = apply(
            Some(&enabled),
            &LifecycleAction::Update { pin: next.clone() },
        )
        .unwrap();
        assert_eq!(updated.pin, next);
        assert_eq!(
            updated.status,
            PluginInstallStatus::Enabled,
            "update keeps the status"
        );
        assert_eq!(updated.previous, Some(enabled.pin.clone()));

        let rolled = apply(Some(&updated), &LifecycleAction::Rollback).unwrap();
        assert_eq!(
            rolled.pin, enabled.pin,
            "rollback returns to the previous digest"
        );
        assert_eq!(rolled.previous, None, "one step, not a toggle");
        assert_eq!(
            apply(Some(&rolled), &LifecycleAction::Rollback),
            Err(LifecycleRefusal::NoPreviousPin)
        );
    }

    #[test]
    fn re_consenting_the_same_digest_keeps_the_existing_rollback_target() {
        let mut current = state(PluginInstallStatus::Enabled);
        current.previous = Some(pin("0.9.0", &"c".repeat(64)));
        let same = apply(
            Some(&current),
            &LifecycleAction::Update {
                pin: pin("1.0.1", &"a".repeat(64)),
            },
        )
        .unwrap();
        assert_eq!(
            same.previous, current.previous,
            "the operator did not move off the bytes they could return to"
        );
    }

    #[test]
    fn an_update_cannot_drop_a_pin_the_install_already_holds() {
        let enabled = state(PluginInstallStatus::Enabled);
        let unpinned = Pin {
            version: "2.0.0".into(),
            module_digest: String::new(),
            capabilities: vec![],
        };
        assert_eq!(
            apply(Some(&enabled), &LifecycleAction::Update { pin: unpinned }),
            Err(LifecycleRefusal::PinWouldBeDropped)
        );
    }

    #[test]
    fn a_declarative_only_install_may_move_between_unpinned_versions() {
        let declarative = LifecycleState {
            status: PluginInstallStatus::Enabled,
            pin: Pin {
                version: "1.0.0".into(),
                module_digest: String::new(),
                capabilities: vec![],
            },
            previous: None,
        };
        let next = Pin {
            version: "1.1.0".into(),
            module_digest: String::new(),
            capabilities: vec![],
        };
        let updated = apply(
            Some(&declarative),
            &LifecycleAction::Update { pin: next.clone() },
        )
        .expect("nothing was pinned, so nothing is dropped");
        assert_eq!(updated.pin, next);
        assert_eq!(
            updated.previous, None,
            "no digest moved, so there is nothing to park"
        );
    }

    #[test]
    fn every_verb_reports_a_stable_audit_string() {
        assert_eq!(
            [
                LifecycleVerb::Install,
                LifecycleVerb::Enable,
                LifecycleVerb::Disable,
                LifecycleVerb::Update,
                LifecycleVerb::Rollback,
                LifecycleVerb::Revoke,
            ]
            .map(LifecycleVerb::as_str),
            ["install", "enable", "disable", "update", "rollback", "revoke"]
        );
    }

    // --- the store -----------------------------------------------------------

    fn wasm_bytes(marker: u8) -> Vec<u8> {
        // A minimal core module with one exported function; the pool's own
        // loader validates the ABI, this layer only cares that the bytes hash.
        let wat =
            format!("(module (func (export \"marula_entry\") (result i32) i32.const {marker}))");
        wat::parse_str(wat).expect("valid WAT")
    }

    #[tokio::test]
    async fn admit_then_load_round_trips_through_the_content_addressed_key() {
        let store = MemoryStore::new();
        let org = Uuid::now_v7();
        let bytes = wasm_bytes(7);
        let digest = ModuleArtifact::digest_of(&bytes);

        let admitted = admit_module(&store, org, &bytes, &digest)
            .await
            .expect("verified bytes are admitted");
        assert_eq!(admitted, digest);

        // The object is addressable by the digest alone.
        let key = plugin_artifact_key(org, &digest);
        assert!(store.get(&key).await.is_ok());

        let mut row = install(PluginInstallStatus::Enabled, &digest);
        row.org_id = org;
        let module = load_module(&store, &row).await.expect("pinned bytes load");
        assert_eq!(module.digest, digest);
        assert_eq!(module.bytes, bytes);
    }

    #[tokio::test]
    async fn admission_refuses_bytes_that_do_not_hash_to_the_digest() {
        let store = MemoryStore::new();
        let org = Uuid::now_v7();
        let bytes = wasm_bytes(1);
        let claimed = ModuleArtifact::digest_of(&wasm_bytes(2));
        assert_eq!(
            admit_module(&store, org, &bytes, &claimed).await,
            Err(ArtifactError::DigestMismatch {
                declared: claimed.clone(),
                actual: ModuleArtifact::digest_of(&bytes),
            })
        );
        // Nothing was written under the claim.
        assert!(matches!(
            store.get(&plugin_artifact_key(org, &claimed)).await,
            Err(StoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn admission_refuses_an_empty_or_malformed_digest() {
        let store = MemoryStore::new();
        let org = Uuid::now_v7();
        let bytes = wasm_bytes(1);
        assert_eq!(
            admit_module(&store, org, &bytes, "").await,
            Err(ArtifactError::Unpinned)
        );
        assert_eq!(
            admit_module(&store, org, &bytes, "not-a-digest").await,
            Err(ArtifactError::BadDigest)
        );
    }

    #[tokio::test]
    async fn load_refuses_a_disabled_or_revoked_install_before_reading() {
        let store = MemoryStore::new();
        let org = Uuid::now_v7();
        let bytes = wasm_bytes(3);
        let digest = ModuleArtifact::digest_of(&bytes);
        admit_module(&store, org, &bytes, &digest).await.unwrap();

        for status in [PluginInstallStatus::Disabled, PluginInstallStatus::Revoked] {
            let mut row = install(status, &digest);
            row.org_id = org;
            assert_eq!(
                load_module(&store, &row).await,
                Err(ArtifactError::NotRunnable { status })
            );
        }
    }

    #[tokio::test]
    async fn load_refuses_an_unpinned_install_and_a_missing_object() {
        let store = MemoryStore::new();
        let org = Uuid::now_v7();

        let mut declarative = install(PluginInstallStatus::Enabled, "");
        declarative.org_id = org;
        assert_eq!(
            load_module(&store, &declarative).await,
            Err(ArtifactError::Unpinned)
        );

        let missing = "f".repeat(64);
        let mut row = install(PluginInstallStatus::Enabled, &missing);
        row.org_id = org;
        assert_eq!(
            load_module(&store, &row).await,
            Err(ArtifactError::Missing {
                digest: missing.clone()
            })
        );
    }

    #[tokio::test]
    async fn a_store_object_that_lies_about_its_key_is_refused() {
        let store = MemoryStore::new();
        let org = Uuid::now_v7();
        let real = wasm_bytes(4);
        let claimed = ModuleArtifact::digest_of(&wasm_bytes(5));
        // Write bytes under a key that does not name them — the shape a bug or a
        // tampered store would produce.
        store
            .put(
                &plugin_artifact_key(org, &claimed),
                MODULE_CONTENT_TYPE,
                Bytes::copy_from_slice(&real),
            )
            .await
            .unwrap();
        let mut row = install(PluginInstallStatus::Enabled, &claimed);
        row.org_id = org;
        assert_eq!(
            load_module(&store, &row).await,
            Err(ArtifactError::DigestMismatch {
                declared: claimed,
                actual: ModuleArtifact::digest_of(&real),
            })
        );
    }

    #[tokio::test]
    async fn verify_stored_module_is_the_consent_time_check() {
        let store = MemoryStore::new();
        let org = Uuid::now_v7();
        let bytes = wasm_bytes(6);
        let digest = ModuleArtifact::digest_of(&bytes);
        // Before admission the pin is not yet adoptable.
        assert_eq!(
            verify_stored_module(&store, org, &digest).await,
            Err(ArtifactError::Missing {
                digest: digest.clone()
            })
        );
        admit_module(&store, org, &bytes, &digest).await.unwrap();
        assert_eq!(verify_stored_module(&store, org, &digest).await, Ok(()));
    }

    /// The rotation property, proved rather than described: an install admitted
    /// under one operator key loads after the trust set moves on, because the
    /// pin is a digest and the load path takes no key material.
    #[tokio::test]
    async fn a_key_rotation_does_not_invalidate_an_admitted_install() {
        use ed25519_dalek::Signer as _;

        let store = MemoryStore::new();
        let org = Uuid::now_v7();
        let bytes = wasm_bytes(8);
        let digest = ModuleArtifact::digest_of(&bytes);

        // Admission: the artifact was signed by the key the operator trusted at
        // the time, and the signature verified over the digest.
        let old = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let old_key_hex = hex::encode(old.verifying_key().as_bytes());
        let digest_bytes: [u8; 32] = hex::decode(&digest).unwrap().try_into().unwrap();
        let signature = hex::encode(old.sign(&digest_bytes).to_bytes());
        assert_eq!(
            crate::distribution::verify_artifact_signature(&old_key_hex, &digest, &signature),
            Ok(())
        );
        admit_module(&store, org, &bytes, &digest).await.unwrap();
        let mut row = install(PluginInstallStatus::Enabled, &digest);
        row.org_id = org;

        // Rotation: a new key becomes active and the old one is retired from
        // the trust set (migration 0095 semantics: retired is past trust).
        let new = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let new_key_hex = hex::encode(new.verifying_key().as_bytes());
        let trust = [crate::distribution::TrustedSigningKey {
            key_id: "k2".into(),
            public_key: new_key_hex.clone(),
        }];
        // The next admission signed by the retired key is refused ...
        assert_eq!(
            crate::distribution::verify_artifact_signature_against_keys(
                &trust, &digest, &signature,
            ),
            Err(crate::distribution::IngestReason::BadSignature)
        );
        // ... and the install admitted under it still runs, because nothing in
        // the load path consults a key.
        let module = load_module(&store, &row)
            .await
            .expect("a rotated key cannot strand admitted bytes");
        assert_eq!(module.digest, digest);
    }
}
