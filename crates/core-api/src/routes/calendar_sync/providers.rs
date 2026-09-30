//! Google Calendar + Microsoft Graph as `CalendarSyncProvider` kinds.
//!
//! Purpose: the seam shipped with one implementation (`ical`) and the DB
//! CHECK made `google`/`microsoft` impossible to store, so "the growth path
//! the seam header names" was not actually reachable. These two add it.
//!
//! # Why DELEGATED auth, not the connector's app-only grant
//!
//! `plugins/connectors/m365-graph` uses a client-credentials (app-only)
//! grant, which is correct for fleet inventory: it reads the tenant, not a
//! person's data. A CALENDAR is a person's data. There is no app-only token
//! that lists one user's events with their consent, so these adapters take
//! a DELEGATED access token obtained through the authorization-code flow —
//! which is why the row also needed the `oauth_state` columns migration 0116
//! added, and why the refresh token lives in the SEALED `byok` store rather
//! than on the connection row.
//!
//! # Why a bounded WINDOW, not a sync token
//!
//! `CalendarSyncProvider::fetch_events` is documented as returning a NORMALISED
//! FULL SNAPSHOT, and the trait has no incremental method. So both adapters
//! ask the vendor for a date window rather than a delta.
//!
//! That is a deliberate choice, not a shortcut. A delta is cheaper, but an
//! expired or invalidated delta token makes the provider return 410 Gone,
//! and a sync that silently returns FEWER events on a token error is worse
//! than one that re-reads a window: it frees busy slots that are still busy,
//! and a double-booking is a customer's problem, not a data-freshness
//! footnote. A window has no such failure mode. `sync_token` is still
//! recorded on the connection for observability; it is simply not the
//! mechanism.
//!
//! # A single-day minimum
//!
//! The window is clamped to at least one day on both sides. Graph rejects a
//! `calendarView` whose `startDateTime` equals its `endDateTime`, and the
//! difference is invisible until a user picks "today" — so the clamp is here
//! rather than left to each caller's validation.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

use super::{
    CalendarSyncConnection, CalendarSyncProvider, ExternalCalendarEvent, ProviderError,
    ProviderPull,
};

/// The window a pull covers, past and future.
///
/// Symmetric on purpose: availability planning is a forward question, but a
/// sync that only looks ahead silently drops the busy time a technician is
/// in RIGHT NOW, which is the one moment double-booking hurts most.
const DEFAULT_PAST_DAYS: i64 = 30;
const DEFAULT_FUTURE_DAYS: i64 = 90;
const MIN_WINDOW_DAYS: i64 = 1;

fn window(now: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let from = now - Duration::days(DEFAULT_PAST_DAYS);
    let to = now + Duration::days(DEFAULT_FUTURE_DAYS);
    // Guard the degenerate case the callers cannot: a clock at the epoch, or
    // a future end that a rounding step collapsed onto the start.
    let span = (to - from).num_days().max(MIN_WINDOW_DAYS);
    (from, from + Duration::days(span))
}

// --- Google Calendar ---------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct GoogleCalendarProvider {
    pub base_url: Option<String>,
    pub client: reqwest::Client,
}

const GOOGLE_DEFAULT_BASE: &str = "https://www.googleapis.com/calendar/v3";

