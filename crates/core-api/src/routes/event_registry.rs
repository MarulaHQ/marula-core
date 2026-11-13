//! The event-type registry read surface (Wave 67 lane `w67-read-surfaces`).
//!
//! Purpose: `/settings/integrations` could OFFER event types but not LIST the
//! ones this org may actually subscribe to. The registry (`event_types`,
//! migration 0124) is validation-only server-side, so the surface re-typed a
//! platform catalogue in TypeScript and marked every other selected type
//! "needs grant" because it had no way to know. The automations editor had the
//! same hole for plugin-emitted trigger types.
//!
//! # What one read answers
//!
//! Three lists, each a different admission rule, kept apart because merging
//! them would erase the distinction the refusal is built on:
//!
//! * `platform` — Marula's own types. Always subscribable, no grant needed.
//! * `granted` — the org's `event_type_grants` rows. Each carries whatever the
//!   catalogue knows (label, description) and `in_registry`, because a grant
//!   for a type no install declares is legal and must still read back.
//! * `plugin_declared` — the trigger types the org's live plugin installs
//!   declare. These arrive as signed ingest events, not as workflow nodes
//!   (Wave 43 lane 13's catalogue decision), so they belong beside the registry
//!   rather than in the node palette.
//!
//! # Permission
//!
//! `ManagePlugins`, the same grant the webhook admin routes take: this is the
//! read half of a surface only a plugin admin can change, and it names an org's
//! inbound surface inventory. A member who cannot edit a subscription has no
//! reason to enumerate the types behind it.
//!
//! Non-purpose: no grant write (creating one is a separate decision with its own
//! audit row), no catalogue seeding, no type registration.

use crate::auth::middleware::{authed, require_permission};
use crate::error::ApiResult;
use crate::org_tx;
use crate::state::AppState;
use axum::{extract::State, http::Extensions, Json};
use core_domain::policy::Permission;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// One catalogue type.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EventTypeEntryView {
    pub event_type: String,
    pub label: String,
    pub description: Option<String>,
}

/// One of the org's grants, with what the catalogue knows about the type.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GrantedEventTypeView {
    pub event_type: String,
    /// Absent when the grant names a type the catalogue has never seen — the
    /// escape hatch migration 0124 deliberately allows.
    pub label: Option<String>,
    pub description: Option<String>,
    /// False for a grant whose type no catalogue row declares.
    pub in_registry: bool,
    pub granted_at: chrono::DateTime<chrono::Utc>,
}

/// The registry as one read.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EventRegistryView {
    /// Marula's own types: subscribable without a grant.
    pub platform: Vec<EventTypeEntryView>,
    /// The org's grants, including those for types outside the catalogue.
    pub granted: Vec<GrantedEventTypeView>,
    /// Trigger types the org's live plugin installs declare.
    pub plugin_declared: Vec<String>,
}

/// GET /api/v1/event-types — the registry: platform types, the org's grants,
/// and the plugin-declared trigger types.
#[utoipa::path(operation_id = "event_type_registry", get, path = "/api/v1/event-types",
    tag = "integrations", security(("bearer" = [])),
    responses((status = 200, body = EventRegistryView)))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<EventRegistryView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManagePlugins)?;
    let org_id = authed.principal.user.org_id;

    let mut tx = org_tx::begin(&state, org_id, authed.principal.user.id).await?;
    let platform = core_db::event_types::list_platform_event_types(&mut *tx.tx).await?;
    let granted = core_db::event_types::list_org_grants_detailed(&mut *tx.tx, org_id).await?;
    let plugin_declared =
        core_db::event_types::list_org_plugin_event_types(&mut *tx.tx, org_id).await?;
    tx.commit().await?;

    Ok(Json(EventRegistryView {
        platform: platform
            .into_iter()
            .map(|e| EventTypeEntryView {
                event_type: e.event_type,
                label: e.label,
                description: e.description,
            })
            .collect(),
        granted: granted
            .into_iter()
            .map(|g| GrantedEventTypeView {
                event_type: g.event_type,
                label: g.label,
                description: g.description,
                in_registry: g.in_registry,
                granted_at: g.granted_at,
            })
            .collect(),
        plugin_declared,
    }))
}
