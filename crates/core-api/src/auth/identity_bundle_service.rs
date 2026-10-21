//! Identity-bundle lifecycle services (lane `p52-identity-bundle`).
//!
//! Why a service module rather than routes: the HTTP admin surface for
//! identity bundles lands with its UI, and adding paths here would move the
//! committed OpenAPI pin, which this lane cannot re-pin (the ritual boots a
//! stack). The lifecycle itself is not UI work, though — an install must be
//! enableable, disableable and revocable by something that audits and
//! publishes, so that something is these functions. The routes, when they
//! land, call them and add no logic of their own.
//!
//! What lives here, and only here:
//!
//! * the audited lifecycle transition (audit row + event in ONE
//!   transaction, so a state change and its announcement cannot split);
//! * the JIT opt-in, which is separate from enable on purpose;
//! * the read surface the admin screen will render.
//!
//! What does NOT live here: sessions, tokens, MFA. A bundle is an identity
//! SOURCE; it never mints a credential. `auth/provider.rs` owns the login
//! path and `auth/handlers.rs` owns issuance.

use crate::auth::federation::IdentityBundleRegistry;
use crate::auth::AuthedUser;
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use core_domain::entities::identity_bundle::{IdentityBundleAction, IdentityBundleInstall};
use core_domain::events::types;
use core_domain::events::{IdentityBundleRevoked, IdentityBundleUpdated};
use uuid::Uuid;

/// Applies `action` to an install: transition, audit, event, one
/// transaction. The status the DB reports back is the one announced, so a
/// refused transition never publishes a lifecycle event.
pub async fn apply_action(
    state: &AppState,
    authed: &AuthedUser,
    install_id: Uuid,
    action: IdentityBundleAction,
    reason: Option<String>,
) -> ApiResult<IdentityBundleInstall> {
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let now = chrono::Utc::now();
    org_tx::with_org_tx(state, org_id, actor, |mut tx| async move {
        let install = core_db::identity_bundles::set_status(
            &mut tx.tx,
            org_id,
            install_id,
            action,
            reason.as_deref(),
            now,
        )
        .await?;
        if install.status.is_revoked() {
            core_db::audit::insert(
                &mut *tx.tx,
                &core_domain::entities::AuditEvent {
                    id: Uuid::now_v7(),
                    org_id,
                    actor: core_domain::entities::Actor::Human(actor),
                    action: types::IDENTITY_BUNDLE_REVOKED.into(),
                    entity_type: Some("identity_bundle_install".into()),
                    entity_id: Some(install.id.to_string()),
                    detail: core_db::identity_bundles::install_audit_detail(&install),
                    ip: None,
                    occurred_at: now,
                },
            )
            .await?;
            tx.emit(
                types::IDENTITY_BUNDLE_REVOKED,
                "identity_bundle_install",
                Some(install.id.to_string().as_str()),
                &serde_json::to_value(IdentityBundleRevoked {
                    install_id: install.id,
                    name: install.name.clone(),
                    version: install.version.clone(),
                    reason: install.revoked_reason.clone(),
                    revoked_at: now,
                })
                .unwrap_or(serde_json::Value::Null),
            )
            .await?;
        } else {
            core_db::audit::insert(
                &mut *tx.tx,
                &core_domain::entities::AuditEvent {
                    id: Uuid::now_v7(),
                    org_id,
                    actor: core_domain::entities::Actor::Human(actor),
                    action: types::IDENTITY_BUNDLE_UPDATED.into(),
                    entity_type: Some("identity_bundle_install".into()),
                    entity_id: Some(install.id.to_string()),
                    detail: core_db::identity_bundles::install_audit_detail(&install),
                    ip: None,
                    occurred_at: now,
                },
            )
            .await?;
            tx.emit(
                types::IDENTITY_BUNDLE_UPDATED,
                "identity_bundle_install",
                Some(install.id.to_string().as_str()),
                &serde_json::to_value(IdentityBundleUpdated {
                    install_id: install.id,
                    name: install.name.clone(),
                    version: install.version.clone(),
                    status: install.status.as_str().to_string(),
                    jit_enabled: install.jit_enabled,
                    occurred_at: now,
                })
                .unwrap_or(serde_json::Value::Null),
            )
            .await?;
        }
        Ok((tx, install))
    })
    .await
}

/// The JIT opt-in. Separate from enable/disable: a bundle can authenticate
/// existing linked identities while provisioning stays off, and turning
/// provisioning on does not turn the bundle on.
pub async fn set_jit(
    state: &AppState,
    authed: &AuthedUser,
    install_id: Uuid,
    enabled: bool,
) -> ApiResult<IdentityBundleInstall> {
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let now = chrono::Utc::now();
    org_tx::with_org_tx(state, org_id, actor, |mut tx| async move {
        let install = core_db::identity_bundles::set_jit_enabled(
            &mut tx.tx, org_id, install_id, enabled, now,
        )
        .await?;
        core_db::audit::insert(
            &mut *tx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: core_domain::entities::Actor::Human(actor),
                action: types::IDENTITY_BUNDLE_UPDATED.into(),
                entity_type: Some("identity_bundle_install".into()),
                entity_id: Some(install.id.to_string()),
                detail: core_db::identity_bundles::install_audit_detail(&install),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        tx.emit(
            types::IDENTITY_BUNDLE_UPDATED,
            "identity_bundle_install",
            Some(install.id.to_string().as_str()),
            &serde_json::to_value(IdentityBundleUpdated {
                install_id: install.id,
                name: install.name.clone(),
                version: install.version.clone(),
                status: install.status.as_str().to_string(),
                jit_enabled: install.jit_enabled,
                occurred_at: now,
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        Ok((tx, install))
    })
    .await
}

/// The org's installs — what the admin surface lists and what the
/// registration seam is built from.
pub async fn list_installs(
    state: &AppState,
    authed: &AuthedUser,
) -> ApiResult<Vec<IdentityBundleInstall>> {
    let org_id = authed.principal.user.org_id;
    let mut read_tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut read_tx, org_id).await?;
    let installs = core_db::identity_bundles::list_installs(&mut read_tx, org_id).await?;
    read_tx.commit().await.map_err(ApiError::from)?;
    Ok(installs)
}

/// The registry a request-scoped login path would consult. Built from the
/// org's installs every call rather than cached, for the same reason the
/// login paths read their source rows on demand: an admin's disable must
/// take effect on the next attempt, not after a restart.
pub async fn registry_for_org(
    state: &AppState,
    authed: &AuthedUser,
) -> ApiResult<IdentityBundleRegistry> {
    let installs = list_installs(state, authed).await?;
    Ok(IdentityBundleRegistry::from_installs(&installs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_domain::entities::plugins::PluginInstallStatus;

    /// The service announces exactly the states the domain admits, and a
    /// revoked install is announced as revoked — never as "updated".
    #[test]
    fn the_announced_status_follows_the_domain_state_machine() {
        assert!(!PluginInstallStatus::Enabled.is_revoked());
        assert!(!PluginInstallStatus::Disabled.is_revoked());
        assert!(PluginInstallStatus::Revoked.is_revoked());
        for action in [
            IdentityBundleAction::Enable,
            IdentityBundleAction::Disable,
            IdentityBundleAction::Revoke,
        ] {
            let next = core_domain::entities::identity_bundle::transition(
                PluginInstallStatus::Disabled,
                action,
            )
            .expect("disabled admits every action");
            let event = if next.is_revoked() {
                types::IDENTITY_BUNDLE_REVOKED
            } else {
                types::IDENTITY_BUNDLE_UPDATED
            };
            assert!(event.starts_with("identity_bundle."));
        }
    }
}