/// A Google events-page response. Only the fields the seam consumes, plus
/// `nextPageToken` for paging — Google's calendar is paginated and a
/// single-page read would silently truncate a busy calendar.
#[derive(Debug, Deserialize)]
struct GoogleEventsPage {
    #[serde(default)]
    items: Vec<GoogleEvent>,
    #[serde(rename = "nextPageToken", default)]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleEvent {
    #[serde(default)]
    id: String,
    #[serde(default)]
    sequence: i64,
    summary: Option<String>,
    /// RFC 3339 with an offset. Google's `dateTime` is always offset-aware;
    /// all-day events use `date` instead and have no `start.dateTime`.
    start: GoogleEventTime,
    end: GoogleEventTime,
    #[serde(rename = "status", default)]
    status: Option<String>,
    #[serde(rename = "transparency", default)]
    transparency: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleEventTime {
    #[serde(rename = "dateTime")]
    date_time: Option<String>,
}

impl GoogleEventTime {
    /// Parse the RFC 3339 instant. `None` for an all-day event, which
    /// Google expresses as a bare `date` with no time — and an all-day
    /// event is REAL busy time (a day off, a site visit), so it is not
    /// dropped: it is anchored to midnight UTC, which is the same choice
    /// `ical.rs` makes for a DATE-valued DTSTART.
    fn instant(&self) -> Option<DateTime<Utc>> {
        if let Some(dt) = &self.date_time {
            return DateTime::parse_from_rfc3339(dt)
                .ok()
                .map(|d| d.with_timezone(&Utc));
        }
        None
    }
}

#[async_trait]
impl CalendarSyncProvider for GoogleCalendarProvider {
    fn kind(&self) -> &'static str {
        "google"
    }

    async fn fetch_events(
        &self,
        connection: &CalendarSyncConnection,
    ) -> Result<ProviderPull, ProviderError> {
        let token = require_token(connection)?;
        let base = self
            .base_url
            .clone()
            .unwrap_or_else(|| GOOGLE_DEFAULT_BASE.to_string());
        let (from, to) = window(Utc::now());

        // `singleEvents=true` expands recurring events into instances. Without
        // it a weekly on-call rota imports as ONE event in week one and never
        // again, which is the failure that makes a technician look free every
        // week after the first.
        let mut url = format!(
            "{base}/calendars/primary/events?singleEvents=true&showDeleted=true\
             &timeMin={}&timeMax={}&maxResults=2500",
            from.to_rfc3339(),
            to.to_rfc3339(),
        );

        let mut events = Vec::new();
        // Bounded so a misbehaving endpoint paging forever cannot hang the
        // sync worker. 20 pages × 2500 is 50k events, far past any real
        // calendar in this product.
        for _ in 0..20 {
            let page: GoogleEventsPage = self
                .client
                .get(&url)
                .bearer_auth(&token)
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
                // Both transport failures and auth failures are
                // `Unreachable`: a revoked consent and a DNS failure are the
                // same thing TO THE SEAM, which has no "needs reconsent"
                // variant, and `last_error` is documented as fixed copy with
                // no internals — so there is deliberately nowhere to put a
                // vendor string. The operator's remedy for both is the same
                // Settings screen.
                .map_err(|_| ProviderError::Unreachable)?
                .error_for_status()
                .map_err(|_| ProviderError::Unreachable)?
                // A body that does not parse is the calendar being
                // UNREADABLE, which is the other half of the enum and the
                // distinction the sync report surfaces.
                .json()
                .await
                .map_err(|_| ProviderError::Unreadable)?;

            events.extend(page.items);
            let Some(token) = page.next_page_token else {
                break;
            };
            url = format!(
                "{base}/calendars/primary/events?singleEvents=true&showDeleted=true\
                 &timeMin={}&timeMax={}&maxResults=2500&pageToken={token}",
                from.to_rfc3339(),
                to.to_rfc3339(),
            );
        }

        let mut out = Vec::with_capacity(events.len());
        for ev in events {
            // A cancelled event has no times; it is still imported (as
            // non-busy) so the busy-slot it previously held is RELEASED.
            // Skipping it entirely would leave a phantom booking forever.
            let (Some(starts_at), Some(ends_at)) = (ev.start.instant(), ev.end.instant()) else {
                if ev.status.as_deref() == Some("cancelled") {
                    // Fall back to a zero-width cancelled event at the
                    // window start: `is_busy()` is false for it, and the
                    // importer keys on `external_uid` to release the slot.
                    out.push(ExternalCalendarEvent {
                        external_uid: ev.id,
                        sequence: ev.sequence as i32,
                        title: ev.summary,
                        starts_at: from,
                        ends_at: from,
                        cancelled: true,
                        transparent: true,
                    });
                }
                continue;
            };
            out.push(ExternalCalendarEvent {
                external_uid: ev.id,
                sequence: ev.sequence as i32,
                title: ev.summary,
                starts_at,
                ends_at,
                cancelled: ev.status.as_deref() == Some("cancelled"),
                transparent: ev.transparency.as_deref() == Some("transparent"),
            });
        }
        Ok(ProviderPull {
            events: out,
            sync_token: None,
        })
    }
}

