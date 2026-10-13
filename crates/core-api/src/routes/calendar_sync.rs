//! Two-way calendar sync (Wave 14 lane 2): the `CalendarSyncProvider`
//! adapter seam, the first-party iCal provider, and the connection /
//! poll / feed surfaces.
//!
//! Purpose: BIDIRECTIONAL sync between a user's Marula calendar and an
//! external iCal feed. OUTBOUND: a private token-scoped iCal feed of the
//! user's live appointments (the external calendar subscribes to it —
//! the raw token is shown exactly once at creation/rotation and only its
//! SHA-256 hash is stored). INBOUND: a poll of the external iCal URL
//! mirrors external events and feeds their busy time into the 2.12
//! availability foundation — each non-cancelled, non-transparent event
//! maps onto a `calendar_exclusions` row, so the SAME book-time
//! double-check portal appointment requests and scheduling-link bookings
//! run (windows − on-call − time-off − bookings, re-read inside the
//! booking transaction) automatically honors external busy time. No
//! second conflict algorithm; overlap-with-appointments is reported in
//! `calendar.sync_completed.overlaps`, never blocked.
//!
//! The provider seam: [`CalendarSyncProvider`] is the adapter interface —
//! `provider_for` is the only place a `provider` discriminator becomes an
//! implementation, so Google/O365 providers land by adding an adapter,
//! not by reshaping sync state. The first-party iCal provider is
//! deliberately dependency-free: a small strict parser (UTC `...Z`
//! datetimes only) and serializer; anything it cannot place deterministically
//! it skips rather than guessing.
//!
//! Non-purpose: recurring-event expansion (RRULE — skipped events, next
//! provider), OAuth calendar providers (the seam exists for them),
//! reminders, the UI surfaces (Wave 14 lane 7 wires these through the
//! SDK), push/webhook subscriptions.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::routes::scheduling::{sched, target_user};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::{header::CONTENT_TYPE, Extensions, StatusCode},
    response::IntoResponse,
    Json,
};
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use core_domain::entities::{
    AppointmentStatus, CalendarSyncConnection, CalendarSyncDirection, CalendarSyncStatus,
    ExternalCalendarEvent,
};
use core_domain::events::{CalendarSyncCompleted, CalendarSyncConnectionUpdated};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use utoipa::ToSchema;
use uuid::Uuid;

// --- the provider seam -----------------------------------------------------------

/// What one provider pull produced.
#[derive(Debug)]
pub struct ProviderPull {
    pub events: Vec<ExternalCalendarEvent>,
    /// Opaque provider continuation (ETag / sync token). None when the
    /// provider is snapshot-only (iCal is).
    pub sync_token: Option<String>,
}

/// Provider-side failure with fixed client copy — never internals.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("the external calendar is unreachable")]
    Unreachable,
    #[error("the external calendar could not be read")]
    Unreadable,
}

/// The adapter seam: one implementation per external calendar system.
/// `fetch_events` returns the provider's normalized FULL snapshot of the
/// external calendar; the mirror/apply layer (`core_db::calendar_sync`)
/// owns duplicate, conflict, and release rules.
#[async_trait::async_trait]
pub trait CalendarSyncProvider: Send + Sync {
    fn kind(&self) -> &'static str;
    async fn fetch_events(
        &self,
        connection: &CalendarSyncConnection,
    ) -> Result<ProviderPull, ProviderError>;
}

/// The ONLY place a provider discriminator becomes an implementation.
/// Unknown providers fail closed (they cannot exist today — the DB CHECK
/// admits only 'ical' — but a future migration adding a provider must
/// add its seam arm here in the same change).
fn provider_for(connection: &CalendarSyncConnection) -> ApiResult<Box<dyn CalendarSyncProvider>> {
    match connection.provider.as_str() {
        "ical" => Ok(Box::new(ICalProvider)),
        // Delegated-OAuth providers. Both read a DATE WINDOW rather than a
        // delta: the seam's contract is a normalised FULL SNAPSHOT, and an
        // invalidated delta token makes a vendor return 410 with FEWER
        // events — which frees busy slots that are still busy. A window has
        // no such failure mode. See providers.rs.
        "google" => Ok(Box::new(GoogleCalendarProvider::default())),
        "microsoft" => Ok(Box::new(MicrosoftGraphProvider::default())),
        other => {
            tracing::error!(provider = other, "unknown calendar-sync provider");
            Err(ApiError::Internal(anyhow::anyhow!(
                "unknown calendar-sync provider"
            )))
        }
    }
}

mod providers;

pub use providers::{GoogleCalendarProvider, MicrosoftGraphProvider};

// --- first-party iCal provider ----------------------------------------------------

/// The first-party provider: plain HTTP GET of an iCalendar resource.
#[derive(Debug, Clone, Default)]
pub struct ICalProvider;

#[async_trait::async_trait]
impl CalendarSyncProvider for ICalProvider {
    fn kind(&self) -> &'static str {
        "ical"
    }

    async fn fetch_events(
        &self,
        connection: &CalendarSyncConnection,
    ) -> Result<ProviderPull, ProviderError> {
        let url = connection
            .feed_url
            .as_deref()
            .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
            .ok_or(ProviderError::Unreachable)?;
        let response = reqwest::Client::new()
            .get(url)
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "calendar-sync poll failed");
                ProviderError::Unreachable
            })?;
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "calendar-sync poll non-200");
            return Err(ProviderError::Unreachable);
        }
        let sync_token = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body = response
            .text()
            .await
            .map_err(|_| ProviderError::Unreadable)?;
        Ok(ProviderPull {
            events: parse_ical(&body),
            sync_token,
        })
    }
}

// --- iCal parsing (inbound) -------------------------------------------------------

