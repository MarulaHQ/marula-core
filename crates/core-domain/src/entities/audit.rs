//! Append-only audit event entity (§6). Written for auth events, permission
//! changes, exports, plugin invocations, AI calls, invoice/payment mutations —
//! and every actor carries provenance (human, agent, plugin, system).

use crate::ids::{AuditEventId, OrgId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Who performed the audited action. Append-only provenance record.
/// Portal variants carry the client-side tier so audit trails distinguish
/// staff actions from client-portal actions at the storage layer
/// (actor_kind portal_contact / portal_approver / portal_it_manager).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum Actor {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    Human(UserId),
    Agent(String),
    Plugin(String),
    Connector(String),
    System,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    PortalContact(UserId),
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    PortalApprover(UserId),
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    PortalItManager(UserId),
    /// Bulk-import runs (TASKLIST D2): the descriptor names the driving
    /// identity without pretending it row-walked — `"user:<uuid>"` for
    /// admin JWT runs, `"api-key:<key-id>""` for machine pushes. Row-level
    /// mutations still emit their ordinary domain events; the import actor
    /// owns batch-lifecycle provenance (kind "import" in audit storage).
    Import(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuditEvent {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: AuditEventId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub actor: Actor,
    /// Dot-path of the action, e.g. "auth.login", "ticket.status_change".
    pub action: String,
    /// Entity type + id the action targeted, when applicable.
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    /// Structured detail — never contains secrets.
    pub detail: serde_json::Value,
    pub ip: Option<String>,
    pub occurred_at: DateTime<Utc>,
}
