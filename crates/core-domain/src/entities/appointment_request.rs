//! Portal appointment requests (the private build tracker Phase 3, Portal v2): the intent
//! row between a client portal user and a staff calendar.
//!
//! A request is NOT a booking: several clients may hold overlapping
//! pending requests and nothing here occupies a calendar. Only the staff
//! approval decision creates the real `Appointment` (and the request
//! links it); decline and client cancel are terminal without any
//! calendar effect. Status moves forward only — history is never
//! rewritten.

use crate::entities::AppointmentKind;
use crate::error::DomainError;
use crate::ids::{ClientId, OrgId, UserId};
use chrono::{DateTime, Duration, Utc};

/// Lifecycle of an appointment request. `approved`/`declined`/`cancelled`
/// are terminal; approved requests carry the created appointment's id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AppointmentRequestStatus {
    Pending,
    Approved,
    Declined,
    Cancelled,
}

impl AppointmentRequestStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Declined => "declined",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => Self::Pending,
            "approved" => Self::Approved,
            "declined" => Self::Declined,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

/// A client's request for a slot on one staff member's calendar.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AppointmentRequest {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: uuid::Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: OrgId,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub client_id: ClientId,
    /// The requesting portal identity (NOT a staff row).
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub portal_user_id: uuid::Uuid,
    /// The staff member whose calendar was requested.
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: UserId,
    pub title: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub starts_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub ends_at: DateTime<Utc>,
    pub kind: AppointmentKind,
    pub notes: Option<String>,
    pub status: AppointmentRequestStatus,
    /// Note attached on approve/decline.
    pub decision_note: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub decided_by: Option<UserId>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = "date-time"))]
    pub decided_at: Option<DateTime<Utc>>,
    /// The real booking created by approval (null until then).
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub appointment_id: Option<uuid::Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub created_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub updated_at: DateTime<Utc>,
}

impl AppointmentRequest {
    /// Structural validation for the portal create path: sane span, not
    /// in the past, not absurdly far ahead. Availability containment and
    /// conflict checks live in the API layer against live rows, and the
    /// approval path re-validates everything at booking time.
    pub fn validate_new(
        &self,
        now: DateTime<Utc>,
        max_duration: Duration,
        max_lead: Duration,
    ) -> Result<(), DomainError> {
        if self.title.trim().is_empty() {
            return Err(DomainError::Validation("title is required".into()));
        }
        if self.ends_at <= self.starts_at {
            return Err(DomainError::Validation(
                "ends_at must be after starts_at".into(),
            ));
        }
        if self.ends_at - self.starts_at > max_duration {
            return Err(DomainError::Validation(format!(
                "requested span exceeds the {max_duration:?} cap"
            )));
        }
        if self.starts_at <= now {
            return Err(DomainError::Validation(
                "requested slot must start in the future".into(),
            ));
        }
        if self.starts_at > now + max_lead {
            return Err(DomainError::Validation(format!(
                "requested slot is more than {max_lead:?} ahead; pick a nearer date"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(start: DateTime<Utc>, end: DateTime<Utc>) -> AppointmentRequest {
        let now = Utc::now();
        AppointmentRequest {
            id: uuid::Uuid::now_v7(),
            org_id: uuid::Uuid::now_v7(),
            client_id: uuid::Uuid::now_v7(),
            portal_user_id: uuid::Uuid::now_v7(),
            user_id: uuid::Uuid::now_v7(),
            title: "Firewall review".into(),
            starts_at: start,
            ends_at: end,
            kind: AppointmentKind::Remote,
            notes: None,
            status: AppointmentRequestStatus::Pending,
            decision_note: None,
            decided_by: None,
            decided_at: None,
            appointment_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn validate(r: &AppointmentRequest) -> Result<(), DomainError> {
        let now = Utc::now();
        r.validate_new(now, Duration::hours(8), Duration::days(90))
    }

    #[test]
    fn accepts_sane_future_span() {
        let start = Utc::now() + Duration::hours(48);
        assert!(validate(&request(start, start + Duration::hours(1))).is_ok());
    }

    #[test]
    fn rejects_past_inverted_overlong_and_too_far() {
        let n = Utc::now();
        // In the past.
        let past = request(n - Duration::hours(2), n - Duration::hours(1));
        assert!(validate(&past).is_err());
        // Inverted.
        let inverted = request(n + Duration::hours(5), n + Duration::hours(4));
        assert!(validate(&inverted).is_err());
        // Over the duration cap.
        let overlong = request(n + Duration::hours(10), n + Duration::hours(20));
        assert!(validate(&overlong).is_err());
        // Beyond the lead-time cap.
        let far = request(
            n + Duration::days(120),
            n + Duration::days(120) + Duration::hours(1),
        );
        assert!(validate(&far).is_err());
    }

    #[test]
    fn status_round_trip() {
        for s in [
            AppointmentRequestStatus::Pending,
            AppointmentRequestStatus::Approved,
            AppointmentRequestStatus::Declined,
            AppointmentRequestStatus::Cancelled,
        ] {
            assert_eq!(AppointmentRequestStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(AppointmentRequestStatus::parse("maybe"), None);
    }
}