/// Unfolds RFC 5545 folded lines (CRLF followed by SPACE/TAB) and splits
/// into content lines.
fn unfold_lines(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for raw in body.split(['\r', '\n']).filter(|l| !l.is_empty()) {
        if raw.starts_with(' ') || raw.starts_with('\t') {
            current.push_str(&raw[1..]);
        } else {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            current = raw.to_string();
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Parses the strict subset of iCalendar this provider understands:
/// VEVENT blocks with UTC `YYYYMMDDTHHMMSSZ` DTSTART/DTEND. Events that
/// cannot be placed deterministically (TZID-naive, DATE-only, RRULE,
/// missing DTEND) are SKIPPED — never guessed at. DTEND defaults to
/// DTSTART + 1h for the rare ZDT-endless form? No: a VEVENT without a
/// usable end is skipped, full stop.
pub fn parse_ical(body: &str) -> Vec<ExternalCalendarEvent> {
    let mut events = Vec::new();
    let mut in_event = false;
    let mut uid = String::new();
    let mut sequence = 0_i32;
    let mut title: Option<String> = None;
    let mut dtstart: Option<DateTime<Utc>> = None;
    let mut dtend: Option<DateTime<Utc>> = None;
    let mut cancelled = false;
    let mut transparent = false;

    for line in unfold_lines(body) {
        let Some((name, params, value)) = split_property(&line) else {
            continue;
        };
        match (in_event, name) {
            (false, "BEGIN") if value.eq_ignore_ascii_case("VEVENT") => {
                in_event = true;
                uid.clear();
                sequence = 0;
                title = None;
                dtstart = None;
                dtend = None;
                cancelled = false;
                transparent = false;
            }
            (true, "END") if value.eq_ignore_ascii_case("VEVENT") => {
                if let (Some(starts_at), Some(ends_at)) = (dtstart, dtend) {
                    if !uid.is_empty() {
                        events.push(ExternalCalendarEvent {
                            external_uid: uid.clone(),
                            sequence,
                            title: title.clone(),
                            starts_at,
                            ends_at,
                            cancelled,
                            transparent,
                        });
                    }
                }
                in_event = false;
            }
            (true, "UID") => uid = value.to_string(),
            (true, "SEQUENCE") => sequence = value.trim().parse().unwrap_or(0),
            (true, "SUMMARY") => title = Some(unescape_text(value)),
            (true, "STATUS") if value.eq_ignore_ascii_case("CANCELLED") => cancelled = true,
            (true, "TRANSP") if value.eq_ignore_ascii_case("TRANSPARENT") => transparent = true,
            (true, "DTSTART") if params.iter().all(|p| !p.starts_with("TZID=")) => {
                dtstart = parse_ical_utc(value);
            }
            (true, "DTEND") if params.iter().all(|p| !p.starts_with("TZID=")) => {
                dtend = parse_ical_utc(value);
            }
            _ => {}
        }
    }
    events
}

/// Splits `NAME;PARAM=…:VALUE` into its three parts.
fn split_property(line: &str) -> Option<(&str, Vec<&str>, &str)> {
    let colon = line.find(':')?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim();
    if name.is_empty() {
        return None;
    }
    Some((name, parts.collect(), value))
}

/// `YYYYMMDDTHHMMSSZ` only — the UTC form. Anything else is None.
fn parse_ical_utc(value: &str) -> Option<DateTime<Utc>> {
    let v = value.trim();
    if !v.ends_with('Z') || v.len() != 16 {
        return None;
    }
    Some(
        chrono::NaiveDateTime::parse_from_str(&v[..15], "%Y%m%dT%H%M%S")
            .ok()?
            .and_utc(),
    )
}

/// Unescapes the RFC 5545 TEXT escapes we emit on the outbound side.
fn unescape_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(next) => out.push(next),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

// --- iCal rendering (outbound feed) -----------------------------------------------

fn ical_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

fn ical_utc(t: DateTime<Utc>) -> String {
    t.format("%Y%m%dT%H%M%SZ").to_string()
}

/// The user's live appointments (cancelled excluded — done stays, history
/// is real) as a subscribable VCALENDAR document.
pub fn render_feed(user_name: &str, appointments: &[core_domain::entities::Appointment]) -> String {
    let now = ical_utc(Utc::now());
    let mut out = String::new();
    out.push_str("BEGIN:VCALENDAR\r\n");
    out.push_str("VERSION:2.0\r\n");
    out.push_str("PRODID:-//Marula PSA//Calendar Sync 1.0//EN\r\n");
    out.push_str("CALSCALE:GREGORIAN\r\n");
    out.push_str("METHOD:PUBLISH\r\n");
    out.push_str(&format!("X-WR-CALNAME:Marula — {user_name}\r\n"));
    for a in appointments {
        if a.status == AppointmentStatus::Cancelled {
            continue;
        }
        out.push_str("BEGIN:VEVENT\r\n");
        out.push_str(&format!("UID:{}@marula\r\n", a.id));
        out.push_str(&format!("DTSTAMP:{now}\r\n"));
        out.push_str(&format!("DTSTART:{}\r\n", ical_utc(a.starts_at)));
        out.push_str(&format!("DTEND:{}\r\n", ical_utc(a.ends_at)));
        let summary = match a.status {
            AppointmentStatus::InProgress => format!("[in progress] {}", a.title),
            _ => a.title.clone(),
        };
        out.push_str(&format!("SUMMARY:{}\r\n", ical_escape(&summary)));
        out.push_str("TRANSP:OPAQUE\r\n");
        if let Some(notes) = a.notes.as_deref().filter(|n| !n.is_empty()) {
            out.push_str(&format!(
                "DESCRIPTION:{}\r\n",
                ical_escape(&notes.replace('\r', ""))
            ));
        }
        out.push_str("END:VEVENT\r\n");
    }
    out.push_str("END:VCALENDAR\r\n");
    out
}

// --- tokens ----------------------------------------------------------------------

/// 32 bytes of OS randomness, URL-safe — unguessable by construction.
fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Only the SHA-256 hash of a feed token is ever stored.
fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// --- rate limiting (public feed) ---------------------------------------------------

const RATE_WINDOW_SECS: u64 = 60;
const FEED_RATE_LIMIT_PER_MIN: u32 = 120;

struct RateWindow(std::time::Instant, u32);

fn check_rate_limit(key: &str) -> ApiResult<()> {
    static WINDOWS: OnceLock<Mutex<std::collections::HashMap<String, RateWindow>>> =
        OnceLock::new();
    let buckets = WINDOWS.get_or_init(Default::default);
    let mut map = buckets
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let now = Instant::now();
    let window = map.entry(key.to_string()).or_insert(RateWindow(now, 0));
    if now.duration_since(window.0).as_secs() >= RATE_WINDOW_SECS {
        *window = RateWindow(now, 0);
    }
    window.1 += 1;
    if window.1 > FEED_RATE_LIMIT_PER_MIN {
        let elapsed = now.duration_since(window.0).as_secs();
        let retry_after = RATE_WINDOW_SECS.saturating_sub(elapsed).max(1);
        return Err(ApiError::RateLimited {
            retry_after_secs: i64::try_from(retry_after).unwrap_or(i64::MAX),
        });
    }
    Ok(())
}

// --- wire shapes --------------------------------------------------------------------

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateConnectionRequest {
    /// Whose calendar. Defaults to the caller; another user requires
    /// owner/admin.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
    pub name: String,
    /// Provider discriminator; 'ical' is the only one today.
    #[serde(default = "default_provider")]
    pub provider: String,
    /// `outbound` | `inbound` | `two_way`.
    pub direction: String,
    /// Required for inbound/two_way: the external iCal URL to poll.
    pub feed_url: Option<String>,
}

fn default_provider() -> String {
    "ical".into()
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateConnectionRequest {
    pub name: Option<String>,
    /// `outbound` | `inbound` | `two_way`.
    pub direction: Option<String>,
    pub feed_url: Option<String>,
    /// `active` | `disabled` | `error`.
    pub status: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectionView {
    #[schema(value_type = uuid::Uuid)]
    pub connection: CalendarSyncConnection,
    /// The raw outbound feed token — returned ONLY here (creation) and
    /// by the rotate endpoint. Never readable again afterwards.
    pub feed_token: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct SyncForceQuery {
    /// Force a poll even when the connection is disabled (admin repair).
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SyncOutcomeResponse {
    #[schema(value_type = uuid::Uuid)]
    pub connection_id: Uuid,
    /// The apply counters, wire-shaped (SyncOutcome is an internal struct).
    pub outcome: SyncOutcomeWire,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SyncOutcomeWire {
    pub imported: i64,
    pub updated: i64,
    pub released: i64,
    pub overlaps: i64,
    pub duplicates: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EventsListResponse {
    #[schema(value_type = uuid::Uuid)]
    pub connection_id: Uuid,
    pub events: Vec<core_domain::entities::CalendarSyncEvent>,
}

fn parse_direction(value: &str) -> ApiResult<CalendarSyncDirection> {
    CalendarSyncDirection::parse(value).ok_or_else(|| {
        ApiError::Validation("direction must be outbound | inbound | two_way".into())
    })
}

fn parse_status(value: &str) -> ApiResult<CalendarSyncStatus> {
    CalendarSyncStatus::parse(value)
        .ok_or_else(|| ApiError::Validation("status must be active | error | disabled".into()))
}

// --- connection endpoints -------------------------------------------------------------

/// GET /api/calendar-sync — the caller's (or target user's) connections.
#[utoipa::path(operation_id = "calendar_sync_list", get, path = "/api/calendar-sync", tag = "scheduling", security(("bearer" = [])),
    params(crate::routes::availability::AvailabilityQuery),
    responses((status = 200, body = Vec<CalendarSyncConnection>)))]
pub async fn list_connections(
    State(state): State<AppState>,
    extensions: Extensions,
    axum::extract::Query(q): axum::extract::Query<crate::routes::availability::AvailabilityQuery>,
) -> ApiResult<Json<Vec<CalendarSyncConnection>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::View))?;
    let user_id = target_user(&authed.principal, q.user_id, "calendar-sync reads")?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let connections = core_db::calendar_sync::list_connections(&mut tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(
        connections
            .into_iter()
            .filter(|c| c.user_id == user_id)
            .collect(),
    ))
}

/// POST /api/calendar-sync — create a connection. Outbound-capable
/// directions get a feed token generated immediately (returned once).
#[utoipa::path(operation_id = "calendar_sync_create", post, path = "/api/calendar-sync", tag = "scheduling", security(("bearer" = [])),
    request_body = CreateConnectionRequest,
    responses((status = 201, body = ConnectionView)))]
pub async fn create_connection(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateConnectionRequest>,
) -> ApiResult<(StatusCode, Json<ConnectionView>)> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::Edit))?;
    let user_id = target_user(&authed.principal, req.user_id, "calendar-sync writes")?;
    let direction = parse_direction(&req.direction)?;
    let connection = CalendarSyncConnection {
        id: Uuid::now_v7(),
        org_id: authed.principal.user.org_id,
        user_id,
        name: req.name.trim().to_string(),
        provider: req.provider.clone(),
        direction,
        status: CalendarSyncStatus::Active,
        feed_url: req.feed_url.clone(),
        feed_path: None,
        last_synced_at: None,
        last_error: None,
        // A delegated provider's token arrives through the OAuth callback,
        // not through create. Both are NULL here and the connection stays in
        // a not-connected state until consent completes.
        access_token: None,
        token_expires_at: None,
        external_account: None,
        scopes: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    connection.validate().map_err(ApiError::from)?;
    // Accept the three kinds the DB CHECK now admits, with the per-provider
    // requirement each one actually has. A delegated provider is created
    // WITHOUT a token and completes the OAuth flow afterwards, so requiring
    // one here would make the consent flow impossible to start.
    match req.provider.as_str() {
        "ical" => {
            // Only when we are READING from the external calendar. An
            // OUTBOUND connection publishes this org's own feed and
            // legitimately has no external URL to point at — requiring one
            // there turned a valid outbound create into a 422 and, worse,
            // shadowed the duplicate-name 409 that the test for it asserts.
            if direction != CalendarSyncDirection::Outbound
                && req.feed_url.as_deref().is_none_or(|u| u.trim().is_empty())
            {
                return Err(ApiError::Validation(
                    "feed_url is required for an inbound 'ical' connection".into(),
                ));
            }
        }
        "google" | "microsoft" => {
            if !matches!(direction, CalendarSyncDirection::Inbound) {
                // Mirroring a vendor calendar OUTBOUND would publish this
                // org's schedule to Google/Microsoft with no consent, and the
                // delegated grant these providers use is read-only by
                // construction. Refusing at create is the only place this can
                // be caught before a customer's bookings are exported.
                return Err(ApiError::Validation(format!(
                    "the '{}' provider supports inbound sync only",
                    req.provider
                )));
            }
        }
        other => {
            return Err(ApiError::Validation(format!(
                "unknown provider '{other}': expected ical, google or microsoft"
            )))
        }
    }
    let org_id = connection.org_id;
    let actor = authed.principal.user.id;

    // The outbound token is minted BEFORE the tx so the response can
    // carry it once; only its hash is persisted.
    let feed_token = matches!(
        direction,
        CalendarSyncDirection::Outbound | CalendarSyncDirection::TwoWay
    )
    .then(generate_token);

    let connection_for_tx = connection.clone();
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        core_db::calendar_sync::create_connection(&mut *tx.tx, &connection_for_tx).await?;
        if let Some(token) = &feed_token {
            core_db::calendar_sync::set_feed_token(
                &mut tx.tx,
                org_id,
                connection_for_tx.id,
                &hash_token(token),
            )
            .await?;
        }
        tx.emit(
            core_domain::events::types::CALENDAR_SYNC_CONNECTION_UPDATED,
            "calendar_sync_connection",
            Some(connection.id.to_string().as_str()),
            &serde_json::to_value(CalendarSyncConnectionUpdated {
                connection_id: connection.id,
                user_id,
                provider: connection_for_tx.provider.clone(),
                direction: connection_for_tx.direction.as_str().into(),
                status: connection_for_tx.status.as_str().into(),
                last_error: None,
                occurred_at: Utc::now(),
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        Ok((tx, feed_token))
    })
    .await
    .map(|feed_token| {
        (
            StatusCode::CREATED,
            Json(ConnectionView {
                connection,
                feed_token,
            }),
        )
    })
}

/// PATCH /api/calendar-sync/{id} — edit name/direction/feed_url/status.
#[utoipa::path(operation_id = "calendar_sync_update", patch, path = "/api/calendar-sync/{id}", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)), request_body = UpdateConnectionRequest,
    responses((status = 200, body = CalendarSyncConnection), (status = 404)))]
