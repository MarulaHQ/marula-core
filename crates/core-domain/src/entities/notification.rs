//! Notification center entities: per-user notifications with channel
//! fan-out state, preferences, and quiet hours (Stage 2).

use crate::ids::{NotificationId, OrgId, TicketId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    Mention,
    Assignment,
    SlaRisk,
    SlaBreach,
    Escalation,
    MissingTime,
    TicketUpdate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    InApp,
    Email,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Notification {
    pub id: NotificationId,
    pub org_id: OrgId,
    pub user_id: UserId,
    pub kind: NotificationKind,
    pub ticket_id: Option<TicketId>,
    pub title: String,
    pub body: Option<String>,
    pub read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Per-user, per-channel delivery preferences with quiet hours (UTC offsets
/// resolved against the user's timezone at send time).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NotificationPreference {
    pub user_id: UserId,
    pub kind: NotificationKind,
    pub in_app: bool,
    pub email: bool,
    /// Quiet hours in the user's local time; deliveries outside urgency
    /// classes are deferred to the end of the window.
    pub quiet_from: Option<String>,
    pub quiet_until: Option<String>,
}
