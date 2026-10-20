//! Scheduling entities (the private build tracker 2.12): appointments, availability
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

/// The resolved journey start point a travel estimate ran from (Wave 32
/// lane 7). Snapshotted onto the appointment so the estimate stays
/// explainable after the start point or the site changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TravelOrigin {
    /// `home` | `office` | `site` | `address`.
    pub kind: String,
    pub label: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub site_id: Option<uuid::Uuid>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// How it was resolved: `explicit` (dispatcher-set day start) or
    /// `previous_day_site` (auto-suggested continuity).
    pub source: Option<String>,
}

/// A booked slot on one user's calendar, optionally tied to a ticket and
/// client/site. `travel_minutes` is travel time booked immediately BEFORE
/// `starts_at` (Wave 32 lane 7): it ADDS to the appointment and occupies
/// dispatch capacity (`crate::scheduling::occupied_minutes`), never
/// replacing or shrinking the booked span. The `travel_*` fields carry
/// the estimate's provenance so a heuristic number is never mistaken for
/// a measured one.
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
    /// Where the estimate travelled FROM; `None` when no provider
    /// provenance exists (manual minutes or no estimate).
    #[serde(default)]
    pub travel_from: Option<TravelOrigin>,
    /// Provider-reported leg distance in metres.
    #[serde(default)]
    pub travel_distance_meters: Option<i32>,
    /// Provider id that produced `travel_minutes` (`none` = built-in
    /// heuristic); `None` for manual values.
    #[serde(default)]
    pub travel_provider: Option<String>,
    /// `heuristic` | `measured`; `None` whenever `travel_provider` is.
    #[serde(default)]
    pub travel_quality: Option<String>,
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