pub async fn update_connection(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateConnectionRequest>,
) -> ApiResult<Json<CalendarSyncConnection>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let direction = match &req.direction {
        Some(d) => Some(parse_direction(d)?),
        None => None,
    };
    let status = match &req.status {
        Some(s) => Some(parse_status(s)?),
        None => None,
    };
    if let Some(name) = &req.name {
        if name.trim().is_empty() {
            return Err(ApiError::Validation("name must not be empty".into()));
        }
    }
    let updated = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let existing = core_db::calendar_sync::get_connection(&mut tx.tx, org_id, id).await?;
        if existing.user_id != authed.principal.user.id && !authed.principal.is_owner_or_admin() {
            return Err(ApiError::Forbidden(
                "calendar-sync writes for another user require owner or admin".into(),
            ));
        }
        let name = req.name.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let feed_url = req
            .feed_url
            .as_deref()
            .map(|u| u.trim())
            .filter(|u| !u.is_empty());
        let status = status.or(None);
        core_db::calendar_sync::update_connection(
            &mut tx.tx,
            org_id,
            id,
            name,
            direction,
            feed_url.is_some().then_some(feed_url),
            status,
        )
        .await?;
        // Re-validate the composite (direction ↔ feed_url).
        let updated = core_db::calendar_sync::get_connection(&mut tx.tx, org_id, id).await?;
        updated.validate().map_err(ApiError::from)?;
        tx.emit(
            core_domain::events::types::CALENDAR_SYNC_CONNECTION_UPDATED,
            "calendar_sync_connection",
            Some(id.to_string().as_str()),
            &serde_json::to_value(CalendarSyncConnectionUpdated {
                connection_id: id,
                user_id: updated.user_id,
                provider: updated.provider.clone(),
                direction: updated.direction.as_str().into(),
                status: updated.status.as_str().into(),
                last_error: updated.last_error.clone(),
                occurred_at: Utc::now(),
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        Ok((tx, updated))
    })
    .await?;
    Ok(Json(updated))
}