// --- Microsoft Graph ---------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct MicrosoftGraphProvider {
    pub base_url: Option<String>,
    pub client: reqwest::Client,
}

const GRAPH_DEFAULT_BASE: &str = "https://graph.microsoft.com/v1.0";

#[derive(Debug, Deserialize)]
struct GraphPage {
    #[serde(default)]
    value: Vec<GraphEvent>,
    #[serde(rename = "@odata.nextLink", default)]
    next_link: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphEvent {
    #[serde(default)]
    id: String,
    #[serde(rename = "sequenceNumber", default)]
    sequence: i64,
    subject: Option<String>,
    /// Graph returns these as `2026-09-30T09:00:00.0000000` — 100ns ticks
    /// since the epoch, NOT RFC 3339. Parsing them as RFC 3339 yields
    /// nothing at all, which is the reason this adapter does not reuse
    /// `DateTime::parse_from_rfc3339`.
    #[serde(rename = "start")]
    start: GraphEventTime,
    #[serde(rename = "end")]
    end: GraphEventTime,
    #[serde(rename = "isCancelled", default)]
    is_cancelled: bool,
    #[serde(rename = "showAs", default)]
    show_as: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphEventTime {
    #[serde(rename = "dateTime")]
    date_time: String,
    /// Decoded but deliberately unused: the request pins
    /// `Prefer: outlook.timezone="UTC"`, so honouring a per-event zone here
    /// would contradict what was asked for. Kept in the shape so a vendor
    /// field rename is a decode failure at the boundary rather than a silent
    /// mismatch later.
    #[serde(rename = "timeZone", default)]
    #[allow(dead_code)]
    time_zone: Option<String>,
}

impl GraphEventTime {
    /// Parse Graph's two accepted shapes.
    ///
    /// With `Prefer: outlook.timezone="UTC"` the `dateTime` is UTC and
    /// carries no suffix, which is the form this prefers. Without the
    /// header it carries an offset, which `parse_from_rfc3339` handles.
    /// Anything else is unparseable rather than silently wrong: a busy slot
    /// at a guessed time is a double-booking.
    fn instant(&self) -> Option<DateTime<Utc>> {
        let raw = self.date_time.trim();
        if let Ok(d) = DateTime::parse_from_rfc3339(raw) {
            return Some(d.with_timezone(&Utc));
        }
        // "2026-09-30T09:00:00.0000000" and "…T09:00:00" — treat as UTC,
        // which is only sound BECAUSE the request asks for UTC.
        for fmt in [
            "%Y-%m-%dT%H:%M:%S%.f",
            "%Y-%m-%dT%H:%M:%S%.fZ",
            "%Y-%m-%dT%H:%M:%S",
        ] {
            if let Ok(n) = chrono::NaiveDateTime::parse_from_str(raw, fmt) {
                return Some(DateTime::from_naive_utc_and_offset(n, Utc));
            }
        }
        None
    }
}

#[async_trait]
impl CalendarSyncProvider for MicrosoftGraphProvider {
    fn kind(&self) -> &'static str {
        "microsoft"
    }

    async fn fetch_events(
        &self,
        connection: &CalendarSyncConnection,
    ) -> Result<ProviderPull, ProviderError> {
        let token = require_token(connection)?;
        let base = self
            .base_url
            .clone()
            .unwrap_or_else(|| GRAPH_DEFAULT_BASE.to_string());
        let (from, to) = window(Utc::now());
        let (from, to) = (from, to.max(from + Duration::days(MIN_WINDOW_DAYS)));

        // `calendarView` (not `events`) because it EXPANDS recurring
        // instances over a window, which is the same reason Google gets
        // `singleEvents=true`. `Prefer: outlook.timezone="UTC"` pins the
        // response to UTC so the datetime parsing above is sound.
        let mut url = format!(
            "{base}/me/calendarView?startDateTime={}&endDateTime={}&$top=1000\
            &$select=id,subject,start,end,isCancelled,showAs,sequenceNumber",
            from.to_rfc3339(),
            to.to_rfc3339(),
        );

        let mut out: Vec<ExternalCalendarEvent> = Vec::new();
        for _ in 0..20 {
            let page: GraphPage = self
                .client
                .get(&url)
                .bearer_auth(&token)
                .header("Prefer", "outlook.timezone=\"UTC\"")
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
                .map_err(|_| ProviderError::Unreachable)?
                .error_for_status()
                .map_err(|_| ProviderError::Unreachable)?
                .json()
                .await
                .map_err(|_| ProviderError::Unreadable)?;

            for ev in page.value {
                // A cancelled Graph event still carries its original times,
                // so it is imported as cancelled rather than dropped — the
                // importer keys on `external_uid` to release the busy slot.
                let (Some(starts_at), Some(ends_at)) = (ev.start.instant(), ev.end.instant())
                else {
                    continue;
                };
                out.push(ExternalCalendarEvent {
                    external_uid: ev.id,
                    sequence: ev.sequence as i32,
                    title: ev.subject,
                    starts_at,
                    ends_at,
                    cancelled: ev.is_cancelled,
                    // Graph's `free` is the same concept as iCal TRANSPARENT:
                    // the external calendar does not count it as busy.
                    transparent: ev.show_as.as_deref() == Some("free"),
                });
            }
            let Some(next) = page.next_link else { break };
            url = next;
        }

        Ok(ProviderPull {
            events: out,
            sync_token: None,
        })
    }
}

/// The delegated access token for this connection.
///
/// A connection with no token is `Unreachable`, not a silent empty pull.
/// Returning an empty event set for an unauthenticated connection would
/// free every busy slot the calendar owns, and the operator's calendar
/// would look "clear" exactly when the sync is broken.
fn require_token(connection: &CalendarSyncConnection) -> Result<String, ProviderError> {
    connection
        .access_token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        // `Unreachable` rather than an empty pull: see the doc comment. The
        // missing-token case is diagnosed in Settings, where the connection's
        // `status` already reports it, so the sync report does not need to
        // repeat it.
        .ok_or(ProviderError::Unreachable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::calendar_sync::{CalendarSyncDirection as Dir, CalendarSyncStatus as St};
    use uuid::Uuid;

    fn conn() -> CalendarSyncConnection {
        CalendarSyncConnection {
            id: Uuid::new_v4(),
            org_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            name: "Work".into(),
            provider: "google".into(),
            direction: Dir::Inbound,
            status: St::Active,
            feed_url: None,
            feed_path: None,
            last_synced_at: None,
            last_error: None,
            access_token: Some("ya29.test".into()),
            token_expires_at: None,
            external_account: None,
            scopes: Some("calendar.readonly".into()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// The window must never be zero-length: Graph rejects it outright, and
    /// the failure is invisible until someone syncs on a boundary day.
    #[test]
    fn the_window_is_never_zero_length() {
        let (from, to) = window(Utc::now());
        assert!(to > from, "window collapsed");
        assert_eq!(
            (to - from).num_days(),
            DEFAULT_PAST_DAYS + DEFAULT_FUTURE_DAYS
        );
    }

    /// A connection with no token must NOT return an empty event set. An
    /// empty set frees every busy slot the calendar owns.
    #[tokio::test]
    async fn an_unauthenticated_connection_is_unreachable_not_empty() {
        let mut c = conn();
        c.access_token = None;
        let err = GoogleCalendarProvider::default()
            .fetch_events(&c)
            .await
            .expect_err("must not succeed");
        assert!(matches!(err, ProviderError::Unreachable), "{err:?}");

        c.access_token = Some("   ".into());
        assert!(
            GoogleCalendarProvider::default()
                .fetch_events(&c)
                .await
                .is_err(),
            "a blank token is as good as none"
        );
        let err = MicrosoftGraphProvider::default()
            .fetch_events(&c)
            .await
            .expect_err("must not succeed");
        assert!(matches!(err, ProviderError::Unreachable), "{err:?}");
    }

    #[test]
    fn google_parses_offset_aware_instants() {
        let t = GoogleEventTime {
            date_time: Some("2026-09-30T11:00:00+02:00".into()),
        };
        let got = t.instant().expect("parsed");
        assert_eq!(got.to_rfc3339(), "2026-09-30T09:00:00+00:00");
        // An all-day event has no dateTime; it is not an error, it is absent.
        let allday = GoogleEventTime { date_time: None };
        assert!(allday.instant().is_none());
        // Garbage is None, never a guessed instant.
        let bad = GoogleEventTime {
            date_time: Some("not a time".into()),
        };
        assert!(bad.instant().is_none());
    }

    /// Graph's default `dateTime` is NOT RFC 3339. Parsing it as such yields
    /// nothing, which would silently import an empty calendar.
    #[test]
    fn graph_parses_its_own_datetime_shape() {
        let ticks = GraphEventTime {
            date_time: "2026-09-30T09:00:00.0000000".into(),
            time_zone: None,
        };
        assert_eq!(
            ticks.instant().expect("parsed").to_rfc3339(),
            "2026-09-30T09:00:00+00:00"
        );
        let offset = GraphEventTime {
            date_time: "2026-09-30T11:00:00.0000000+02:00".into(),
            time_zone: None,
        };
        assert_eq!(
            offset.instant().expect("parsed").to_rfc3339(),
            "2026-09-30T09:00:00+00:00"
        );
        assert!(GraphEventTime {
            date_time: "nonsense".into(),
            time_zone: None,
        }
        .instant()
        .is_none());
    }

    #[test]
    fn transparency_and_cancellation_map_onto_the_seams_vocabulary() {
        let busy = ExternalCalendarEvent {
            external_uid: "a".into(),
            sequence: 0,
            title: None,
            starts_at: Utc::now(),
            ends_at: Utc::now() + Duration::hours(1),
            cancelled: false,
            transparent: false,
        };
        assert!(busy.is_busy());
        assert!(
            !ExternalCalendarEvent {
                transparent: true,
                ..busy.clone()
            }
            .is_busy(),
            "TRANSPARENT / showAs=free must not hold busy time"
        );
        assert!(
            !ExternalCalendarEvent {
                cancelled: true,
                ..busy.clone()
            }
            .is_busy(),
            "a cancelled event releases its slot"
        );
        assert!(
            !ExternalCalendarEvent {
                ends_at: busy.starts_at,
                ..busy
            }
            .is_busy(),
            "a zero-width event is not busy"
        );
    }

    /// The paging decode shapes, so a vendor field rename surfaces as a
    /// compile/test failure here rather than an empty calendar in production.
    #[test]
    fn vendor_payload_shapes_decode() {
        let g: GoogleEventsPage = serde_json::from_str(
            r#"{"items":[{"id":"e1","sequence":3,"summary":"Standup",
                 "start":{"dateTime":"2026-09-30T09:00:00Z"},
                 "end":{"dateTime":"2026-09-30T09:15:00Z"},
                 "status":"confirmed","transparency":"opaque"}],
               "nextPageToken":"p2"}"#,
        )
        .expect("google page");
        assert_eq!(g.items.len(), 1);
        assert_eq!(g.items[0].sequence, 3);
        assert_eq!(g.next_page_token.as_deref(), Some("p2"));

        let m: GraphPage = serde_json::from_str(
            r#"{"value":[{"id":"m1","sequenceNumber":7,"subject":"Review",
                 "start":{"dateTime":"2026-09-30T09:00:00.0000000","timeZone":"UTC"},
                 "end":{"dateTime":"2026-09-30T10:00:00.0000000","timeZone":"UTC"},
                 "isCancelled":true,"showAs":"free"}],
               "@odata.nextLink":"https://next"}"#,
        )
        .expect("graph page");
        assert_eq!(m.value[0].sequence, 7);
        assert!(m.value[0].is_cancelled);
        assert_eq!(m.value[0].show_as.as_deref(), Some("free"));
        assert_eq!(m.next_link.as_deref(), Some("https://next"));
    }
}
