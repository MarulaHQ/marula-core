//! Scheduling-link entities (Wave 11 lane 4): Calendly-style booking
//! surfaces — free-form groups, meeting-type templates, and shareable
//! link tokens. Serde shapes shared by DB mapping and the API.
//!
//! Non-purpose: availability math and booking validation
//! (core_domain::scheduling + core-api book-time re-checks), persistence.

use crate::ids::{OrgId, TicketId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Who a meeting type books against: ONE user's calendar, or the
/// unioned availability of a free-form admin-defined group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MeetingAssigneeScope {
    User,
    Group,
}

impl MeetingAssigneeScope {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "group" => Some(Self::Group),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Group => "group",
        }
    }
}

/// A free-form admin-defined group/department — a pure availability
/// pool ("Service Desk"). Deliberately NOT the org chart: no reporting
/// lines, no permissions; membership only feeds unioned availability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SchedulingGroup {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    pub description: Option<String>,
    /// Current membership (unioned availability pool).
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<uuid::Uuid>))]
    pub member_user_ids: Vec<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub created_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub updated_at: DateTime<Utc>,
}

/// A bookable template: duration, buffer before/after, booking window,
/// org-editable free-form `parameters`, and the assignee scope. Auto-
/// email is per-template; with transport `none` the send is skipped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MeetingType {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    pub name: String,
    pub description: Option<String>,
    pub duration_minutes: i32,
    pub buffer_minutes: i32,
    pub booking_window_days: i32,
    /// Free-form org-edited payload (location hint, agenda, questions…).
    /// Consumers must tolerate any shape; nothing in behavior keys off it.
    pub parameters: serde_json::Value,
    pub assignee_scope: MeetingAssigneeScope,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub assignee_user_id: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub assignee_group_id: Option<uuid::Uuid>,
    pub auto_email_enabled: bool,
    pub auto_email_subject: Option<String>,
    pub auto_email_body: Option<String>,
    pub is_active: bool,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub created_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub updated_at: DateTime<Utc>,
}

/// Link lifecycle: `active` → `used` (a booking landed) or `revoked`
/// (tech pulled it). Expiry is DERIVED (`expires_at` + the template's
/// booking window), never a stored state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SchedulingLinkStatus {
    Active,
    Revoked,
    Used,
}

impl SchedulingLinkStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "revoked" => Some(Self::Revoked),
            "used" => Some(Self::Used),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
            Self::Used => "used",
        }
    }
}

/// A shareable booking link. The raw token is generated at create,
/// returned ONCE in the create response, and never stored (only its
/// SHA-256 hash); the tech's visible lifecycle is the view/schedule/
/// revoke timestamps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SchedulingLink {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub meeting_type_id: uuid::Uuid,
    pub label: Option<String>,
    /// Ticket cross-ref: bookings land back on the originating ticket.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<TicketId>,
    pub status: SchedulingLinkStatus,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = "date-time"))]
    pub expires_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = "date-time"))]
    pub first_viewed_at: Option<DateTime<Utc>>,
    pub view_count: i64,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = "date-time"))]
    pub scheduled_at: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub scheduled_appointment_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub created_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub created_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub updated_at: DateTime<Utc>,
}

/// The link → booking trail row (mirrors the appointment for audit).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SchedulingLinkBooking {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub link_id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub appointment_id: uuid::Uuid,
    pub booker_name: Option<String>,
    pub booker_email: Option<String>,
    pub booker_notes: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub created_at: DateTime<Utc>,
}

impl MeetingType {
    /// Structural validation for API writes: sane durations, a scope
    /// target matching the declared scope, and auto-email consistency.
    pub fn validate(&self) -> Result<(), crate::error::DomainError> {
        if self.name.trim().is_empty() {
            return Err(crate::error::DomainError::Validation(
                "meeting type name must not be empty".into(),
            ));
        }
        if self.buffer_minutes < 0 {
            return Err(crate::error::DomainError::Validation(
                "buffer_minutes must be >= 0".into(),
            ));
        }
        if self.duration_minutes < 5 || self.duration_minutes > 480 {
            return Err(crate::error::DomainError::Validation(
                "duration_minutes must be 5-480".into(),
            ));
        }
        if !(1..=365).contains(&self.booking_window_days) {
            return Err(crate::error::DomainError::Validation(
                "booking_window_days must be 1-365".into(),
            ));
        }
        match self.assignee_scope {
            MeetingAssigneeScope::User if self.assignee_user_id.is_some() => {}
            MeetingAssigneeScope::Group if self.assignee_group_id.is_some() => {}
            _ => {
                return Err(crate::error::DomainError::Validation(
                    "assignee target must match the declared scope".into(),
                ))
            }
        }
        if self.auto_email_enabled
            && (self
                .auto_email_subject
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
                || self
                    .auto_email_body
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty())
        {
            return Err(crate::error::DomainError::Validation(
                "auto-email requires subject and body".into(),
            ));
        }
        Ok(())
    }
}