/// DELETE /api/calendar-sync/{id} — the connection, its mirror rows, and
/// the exclusion rows the sync owns all go away (external busy time
/// frees the calendar).
#[utoipa::path(operation_id = "calendar_sync_delete", delete, path = "/api/calendar-sync/{id}", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 204), (status = 404)))]
pub async fn delete_connection(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let existing = core_db::calendar_sync::get_connection(&mut tx.tx, org_id, id).await?;
        if existing.user_id != authed.principal.user.id && !authed.principal.is_owner_or_admin() {
            return Err(ApiError::Forbidden(
                "calendar-sync writes for another user require owner or admin".into(),
            ));
        }
        core_db::calendar_sync::delete_connection(&mut tx.tx, org_id, id).await?;
        tx.emit(
            core_domain::events::types::CALENDAR_SYNC_CONNECTION_UPDATED,
            "calendar_sync_connection",
            Some(id.to_string().as_str()),
            &serde_json::to_value(CalendarSyncConnectionUpdated {
                connection_id: id,
                user_id: existing.user_id,
                provider: existing.provider.clone(),
                direction: existing.direction.as_str().into(),
                status: "disabled".into(),
                last_error: None,
                occurred_at: Utc::now(),
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        Ok((tx, ()))
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/calendar-sync/{id}/rotate-token — invalidates the old
/// outbound feed URL and returns the new raw token once.
#[utoipa::path(operation_id = "calendar_sync_rotate_token", post, path = "/api/calendar-sync/{id}/rotate-token", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = ConnectionView), (status = 404)))]
pub async fn rotate_token(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ConnectionView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let token = generate_token();
    let token_for_tx = token.clone();
    let connection = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let existing = core_db::calendar_sync::get_connection(&mut tx.tx, org_id, id).await?;
        if existing.user_id != authed.principal.user.id && !authed.principal.is_owner_or_admin() {
            return Err(ApiError::Forbidden(
                "calendar-sync writes for another user require owner or admin".into(),
            ));
        }
        if !matches!(
            existing.direction,
            CalendarSyncDirection::Outbound | CalendarSyncDirection::TwoWay
        ) {
            return Err(ApiError::Validation(
                "token rotation applies to outbound-capable connections".into(),
            ));
        }
        core_db::calendar_sync::set_feed_token(&mut tx.tx, org_id, id, &hash_token(&token_for_tx))
            .await?;
        let updated = core_db::calendar_sync::get_connection(&mut tx.tx, org_id, id).await?;
        tx.emit(
            core_domain::events::types::CALENDAR_SYNC_CONNECTION_UPDATED,
            "calendar_sync_connection",
            Some(id.to_string().as_str()),
            &serde_json::to_value(CalendarSyncConnectionUpdated {
                connection_id: id,
                user_id: existing.user_id,
                provider: existing.provider.clone(),
                direction: existing.direction.as_str().into(),
                status: existing.status.as_str().into(),
                last_error: None,
                occurred_at: Utc::now(),
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        Ok((tx, updated))
    })
    .await?;
    Ok(Json(ConnectionView {
        connection,
        feed_token: Some(token),
    }))
}

/// Fixed client copy per provider failure — internals stay server-side.
fn fixed_poll_copy(e: &ProviderError) -> String {
    e.to_string()
}

/// GET /api/calendar-sync/{id}/events — the inbound mirror (the
/// conflict/duplicate surface Wave 14 lane 7 renders).
#[utoipa::path(operation_id = "calendar_sync_events", get, path = "/api/calendar-sync/{id}/events", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = EventsListResponse), (status = 404)))]
pub async fn list_events(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<EventsListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let connection = core_db::calendar_sync::get_connection(&mut tx, org_id, id).await?;
    if connection.user_id != authed.principal.user.id && !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "calendar-sync reads for another user require owner or admin".into(),
        ));
    }
    let events = core_db::calendar_sync::list_events(&mut tx, org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(EventsListResponse {
        connection_id: id,
        events,
    }))
}

/// POST /api/calendar-sync/{id}/sync — poll now (manual trigger; the
/// worker lane calls the same seam on its schedule). Network fetch
/// happens OUTSIDE the transaction; the apply + poll-state record + event
/// emission run inside ONE org transaction.
#[utoipa::path(operation_id = "calendar_sync_run", post, path = "/api/calendar-sync/{id}/sync", tag = "scheduling", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path), crate::routes::calendar_sync::SyncForceQuery),
    responses((status = 200, body = SyncOutcomeResponse), (status = 404), (status = 503, description = "external calendar unreachable/unreadable")))]
