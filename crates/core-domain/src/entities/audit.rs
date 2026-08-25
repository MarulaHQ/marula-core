//! Append-only audit event entity (§6). Written for auth events, permission
//! changes, exports, plugin invocations, AI calls, invoice/payment mutations —
//! and every actor carries provenance (human, agent, plugin, system).

use crate::ids::{AuditEventId, OrgId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Who performed the audited action. Append-only provenance record.
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
