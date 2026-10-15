//! Two-way calendar sync entities (Wave 14 lane 2).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What a connection does: `outbound` serves the org's appointments to
/// the external calendar (private token-scoped iCal feed), `inbound`
/// polls the external feed for busy time, `two_way` does both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CalendarSyncDirection {
    Outbound,
    Inbound,
    TwoWay,
}

impl CalendarSyncDirection {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "outbound" => Some(Self::Outbound),
            "inbound" => Some(Self::Inbound),
            "two_way" => Some(Self::TwoWay),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Outbound => "outbound",
            Self::Inbound => "inbound",
            Self::TwoWay => "two_way",
        }
    }
}

/// Connection health: `error` records the last poll failure while the
/// connection stays configured; writes resume when a poll succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CalendarSyncStatus {
    Active,
    Error,
    Disabled,
}

impl CalendarSyncStatus {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "error" => Some(Self::Error),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Error => "error",
            Self::Disabled => "disabled",
        }
    }
}

/// One external-calendar connection for one user's calendar.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CalendarSyncConnection {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: Uuid,
    pub name: String,
    /// The provider seam discriminator ('ical' is the first-party provider).
    pub provider: String,
    pub direction: CalendarSyncDirection,
    pub status: CalendarSyncStatus,
    /// Inbound: the external iCal URL the provider polls.
    pub feed_url: Option<String>,
    /// Outbound: relative feed path clients render against the API
    /// origin; the raw token itself is never readable again.
    pub feed_path: Option<String>,
    /// RFC 3339 timestamp of the last successful poll.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = "date-time"))]
    pub last_synced_at: Option<DateTime<Utc>>,
    /// Last poll failure message (fixed provider-side copy, no internals).
    pub last_error: Option<String>,
    // --- delegated OAuth (Wave 34 claims close-out, claim 5) --------------
    //
    // `ical` needs none of these and leaves them NULL. `google` and
    // `microsoft` are DELEGATED: they read one person's calendar, which no
    // app-only grant can do (see providers.rs for why the m365 connector's
    // client-credentials flow is the wrong one here).
    //
    // The CACHED short-lived access token lives here, not in the sealed
    // store: it is regenerated hourly from the sealed refresh token, and
    // refreshing on every poll would make the poll endpoint a proxy for the
    // vendor's token service. The REFRESH TOKEN — a long-lived bearer that
    // can read the user's whole calendar — is sealed in `byok_secrets` under
    // `calendar.google` / `calendar.microsoft` and is never on this row.
    /// Cached access token, valid until `token_expires_at`.
    pub access_token: Option<String>,
    /// Expiry of `access_token`. CHECK-paired with the token so a token row
    /// can never be read as valid-forever.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = "date-time"))]
    pub token_expires_at: Option<DateTime<Utc>>,
    /// The provider's own account id (Google `sub`, Microsoft `oid`).
    /// Recorded so an operator auditing a sync can tell WHICH account the
    /// connection is bound to. Not an identity Marula authenticates with.
    pub external_account: Option<String>,
    /// Scopes actually granted, recorded so the consent screen and the
    /// stored grant can be compared. Never widened after consent.
    pub scopes: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub created_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub updated_at: DateTime<Utc>,
}

impl CalendarSyncConnection {
    /// The providers that read through a delegated OAuth grant rather than a
    /// polled `.ics` URL. They have no `feed_url` by design — their events
    /// arrive from the vendor API — so the feed_url rule in `validate` does
    /// not apply to them.
    fn is_delegated(&self) -> bool {
        matches!(self.provider.as_str(), "google" | "microsoft")
    }

    pub fn validate(&self) -> Result<(), crate::error::DomainError> {
        if self.name.trim().is_empty() {
            return Err(crate::error::DomainError::Validation(
                "connection name must not be empty".into(),
            ));
        }
        // A `feed_url` is required only where it is actually read: an ical
        // connection that pulls from (or is two-way with) an external URL. An
        // outbound-only connection publishes this org's own feed and has no
        // external URL; a delegated provider has no URL at all. Requiring one
        // for a delegated provider is what made a Google/Microsoft connection
        // impossible to create at all.
        if !self.is_delegated()
            && matches!(
                self.direction,
                CalendarSyncDirection::Inbound | CalendarSyncDirection::TwoWay
            )
            && !self
                .feed_url
                .as_deref()
                .map(str::trim)
                .is_some_and(|u| u.starts_with("http://") || u.starts_with("https://"))
        {
            return Err(crate::error::DomainError::Validation(
                "inbound and two-way connections require an http(s) feed_url".into(),
            ));
        }
        Ok(())
    }
}