pub async fn run_sync(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<SyncForceQuery>,
) -> ApiResult<Json<SyncOutcomeResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(core_domain::policy::Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;

    let connection = {
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org_and_user(&mut tx, org_id, actor)
            .await
            .map_err(ApiError::from)?;
        let c = core_db::calendar_sync::get_connection(&mut tx, org_id, id).await?;
        tx.commit().await.map_err(ApiError::from)?;
        c
    };
    if connection.user_id != actor && !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "calendar-sync writes for another user require owner or admin".into(),
        ));
    }
    if connection.status == CalendarSyncStatus::Disabled && !q.force {
        return Err(ApiError::Conflict(
            "this calendar-sync connection is disabled; sync with ?force=1 to repair".into(),
        ));
    }
    if !matches!(
        connection.direction,
        CalendarSyncDirection::Inbound | CalendarSyncDirection::TwoWay
    ) {
        return Err(ApiError::Validation(
            "only inbound and two-way connections poll an external feed".into(),
        ));
    }
    let provider = provider_for(&connection)?;

    // Network first, transaction second: never hold the org transaction
    // across the wire.
    let pull = provider.fetch_events(&connection).await;
    let (events, sync_token) = match pull {
        Ok(pull) => (pull.events, pull.sync_token),
        Err(e) => {
            let copy = e.to_string();
            org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
                core_db::calendar_sync::record_sync_result(
                    &mut tx.tx,
                    org_id,
                    id,
                    None,
                    false,
                    Some(copy.as_str()),
                    Utc::now(),
                )
                .await?;
                tx.emit(
                    core_domain::events::types::CALENDAR_SYNC_CONNECTION_UPDATED,
                    "calendar_sync_connection",
                    Some(id.to_string().as_str()),
                    &serde_json::to_value(CalendarSyncConnectionUpdated {
                        connection_id: id,
                        user_id: connection.user_id,
                        provider: connection.provider.clone(),
                        direction: connection.direction.as_str().into(),
                        status: "error".into(),
                        last_error: Some(copy),
                        occurred_at: Utc::now(),
                    })
                    .unwrap_or(serde_json::Value::Null),
                )
                .await?;
                Ok((tx, ()))
            })
            .await?;
            return Err(ApiError::Unavailable(fixed_poll_copy(&e)));
        }
    };

    let connection_id = id;
    let user_id = connection.user_id;
    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        let outcome =
            core_db::calendar_sync::apply_sync(&mut tx.tx, org_id, connection_id, user_id, &events)
                .await?;
        core_db::calendar_sync::record_sync_result(
            &mut tx.tx,
            org_id,
            connection_id,
            sync_token.as_deref(),
            true,
            None,
            Utc::now(),
        )
        .await?;
        tx.emit(
            core_domain::events::types::CALENDAR_SYNC_COMPLETED,
            "calendar_sync_connection",
            Some(connection_id.to_string().as_str()),
            &serde_json::to_value(CalendarSyncCompleted {
                connection_id,
                user_id,
                imported: i64::from(outcome.imported),
                updated: i64::from(outcome.updated),
                released: i64::from(outcome.released),
                overlaps: i64::from(outcome.overlaps),
                duplicates: i64::from(outcome.duplicates),
                synced_at: Utc::now(),
            })
            .unwrap_or(serde_json::Value::Null),
        )
        .await?;
        if outcome.imported > 0 || outcome.released > 0 {
            tx.emit(
                core_domain::events::types::AVAILABILITY_UPDATED,
                "availability",
                Some(user_id.to_string().as_str()),
                &serde_json::to_value(core_domain::events::AvailabilityChanged { user_id })
                    .unwrap_or(serde_json::Value::Null),
            )
            .await?;
        }
        Ok((tx, outcome))
    })
    .await?;

    Ok(Json(SyncOutcomeResponse {
        connection_id: id,
        outcome: SyncOutcomeWire {
            imported: i64::from(outcome.imported),
            updated: i64::from(outcome.updated),
            released: i64::from(outcome.released),
            overlaps: i64::from(outcome.overlaps),
            duplicates: i64::from(outcome.duplicates),
        },
    }))
}

