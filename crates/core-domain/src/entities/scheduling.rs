//! Scheduling entities (TASKLIST 2.12): appointments, availability
//! windows, on-call blocks, and calendar exclusions — the serde shapes
//! shared by DB mapping and the API.
//!
//! Non-purpose: no overlap/containment math here; rules live in
//! `core_domain::scheduling`.

use crate::ids::{ClientId, OrgId, SiteId, TicketId, UserId};
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde::{Deserialize, Serialize};

/// What an appointment is physically for. Drives rendering color-coding on
/// calendars and future travel-time logic (on_site only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AppointmentKind {
    OnSite,
    Remote,
    Internal,
    Personal,
}

impl AppointmentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OnSite => "on_site",
            Self::Remote => "remote",
            Self::Internal => "internal",
            Self::Personal => "personal",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "on_site" => Some(Self::OnSite),
            "remote" => Some(Self::Remote),
            "internal" => Some(Self::Internal),
            "personal" => Some(Self::Personal),
            _ => None,
        }
    }
}

/// Lifecycle of a booking. `cancelled` frees the slot (the DB exclusion
/// constraint's partial predicate); `done` still occupies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AppointmentStatus {
    Scheduled,
    InProgress,
    Done,
    Cancelled,
}

impl AppointmentStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "scheduled" => Some(Self::Scheduled),
            "in_progress" => Some(Self::InProgress),
            "done" => Some(Self::Done),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// A booked slot on one user's calendar, optionally tied to a ticket and
/// client/site. `travel_minutes` is a dispatch-board placeholder — never
/// interpreted by scheduling rules in this stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Appointment {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub ticket_id: Option<TicketId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub client_id: Option<ClientId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub site_id: Option<SiteId>,
    pub title: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub starts_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub ends_at: DateTime<Utc>,
    pub kind: AppointmentKind,
    pub status: AppointmentStatus,
    pub travel_minutes: i32,
    pub notes: Option<String>,
    /// Set when the booking came through a Calendly-style scheduling
    /// link (Wave 11 lane 4); NULL for every other creation path.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub scheduling_link_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub created_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub updated_at: DateTime<Utc>,
}

/// One recurring weekly availability template: `weekday` (0 = Monday …
/// 6 = Sunday), local wall-clock `[start_local, end_local)` in `timezone`,
/// optionally bounded by an effective date range. This is a TEMPLATE —
/// occurrences are computed, never stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AvailabilityWindow {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    pub weekday: i16,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub start_local: NaiveTime,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub end_local: NaiveTime,
    /// IANA timezone the wall-clock bounds live in.
    pub timezone: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub effective_from: Option<NaiveDate>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub effective_until: Option<NaiveDate>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub created_at: DateTime<Utc>,
}

/// An absolute UTC on-call rotation segment for one user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OnCallBlock {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub starts_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub ends_at: DateTime<Utc>,
    /// Free-form rotation label ('primary', 'secondary', 'escalation', …).
    pub category: String,
    pub notes: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub created_at: DateTime<Utc>,
}

/// Absolute UTC time-off punching holes in availability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CalendarExclusion {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub starts_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub ends_at: DateTime<Utc>,
    pub reason: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_and_status_round_trip() {
        for k in [
            AppointmentKind::OnSite,
            AppointmentKind::Remote,
            AppointmentKind::Internal,
            AppointmentKind::Personal,
        ] {
            assert_eq!(AppointmentKind::parse(k.as_str()), Some(k));
            assert_eq!(
                serde_json::from_value::<AppointmentKind>(serde_json::to_value(k).unwrap())
                    .unwrap(),
                k
            );
        }
        for s in [
            AppointmentStatus::Scheduled,
            AppointmentStatus::InProgress,
            AppointmentStatus::Done,
            AppointmentStatus::Cancelled,
        ] {
            assert_eq!(AppointmentStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(AppointmentKind::parse("telepathy"), None);
        assert_eq!(AppointmentStatus::parse("maybe"), None);
    }
}