/// The inbound mirror of one external event. `exclusion_id` names the
/// `calendar_exclusions` row the event currently feeds into the 2.12
/// availability foundation (NULL while not busy — cancelled or transparent).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CalendarSyncEvent {
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub org_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub connection_id: Uuid,
    #[cfg_attr(feature = "openapi", schema(value_type = uuid::Uuid))]
    pub user_id: Uuid,
    /// The external event's UID — the duplicate key per connection.
    pub external_uid: String,
    /// Highest external sequence wins on re-poll.
    pub sequence: i32,
    pub title: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub starts_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub ends_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<uuid::Uuid>))]
    pub exclusion_id: Option<Uuid>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub first_seen_at: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = "date-time"))]
    pub last_seen_at: DateTime<Utc>,
}

/// One externally-pulled event, provider-normalized (before the mirror).
#[derive(Debug, Clone)]
pub struct ExternalCalendarEvent {
    pub external_uid: String,
    pub sequence: i32,
    pub title: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    /// STATUS:CANCELLED → not busy; the slot frees again.
    pub cancelled: bool,
    /// TRANSP:TRANSPARENT → the external calendar does not count this
    /// as busy time, so neither do we.
    pub transparent: bool,
}

impl ExternalCalendarEvent {
    /// Does this event feed busy time into availability?
    pub fn is_busy(&self) -> bool {
        !self.cancelled && !self.transparent && self.ends_at > self.starts_at
    }
}

/// What one sync run did: the import counters plus the duplicate/conflict
/// bookkeeping the API reports back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncOutcome {
    /// Events newly imported as busy time.
    pub imported: u32,
    /// Previously-known events whose span/title changed (sequence up).
    pub updated: u32,
    /// Events that disappeared or became cancelled — exclusions removed,
    /// slots freed.
    pub released: u32,
    /// Imported events overlapping a live appointment on the same
    /// calendar (conflict rule: reported, never blocked).
    pub overlaps: u32,
    /// Same-UID re-polls resolved by the highest-sequence-wins rule.
    pub duplicates: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(
        provider: &str,
        direction: CalendarSyncDirection,
        feed_url: Option<&str>,
    ) -> CalendarSyncConnection {
        CalendarSyncConnection {
            id: Uuid::nil(),
            org_id: Uuid::nil(),
            user_id: Uuid::nil(),
            name: "Work calendar".into(),
            provider: provider.into(),
            direction,
            status: CalendarSyncStatus::Active,
            feed_url: feed_url.map(str::to_string),
            feed_path: None,
            last_synced_at: None,
            last_error: None,
            access_token: None,
            token_expires_at: None,
            external_account: None,
            scopes: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// A delegated provider has no `.ics` URL — its events arrive from the
    /// vendor API after consent. Requiring a feed_url for it made a
    /// Google/Microsoft connection impossible to create (422 at the route).
    #[test]
    fn delegated_providers_need_no_feed_url() {
        for provider in ["google", "microsoft"] {
            assert!(
                connection(provider, CalendarSyncDirection::Inbound, None)
                    .validate()
                    .is_ok(),
                "{provider} inbound must validate without a feed_url"
            );
        }
    }

    #[test]
    fn ical_inbound_still_requires_an_http_feed_url() {
        assert!(connection("ical", CalendarSyncDirection::Inbound, None)
            .validate()
            .is_err());
        assert!(
            connection("ical", CalendarSyncDirection::Inbound, Some("ftp://host/x"))
                .validate()
                .is_err(),
            "a non-http scheme is not a feed_url"
        );
        assert!(connection(
            "ical",
            CalendarSyncDirection::Inbound,
            Some("https://host/calendar.ics")
        )
        .validate()
        .is_ok());
    }

    #[test]
    fn outbound_needs_no_feed_url() {
        assert!(connection("ical", CalendarSyncDirection::Outbound, None)
            .validate()
            .is_ok());
    }
}