// --- public outbound feed --------------------------------------------------------------

// --- public outbound feed --------------------------------------------------------------

/// Router for the token-scoped public feed (mounted next to the
/// scheduling-links public surface).
pub fn public_routes(gen: crate::routes::ApiGeneration) -> axum::Router<AppState> {
    use axum::routing::get;
    let base = gen.base();
    axum::Router::new().route(
        &format!("{base}/calendar-sync/feed/{{token}}"),
        get(public_feed),
    )
}

/// GET /api/calendar-sync/feed/{token} — the OUTBOUND direction: the
/// user's live appointments as a subscribable iCalendar document.
/// Unauthenticated but token-scoped and rate-limited (the scheduling-links
/// pattern); the token resolves org + user, then the read is org-scoped.
#[utoipa::path(operation_id = "public_calendar_sync_feed", get, path = "/api/calendar-sync/feed/{token}", tag = "scheduling",
    params(("token" = String, Path)),
    responses((status = 200, body = String, content_type = "text/calendar"), (status = 404, description = "unknown token"), (status = 429)))]
pub async fn public_feed(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> ApiResult<axum::response::Response> {
    check_rate_limit(&format!("feed:{token}"))?;
    let token_hash = hash_token(&token);
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    let (org_id, connection_id) = core_db::calendar_sync::resolve_feed_target(&mut tx, &token_hash)
        .await
        .map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let connection = core_db::calendar_sync::get_connection(&mut tx, org_id, connection_id)
        .await
        .map_err(ApiError::from)?;
    let user_id = connection.user_id;
    let from = Utc::now() - Duration::days(90);
    let to = Utc::now() + Duration::days(180);
    let appointments =
        core_db::scheduling::list_appointments_between(&mut *tx, org_id, &[user_id], from, to)
            .await?;
    let body = render_feed(&connection.name, &appointments);
    Ok((
        StatusCode::OK,
        [(CONTENT_TYPE, "text/calendar; charset=utf-8")],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_utc_vevents_strictly() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a@x\r\nDTSTART:20260914T110000Z\r\nDTEND:20260914T130000Z\r\nSUMMARY:Standup\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let events = parse_ical(ics);
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.external_uid, "a@x");
        assert_eq!(e.starts_at.to_rfc3339(), "2026-09-14T11:00:00+00:00");
        assert_eq!(e.ends_at.to_rfc3339(), "2026-09-14T13:00:00+00:00");
        assert!(e.is_busy());
    }

    #[test]
    fn unfolds_folded_lines_and_unescapes_text() {
        let ics = "BEGIN:VEVENT\r\nUID:a@x\r\nDTSTART:20260914T110000Z\r\nDTEND:20260914T120000Z\r\nSUMMARY:Long\r\n  folded; with\\, commas\r\nEND:VEVENT\r\n";
        let events = parse_ical(ics);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].title.as_deref(),
            Some("Long folded; with, commas")
        );
    }

    #[test]
    fn skips_non_utc_and_incomplete_events() {
        let ics = "BEGIN:VEVENT\r\nUID:tz@x\r\nDTSTART;TZID=Europe/Berlin:20260914T110000\r\nDTEND;TZID=Europe/Berlin:20260914T120000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:noend@x\r\nDTSTART:20260914T110000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:dateonly@x\r\nDTSTART;VALUE=DATE:20260914\r\nDTEND;VALUE=DATE:20260915\r\nEND:VEVENT\r\n";
        assert!(
            parse_ical(ics).is_empty(),
            "cannot place these deterministically — skip, never guess"
        );
    }

    #[test]
    fn cancelled_and_transparent_are_not_busy() {
        let mut e = ExternalCalendarEvent {
            external_uid: "a@x".into(),
            sequence: 0,
            title: None,
            starts_at: Utc::now(),
            ends_at: Utc::now() + Duration::hours(1),
            cancelled: false,
            transparent: false,
        };
        assert!(e.is_busy());
        e.cancelled = true;
        assert!(!e.is_busy());
        e.cancelled = false;
        e.transparent = true;
        assert!(!e.is_busy());
    }

    #[test]
    fn feed_render_escapes_and_excludes_cancelled() {
        use core_domain::entities::{Appointment, AppointmentKind, AppointmentStatus};
        let mk = |title: &str, status: AppointmentStatus| Appointment {
            id: Uuid::now_v7(),
            org_id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            ticket_id: None,
            client_id: None,
            site_id: None,
            title: title.into(),
            starts_at: Utc::now(),
            ends_at: Utc::now() + Duration::hours(1),
            kind: AppointmentKind::OnSite,
            status,
            travel_minutes: 0,
            notes: Some("has; comma, and\nnewline".into()),
            scheduling_link_id: None,
            travel_from: None,
            travel_distance_meters: None,
            travel_provider: None,
            travel_quality: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let appts = vec![
            mk("Live; one", AppointmentStatus::Scheduled),
            mk("Cancelled", AppointmentStatus::Cancelled),
        ];
        let feed = render_feed("Dana", &appts);
        assert!(feed.contains("SUMMARY:Live\\; one"));
        assert!(feed.contains("DESCRIPTION:has\\; comma\\, and\\nnewline"));
        assert!(!feed.contains("Cancelled"));
        assert!(feed.ends_with("END:VCALENDAR\r\n"));
    }
}

// --- delegated OAuth: connect a Google or Microsoft calendar (claim 5) ------
//
// The two provider kinds above are only reachable if an operator can
// actually complete a consent flow, so this is the rest of the row: a start
// endpoint that redirects to the vendor, and a callback that exchanges the
// code, seals the refresh token, and caches the access token.
//
// The refresh token is SEALED, never stored on the connection row. It is a
// long-lived bearer that can read the user's entire calendar, and the
// connection row is readable by anything that can list connections. The
// access token IS cached there, because it is short-lived and regenerable
// and refreshing on every poll would make the poll endpoint a proxy for the
// vendor's token service.

