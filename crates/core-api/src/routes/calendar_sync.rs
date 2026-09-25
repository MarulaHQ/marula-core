//! Two-way calendar sync (Wave 14 lane 2): the `CalendarSyncProvider`
//! adapter seam, the first-party iCal provider, and the connection /
//! poll / feed surfaces.
//!
//! Purpose: BIDIRECTIONAL sync between a user's Meridian calendar and an
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
    extract::{Path, State},
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
        other => {
            tracing::error!(provider = other, "unknown calendar-sync provider");
            Err(ApiError::Internal(anyhow::anyhow!(
                "unknown calendar-sync provider"
            )))
        }
    }
}

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
    out.push_str("PRODID:-//Meridian PSA//Calendar Sync 1.0//EN\r\n");
    out.push_str("CALSCALE:GREGORIAN\r\n");
    out.push_str("METHOD:PUBLISH\r\n");
    out.push_str(&format!("X-WR-CALNAME:Meridian — {user_name}\r\n"));
    for a in appointments {
        if a.status == AppointmentStatus::Cancelled {
            continue;
        }
        out.push_str("BEGIN:VEVENT\r\n");
        out.push_str(&format!("UID:{}@meridian\r\n", a.id));
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
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    connection.validate().map_err(ApiError::from)?;
    if req.provider != "ical" {
        return Err(ApiError::Validation(
            "unknown provider; 'ical' is the only provider today".into(),
        ));
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