/// Sealed-slot names for the two delegated providers. Namespaced under
/// `calendar.` so they cannot collide with the `mail.*` transport slots in
/// the same table.
const SEALED_GOOGLE_PROVIDER: &str = "calendar.google";
const SEALED_MICROSOFT_PROVIDER: &str = "calendar.microsoft";

/// How long an unused authorization state stays valid.
///
/// Bounded because an unbounded state row is a standing invitation to finish
/// someone else's consent flow: the CSRF nonce is what ties the callback
/// back to the request that started it, and a nonce that never expires never
/// stops being a target.
const OAUTH_STATE_TTL_MINUTES: i64 = 10;

fn sealed_slot(provider: &str) -> Option<&'static str> {
    match provider {
        "google" => Some(SEALED_GOOGLE_PROVIDER),
        "microsoft" => Some(SEALED_MICROSOFT_PROVIDER),
        _ => None,
    }
}

/// POST /api/calendar-sync/{connection_id}/authorize — begin consent.
///
/// Returns the vendor URL to send the operator to. The `state` is a
/// server-generated nonce, NOT a client-supplied value: a client that chose
/// its own state would defeat the CSRF protection entirely.
#[utoipa::path(operation_id = "calendar_sync_authorize_start", post,
    path = "/api/calendar-sync/{connection_id}/authorize", tag = "calendar-sync",
    security(("bearer" = [])), params(("connection_id" = Uuid, Path)),
    responses((status = 200, body = OAuthStartResponse)))]
pub async fn authorize_start(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(connection_id): Path<Uuid>,
) -> ApiResult<Json<OAuthStartResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let me = authed.principal.user.id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, me)
        .await
        .map_err(ApiError::from)?;
    let connection = core_db::calendar_sync::get_connection(&mut tx, org_id, connection_id)
        .await
        .map_err(ApiError::from)?;

    let slot = sealed_slot(&connection.provider).ok_or_else(|| {
        ApiError::Validation(format!(
            "the '{}' provider is not a delegated OAuth provider",
            connection.provider
        ))
    })?;
    let (client_id, auth_url) = oauth_client_config(&connection.provider)?;

    // A fresh nonce per attempt, so retrying consent invalidates the old
    // callback URL rather than leaving two live ones.
    let state_nonce = uuid::Uuid::new_v4().to_string();
    let expires = Utc::now() + chrono::Duration::minutes(OAUTH_STATE_TTL_MINUTES);
    sqlx::query(
        "UPDATE calendar_sync_connections
            SET oauth_state = $2, oauth_state_expires_at = $3, updated_at = now()
          WHERE org_id = $1 AND id = $4",
    )
    .bind(org_id)
    .bind(&state_nonce)
    .bind(expires)
    .bind(connection_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;

    let redirect_uri = oauth_redirect_uri(&connection.provider);
    let url = if connection.provider == "google" {
        format!(
            "https://accounts.google.com/o/oauth2/v2/auth\
             ?client_id={client_id}&redirect_uri={redirect_uri}\
             &response_type=code&scope={}\
             &access_type=offline&prompt=consent&include_granted_scopes=true&state={state_nonce}",
            urlencode(GOOGLE_SCOPE),
        )
    } else {
        format!(
            "{auth_url}?client_id={client_id}&redirect_uri={redirect_uri}\
             &response_type=code&response_mode=query\
             &scope={}&offline_access=offline_access&state={state_nonce}",
            urlencode(GRAPH_SCOPE),
        )
    };
    let _ = slot;

    Ok(Json(OAuthStartResponse {
        authorize_url: url,
        expires_in_minutes: OAUTH_STATE_TTL_MINUTES,
    }))
}

const GOOGLE_SCOPE: &str = "https://www.googleapis.com/auth/calendar.readonly";
/// `Calendars.Read` + `offline_access`. The latter is what yields a refresh
/// token at all; without it the connection works until the access token
/// expires an hour later and then silently stops.
const GRAPH_SCOPE: &str = "https://graph.microsoft.com/Calendars.Read offline_access";

/// The consent screen must show a Marula app, and these are per-tenant
/// Google/Microsoft client credentials an operator registers. Without them
/// the start endpoint 503s with a legible message rather than redirecting
/// the operator to a consent screen for a client that does not exist.
fn oauth_client_config(provider: &str) -> ApiResult<(String, String)> {
    let (cid_var, secret_var, auth_url) = match provider {
        "google" => (
            "GOOGLE_CALENDAR_CLIENT_ID",
            "GOOGLE_CALENDAR_CLIENT_SECRET",
            "https://accounts.google.com/o/oauth2/v2/auth",
        ),
        _ => (
            "MICROSOFT_GRAPH_CLIENT_ID",
            "MICROSOFT_GRAPH_CLIENT_SECRET",
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
        ),
    };
    let client_id = std::env::var(cid_var)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            ApiError::Validation(format!(
                "{cid_var} is not configured — a delegated calendar provider needs an operator-registered OAuth client"
            ))
        })?;
    if std::env::var(secret_var)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_none()
    {
        return Err(ApiError::Validation(format!(
            "{secret_var} is not configured — the code exchange needs the client secret"
        )));
    }
    Ok((client_id, auth_url.to_string()))
}

fn oauth_redirect_uri(provider: &str) -> String {
    let base = std::env::var("PUBLIC_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:3000".to_string())
        .trim_end_matches('/')
        .to_string();
    let suffix = if provider == "google" {
        "/api/calendar-sync/google/callback"
    } else {
        "/api/calendar-sync/microsoft/callback"
    };
    format!("{base}{suffix}")
}

/// Minimal percent-encoding for the two places an OAuth URL needs it. A
/// full URL-encoding crate is not a dependency, and `urlencode` only has to
/// handle the characters that actually appear in a scope list.
fn urlencode(raw: &str) -> String {
    raw.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' | b'/' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// Where the operator lands after consent.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OAuthStartResponse {
    /// The vendor consent URL. The operator is redirected here; the callback
    /// URL is pre-registered by the operator with their OAuth client.
    pub authorize_url: String,
    pub expires_in_minutes: i64,
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct OAuthCallbackQuery {
    pub state: String,
    pub code: Option<String>,
    /// The operator may decline; the vendor then redirects with an error
    /// and no code. That is a user choice, not a failure, so it gets its own
    /// message instead of "invalid code".
    pub error: Option<String>,
}

/// GET /api/calendar-sync/{provider}/callback — the vendor's redirect target.
///
/// The `state` is matched against the stored nonce, which is what makes the
/// flow CSRF-safe: a callback whose state does not match the connection this
/// browser started is discarded, so an attacker cannot feed a victim's
/// browser a code obtained under someone else's consent.
#[utoipa::path(operation_id = "calendar_sync_oauth_callback", get,
    path = "/api/calendar-sync/{provider}/callback", tag = "calendar-sync",
    params(("provider" = String, Path), OAuthCallbackQuery),
    responses((status = 200, body = OAuthCallbackResult)))]
pub async fn oauth_callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(q): Query<OAuthCallbackQuery>,
) -> ApiResult<Json<OAuthCallbackResult>> {
    if sealed_slot(&provider).is_none() {
        return Err(ApiError::Validation(format!(
            "'{provider}' is not a delegated OAuth provider"
        )));
    }
    if q.error.is_some() {
        // Declined consent. Not an error state for the product — the
        // connection simply stays unconnected and Settings says so.
        return Ok(Json(OAuthCallbackResult {
            connected: false,
            message: Some("authorization was declined or cancelled".into()),
        }));
    }
    let code = q
        .code
        .as_deref()
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| {
            ApiError::Validation("the callback carried neither a code nor an error".into())
        })?;

    // Find the connection this state belongs to. The lookup is by nonce AND
    // unexpired, so a stale callback is refused rather than completing a
    // consent the operator started ten minutes and a decision ago.
    //
    // This runs on the vendor's REDIRECT, so there is no bearer token and no
    // `AuthedUser`. The org and user come from the state row itself: the
    // nonce is the capability that ties this callback to the request that
    // started it, and `user_id` rides along so the sealed write is attributed
    // to the technician whose calendar it is rather than to nobody.
    //
    // The UPDATE is also what CONSUMES the nonce, atomically — a replayed
    // callback finds nothing to consume, and doing it in the consuming
    // statement rather than a separate delete is what closes the window
    // where two callbacks race.
    let mut conn = state.pool.acquire().await.map_err(ApiError::from)?;
    let row: Option<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        "UPDATE calendar_sync_connections
            SET oauth_state = NULL, oauth_state_expires_at = NULL, updated_at = now()
          WHERE provider = $1 AND oauth_state = $2
            AND oauth_state_expires_at > now()
          RETURNING id, org_id, user_id",
    )
    .bind(&provider)
    .bind(&q.state)
    .fetch_optional(&mut *conn)
    .await
    .map_err(ApiError::from)?;

    let Some((connection_id, org_id, user_id)) = row else {
        return Err(ApiError::Validation(
            "this authorization link is unknown or has expired — start the connection again".into(),
        ));
    };
    drop(conn);

    // Exchange the code. The nonce is consumed BEFORE the exchange, so a
    // replayed callback finds nothing to consume.
    let token = exchange_code(&provider, code, &oauth_redirect_uri(&provider)).await?;

    let sealed = crate::crypto::seal(
        &state.config.secret_key,
        serde_json::to_vec(&token)
            .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?
            .as_slice(),
    )
    .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))?;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    core_db::byok::upsert_secret(
        &mut tx,
        Uuid::now_v7(),
        org_id,
        sealed_slot(&provider).expect("checked above"),
        &sealed,
        Some(user_id),
    )
    .await
    .map_err(ApiError::from)?;
    // Cache the short-lived half on the connection so a poll does not need a
    // vendor round trip for a token.
    sqlx::query(
        "UPDATE calendar_sync_connections
            SET access_token = $3, token_expires_at = $4, scopes = $5, updated_at = now()
          WHERE org_id = $1 AND id = $2",
    )
    .bind(org_id)
    .bind(connection_id)
    .bind(&token.access_token)
    .bind(token.expires_at)
    .bind(&token.scope)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(OAuthCallbackResult {
        connected: true,
        message: None,
    }))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OAuthCallbackResult {
    pub connected: bool,
    /// Set only on a DECLINED consent, so the UI can say "you cancelled"
    /// rather than showing a generic failure.
    pub message: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct OAuthToken {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<DateTime<Utc>>,
    scope: Option<String>,
}

/// Swap the authorization code for tokens, per vendor.
async fn exchange_code(provider: &str, code: &str, redirect_uri: &str) -> ApiResult<OAuthToken> {
    let (client_id_var, secret_var, token_url) = match provider {
        "google" => (
            "GOOGLE_CALENDAR_CLIENT_ID",
            "GOOGLE_CALENDAR_CLIENT_SECRET",
            "https://oauth2.googleapis.com/token",
        ),
        _ => (
            "MICROSOFT_GRAPH_CLIENT_ID",
            "MICROSOFT_GRAPH_CLIENT_SECRET",
            "https://login.microsoftonline.com/common/oauth2/v2.0/token",
        ),
    };
    let client_id = std::env::var(client_id_var).unwrap_or_default();
    let client_secret = std::env::var(secret_var).unwrap_or_default();

    let resp = reqwest::Client::new()
        .post(token_url)
        .form(&[
            ("code", code),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("token exchange: {e}")))?;
    let status = resp.status();
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("token response: {e}")))?;
    if !status.is_success() {
        // The vendor's error text can echo the submitted code, so it is NOT
        // forwarded. The operator's remedy is the same either way: re-run
        // the consent flow.
        tracing::warn!(
            provider,
            status = status.as_u16(),
            code = body.get("error").and_then(|v| v.as_str()).unwrap_or("?"),
            "oauth code exchange rejected"
        );
        return Err(ApiError::Validation(
            "the provider rejected the authorization code — start the connection again".into(),
        ));
    }
    let access_token = body["access_token"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            ApiError::Internal(anyhow::anyhow!("token response carried no access_token"))
        })?
        .to_string();
    let expires_in = body["expires_in"].as_i64();
    Ok(OAuthToken {
        access_token,
        refresh_token: body["refresh_token"].as_str().map(str::to_string),
        expires_at: expires_in.map(|s| Utc::now() + chrono::Duration::seconds(s - 30)),
        scope: body["scope"].as_str().map(str::to_string),
    })
}
