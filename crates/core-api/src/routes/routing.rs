//! Dispatch v3 routing endpoints (Wave 32 lane 7).
//!
//! Purpose: the dispatcher-facing read/write surfaces the routing seam
//! powers — the day's journey plans with typed impossible-travel
//! warnings, suggestion-grade route improvements (silent in NONE mode),
//! dispatcher-editable journey start points, and migration-free mileage
//! reconciliation over the existing travel drafts + expenses.
//!
//! Non-purpose: provider administration (operator config), appointment
//! writes (routes/scheduling.rs books travel at save time).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::scheduling::{sched, target_user};
use crate::routing as svc;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use core_domain::entities::{Actor, AuditEvent, Site, TicketExpense, TravelOrigin};
use core_domain::policy::{Action, Permission};
use core_domain::routing::{TravelPlace, TravelStop, TravelWarning};
use core_routing::{
    estimate_minutes, GeoPoint, MatrixRequest, MatrixResponse, Place, RoutingProvider, TravelMode,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

// --- day routing view --------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct RoutingDayQuery {
    /// The day to assemble; defaults to today (UTC), matching the board.
    #[schema(value_type = Option<String>, format = "date")]
    pub date: Option<NaiveDate>,
}

/// Serialized provider posture (mirrors `core_routing::RoutingStatusView`
/// with a schema-friendly shape).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RoutingProviderStatus {
    pub measured: bool,
    pub provider_id: String,
    pub provider_label: String,
    /// `heuristic` | `measured`.
    pub quality: String,
    pub capabilities: Vec<String>,
    pub modes: Vec<String>,
    pub suggestions_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

fn status_view(state: &AppState) -> RoutingProviderStatus {
    let view = state.routing.status_view();
    RoutingProviderStatus {
        measured: view.measured,
        provider_id: view.provider_id,
        provider_label: view.provider_label,
        quality: view.quality.as_str().to_string(),
        capabilities: view.capabilities,
        modes: view.modes,
        suggestions_available: view.suggestions_available,
        reason: view.reason,
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TravelPlaceView {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub site_id: Option<Uuid>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

fn place_view(place: &TravelPlace) -> TravelPlaceView {
    TravelPlaceView {
        kind: place.kind.clone(),
        label: place.label.clone(),
        site_id: place.site_id,
        latitude: place.latitude,
        longitude: place.longitude,
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TravelWarningView {
    pub kind: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub appointment_id: Option<Uuid>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap_minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_reason: Option<String>,
}

fn warning_view(warning: &TravelWarning) -> TravelWarningView {
    TravelWarningView {
        kind: warning.kind.as_str().to_string(),
        appointment_id: warning.appointment_id,
        message: warning.message.clone(),
        required_minutes: warning.required_minutes,
        gap_minutes: warning.gap_minutes,
        short_minutes: warning.short_minutes,
        provider_reason: warning.provider_reason.clone(),
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct JourneyLegView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_label: Option<String>,
    pub from_kind: String,
    #[schema(value_type = uuid::Uuid)]
    pub to_appointment_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance_meters: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct JourneyStopView {
    #[schema(value_type = uuid::Uuid)]
    pub appointment_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
    /// Booked travel minutes into this stop (the appointment field).
    pub travel_minutes: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub travel_from: Option<TravelOrigin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub travel_distance_meters: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub travel_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub travel_quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<TravelPlaceView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leg: Option<JourneyLegView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StartPointView {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// `explicit` | `previous_day_site`.
    pub source: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub site_id: Option<Uuid>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TechnicianJourneyView {
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    pub display_name: String,
    pub timezone: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_point: Option<StartPointView>,
    /// Sum of leg minutes actually estimated for this day.
    pub total_travel_minutes: i64,
    pub stops: Vec<JourneyStopView>,
    pub warnings: Vec<TravelWarningView>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TravelSuggestionView {
    /// `swap_adjacent` | `reassign`.
    pub kind: String,
    #[schema(value_type = uuid::Uuid)]
    pub appointment_id: Uuid,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub swap_with_appointment_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub target_user_id: Option<Uuid>,
    pub minutes_saved: i64,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SuggestionSetView {
    /// False in NONE mode — suggestion-grade features require a measured
    /// provider (ADR 0010 visibly-absent posture).
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub items: Vec<TravelSuggestionView>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RoutingDayView {
    pub date: NaiveDate,
    #[schema(value_type = String, format = "date-time")]
    pub from: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub to: DateTime<Utc>,
    pub status: RoutingProviderStatus,
    pub technicians: Vec<TechnicianJourneyView>,
    pub suggestions: SuggestionSetView,
}

/// One technician-journey intermediate used while assembling the view.
struct JourneyBuild {
    user_id: Uuid,
    display_name: String,
    timezone: String,
    start: Option<svc::StartPoint>,
    stops: Vec<TravelStop>,
    stop_origins: Vec<Option<TravelOrigin>>,
    leg_minutes: Vec<Option<i64>>,
    leg_errors: Vec<Option<String>>,
    matrix: Option<MatrixResponse>,
    has_start: bool,
    day_spans: Vec<(DateTime<Utc>, DateTime<Utc>)>,
}

fn stop_label(row: &core_db::scheduling::DispatchAppointmentRow) -> Option<String> {
    row.ticket_number
        .clone()
        .or_else(|| Some(row.title.clone()))
}

/// GET /api/dispatch/routing — the day's journey plans, typed warnings,
/// and (measured providers only) suggestion-grade route improvements.
#[utoipa::path(operation_id = "dispatch_routing_day", get, path = "/api/dispatch/routing", tag = "scheduling", security(("bearer" = [])),
    params(RoutingDayQuery),
    responses((status = 200, body = RoutingDayView)))]
pub async fn routing_day(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<RoutingDayQuery>,
) -> ApiResult<Json<RoutingDayView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let date = q.date.unwrap_or_else(|| Utc::now().date_naive());
    let from = date
        .and_hms_opt(0, 0, 0)
        .expect("midnight is a valid local time")
        .and_utc();
    let to = from + Duration::days(1);

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let members = core_db::users::list_users(&mut *tx, org_id).await?;
    let techs: Vec<Uuid> = members
        .iter()
        .filter(|u| u.is_active)
        .map(|u| u.id)
        .collect();
    let appointments = core_db::scheduling::list_dispatch_appointments_between(
        &mut *tx,
        org_id,
        &techs,
        from,
        to,
    )
    .await?;
    let mut site_ids: Vec<Uuid> = appointments.iter().filter_map(|a| a.site_id).collect();
    site_ids.sort_unstable();
    site_ids.dedup();
    let sites = core_db::clients::get_sites_by_ids(&mut *tx, org_id, &site_ids).await?;
    let site_map: HashMap<Uuid, Site> = sites.into_iter().map(|s| (s.id, s)).collect();
    let policy = svc::travel_policy_for(&mut tx, org_id).await?;
    let status = status_view(&state);
    let routing = svc::active_routing(&state, &policy)?;
    let provider = routing.provider();

    let mut journeys: Vec<JourneyBuild> = Vec::new();
    for user in members.iter().filter(|u| u.is_active) {
        let start = svc::resolve_start_point(&mut tx, org_id, user.id, date).await?;
        let day_rows: Vec<&core_db::scheduling::DispatchAppointmentRow> = appointments
            .iter()
            .filter(|a| a.user_id == user.id)
            .collect();
        let mut day_spans = Vec::with_capacity(day_rows.len());
        let mut stops = Vec::new();
        let mut stop_origins: Vec<Option<TravelOrigin>> = Vec::new();
        let mut stop_places: Vec<Option<TravelPlace>> = Vec::new();
        for row in &day_rows {
            day_spans.push((row.starts_at, row.ends_at));
            if row.kind != "on_site" {
                continue;
            }
            let place = row
                .site_id
                .and_then(|site_id| site_map.get(&site_id))
                .map(svc::site_place);
            stops.push(TravelStop {
                appointment_id: row.id,
                label: stop_label(row),
                starts_at: row.starts_at,
                ends_at: row.ends_at,
                travel_minutes: i64::from(row.travel_minutes),
                travel_provider: row.travel_provider.clone(),
                travel_quality: row.travel_quality.clone(),
                place: place.clone(),
            });
            stop_origins.push(
                row.travel_from
                    .clone()
                    .and_then(|value| serde_json::from_value(value).ok()),
            );
            stop_places.push(place);
        }
        let has_start = start
            .as_ref()
            .map(|s| s.place.has_coordinates())
            .unwrap_or(false);
        // One batched matrix for the whole journey: node 0 is the start
        // when present, then stops in scheduled order.
        let mut nodes: Vec<Place> = Vec::new();
        if let Some(start_point) = &start {
            nodes.push(svc::place_from_travel_place(&start_point.place));
        }
        for place in &stop_places {
            nodes.push(match place {
                Some(place) => svc::place_from_travel_place(place),
                None => Place::default(),
            });
        }
        let depart_at = stops.first().map(|s| s.starts_at).unwrap_or(from);
        let matrix = if nodes.is_empty() {
            None
        } else {
            svc::matrix_for_nodes(provider, &nodes, TravelMode::Driving, Some(depart_at))
                .await
                .ok()
        };
        let mut leg_minutes: Vec<Option<i64>> = Vec::with_capacity(stops.len());
        let mut leg_errors: Vec<Option<String>> = Vec::with_capacity(stops.len());
        for k in 0..stops.len() {
            let stop_node = if has_start { k + 1 } else { k };
            let previous = if k == 0 {
                if has_start {
                    Some(0)
                } else {
                    None
                }
            } else {
                Some(stop_node - 1)
            };
            let (minutes, error) = match (previous, matrix.as_ref()) {
                (Some(previous), Some(matrix)) => {
                    match matrix.cell(previous, stop_node) {
                        Some(cell) => match cell.duration_seconds {
                            Some(seconds) => (Some(estimate_minutes(seconds)), None),
                            None => (None, Some("leg_unreachable".to_string())),
                        },
                        None => (None, Some("leg_unreachable".to_string())),
                    }
                }
                (Some(_), None) => (None, Some("provider_unavailable".to_string())),
                (None, _) => (None, None),
            };
            leg_minutes.push(minutes);
            leg_errors.push(error);
        }
        journeys.push(JourneyBuild {
            user_id: user.id,
            display_name: user.display_name.clone(),
            timezone: user.timezone.clone(),
            start,
            stops,
            stop_origins,
            leg_minutes,
            leg_errors,
            matrix,
            has_start,
            day_spans,
        });
    }

    // Suggestions are suggestion-grade: measured providers only.
    let mut suggestions: Vec<TravelSuggestionView> = Vec::new();
    if status.measured {
        for journey in &journeys {
            if !journey.has_start {
                continue;
            }
            let count = journey.stops.len();
            let lookup = |a: usize, b: usize| -> Option<i64> {
                let matrix = journey.matrix.as_ref()?;
                let node_of_stop = |k: usize| if journey.has_start { k + 1 } else { k };
                let a_node = if a == count {
                    if journey.has_start {
                        Some(0)
                    } else {
                        None
                    }
                } else {
                    Some(node_of_stop(a))
                };
                let b_node = (b < count).then(|| node_of_stop(b))?;
                matrix
                    .cell(a_node?, b_node)
                    .and_then(|cell| cell.duration_seconds)
                    .map(estimate_minutes)
            };
            for suggestion in
                core_domain::routing::adjacent_swap_suggestions(&journey.stops, lookup, 5)
            {
                suggestions.push(TravelSuggestionView {
                    kind: suggestion.kind.as_str().to_string(),
                    appointment_id: suggestion.appointment_id,
                    swap_with_appointment_id: suggestion.swap_with_appointment_id,
                    target_user_id: suggestion.target_user_id,
                    minutes_saved: suggestion.minutes_saved,
                    label: suggestion.label,
                });
            }
        }
        suggestions = reassign_suggestions(provider, &journeys, suggestions).await;
    }
    let suggestions = SuggestionSetView {
        available: status.measured,
        reason: if status.measured {
            None
        } else {
            status
                .reason
                .clone()
                .or_else(|| Some("no_measured_provider".to_string()))
        },
        items: suggestions,
    };

    let mut technician_views = Vec::with_capacity(journeys.len());
    for journey in &journeys {
        let mut stop_views = Vec::with_capacity(journey.stops.len());
        let mut total = 0_i64;
        for (k, stop) in journey.stops.iter().enumerate() {
            let leg = journey.leg_minutes.get(k).copied().flatten();
            if let Some(minutes) = leg {
                total += minutes;
            }
            let (from_kind, from_label) = if k == 0 {
                journey
                    .start
                    .as_ref()
                    .map(|s| (s.place.kind.clone(), s.place.label.clone()))
                    .unwrap_or_else(|| ("unknown".into(), None))
            } else {
                journey
                    .stops
                    .get(k - 1)
                    .map(|previous| {
                        (
                            "stop".to_string(),
                            previous.label.clone(),
                        )
                    })
                    .unwrap_or_else(|| ("unknown".into(), None))
            };
            let matrix_cell = journey.matrix.as_ref().and_then(|matrix| {
                let stop_node = if journey.has_start { k + 1 } else { k };
                let previous = if k == 0 {
                    if journey.has_start {
                        Some(0)
                    } else {
                        None
                    }
                } else {
                    Some(stop_node - 1)
                };
                previous.and_then(|previous| matrix.cell(previous, stop_node))
            });
            stop_views.push(JourneyStopView {
                appointment_id: stop.appointment_id,
                label: stop.label.clone(),
                starts_at: stop.starts_at,
                ends_at: stop.ends_at,
                travel_minutes: stop.travel_minutes,
                travel_from: journey.stop_origins.get(k).cloned().flatten(),
                travel_distance_meters: matrix_cell
                    .and_then(|cell| cell.distance_meters)
                    .and_then(|m| i32::try_from(m).ok()),
                travel_provider: stop.travel_provider.clone(),
                travel_quality: stop.travel_quality.clone(),
                place: stop.place.as_ref().map(place_view),
                leg: Some(JourneyLegView {
                    from_label: from_label.clone(),
                    from_kind: from_kind.clone(),
                    to_appointment_id: stop.appointment_id,
                    minutes: leg,
                    distance_meters: matrix_cell.and_then(|cell| cell.distance_meters),
                    provider: journey
                        .matrix
                        .as_ref()
                        .map(|matrix| matrix.provider.clone()),
                    quality: journey
                        .matrix
                        .as_ref()
                        .map(|matrix| matrix.quality.as_str().to_string()),
                    error: journey.leg_errors.get(k).cloned().flatten(),
                }),
            });
        }
        let warnings = core_domain::routing::journey_warnings(&core_domain::routing::JourneyInput {
            stops: &journey.stops,
            leg_minutes: &journey.leg_minutes,
            leg_errors: &journey.leg_errors,
            has_start_point: journey.has_start,
        });
        technician_views.push(TechnicianJourneyView {
            user_id: journey.user_id,
            display_name: journey.display_name.clone(),
            timezone: journey.timezone.clone(),
            start_point: journey.start.as_ref().map(|s| StartPointView {
                kind: s.place.kind.clone(),
                label: s.place.label.clone(),
                source: s.source.clone(),
                site_id: s.place.site_id,
                latitude: s.place.latitude,
                longitude: s.place.longitude,
            }),
            total_travel_minutes: total,
            stops: stop_views,
            warnings: warnings.iter().map(warning_view).collect(),
        });
    }
    tx.commit().await.map_err(ApiError::from)?;

    Ok(Json(RoutingDayView {
        date,
        from,
        to,
        status,
        technicians: technician_views,
        suggestions,
    }))
}

/// Bounded reassignment suggestions: a stop whose entry leg shrinks by at
/// least 10 minutes when its technician's day start anchors closer. One
/// extra matrix call per (stop, candidate), capped at two candidates per
/// stop and six suggestions per day — suggestion-grade, never a solver.
async fn reassign_suggestions(
    provider: &dyn RoutingProvider,
    journeys: &[JourneyBuild],
    mut suggestions: Vec<TravelSuggestionView>,
) -> Vec<TravelSuggestionView> {
    const MAX_REASSIGN: usize = 6;
    const MIN_SAVINGS: i64 = 10;
    let mut emitted = 0_usize;
    'outer: for (ti, journey) in journeys.iter().enumerate() {
        if journey.stops.len() > 12 {
            continue;
        }
        for (si, stop) in journey.stops.iter().enumerate() {
            if emitted >= MAX_REASSIGN {
                break 'outer;
            }
            let Some(place) = stop.place.as_ref() else {
                continue;
            };
            let Some((latitude, longitude)) = place.coordinates() else {
                continue;
            };
            let Some(Some(current_in)) = journey.leg_minutes.get(si).copied() else {
                continue;
            };
            let anchor = GeoPoint::new(latitude, longitude);
            let mut candidates: Vec<(usize, f64)> = journeys
                .iter()
                .enumerate()
                .filter(|(bi, other)| {
                    *bi != ti
                        && other.has_start
                        && other.stops.len() <= 12
                        && !other
                            .day_spans
                            .iter()
                            .any(|(s, e)| *s < stop.ends_at && *e > stop.starts_at)
                })
                .filter_map(|(bi, other)| {
                    let start = other.start.as_ref()?;
                    let (other_lat, other_lon) = start.place.coordinates()?;
                    Some((
                        bi,
                        anchor.haversine_meters(&GeoPoint::new(other_lat, other_lon)),
                    ))
                })
                .collect();
            candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            candidates.truncate(2);
            for (bi, _) in candidates {
                let other = &journeys[bi];
                let candidate_from = other
                    .stops
                    .iter()
                    .rev()
                    .find(|s| s.starts_at < stop.starts_at)
                    .and_then(|s| s.place.as_ref())
                    .or_else(|| other.start.as_ref().map(|s| &s.place));
                let Some(candidate_from) = candidate_from else {
                    continue;
                };
                if !candidate_from.has_coordinates() {
                    continue;
                }
                let request = MatrixRequest::new(
                    vec![svc::place_from_travel_place(candidate_from)],
                    vec![svc::place_from_travel_place(place)],
                    TravelMode::Driving,
                );
                let Ok(response) = provider.matrix(&request).await else {
                    continue;
                };
                let candidate_in = response
                    .cell(0, 0)
                    .and_then(|cell| cell.duration_seconds)
                    .map(estimate_minutes);
                let Some(candidate_in) = candidate_in else {
                    continue;
                };
                let saved = current_in - candidate_in;
                if saved < MIN_SAVINGS {
                    continue;
                }
                let stop_label = stop
                    .label
                    .clone()
                    .unwrap_or_else(|| stop.appointment_id.to_string());
                suggestions.push(TravelSuggestionView {
                    kind: "reassign".into(),
                    appointment_id: stop.appointment_id,
                    swap_with_appointment_id: None,
                    target_user_id: Some(other.user_id),
                    minutes_saved: saved,
                    label: format!(
                        "Move {stop_label} to {} — about {saved} fewer travel minutes.",
                        other.display_name
                    ),
                });
                emitted += 1;
                if emitted >= MAX_REASSIGN {
                    break 'outer;
                }
            }
        }
    }
    suggestions
}

// --- journey start points ----------------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct JourneyStartQuery {
    /// Whose journey; defaults to the caller (another user requires
    /// owner/admin).
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
    /// The day; defaults to today (UTC).
    #[schema(value_type = Option<String>, format = "date")]
    pub date: Option<NaiveDate>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct JourneyStartView {
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    pub date: NaiveDate,
    /// The start point travel estimates will run from (explicit row when
    /// set, else the auto-suggestion).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved: Option<StartPointView>,
    /// The dispatcher's explicit row, when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explicit: Option<StartPointView>,
}

/// GET /api/dispatch/journey-start — the resolved start point plus the
/// explicit override, if any.
#[utoipa::path(operation_id = "dispatch_journey_start_get", get, path = "/api/dispatch/journey-start", tag = "scheduling", security(("bearer" = [])),
    params(JourneyStartQuery),
    responses((status = 200, body = JourneyStartView)))]
pub async fn journey_start_get(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<JourneyStartQuery>,
) -> ApiResult<Json<JourneyStartView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let user_id = target_user(&authed.principal, q.user_id, "journey starts")?;
    let org_id = authed.principal.user.org_id;
    let date = q.date.unwrap_or_else(|| Utc::now().date_naive());
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let explicit = core_db::routing::get_journey_start_point(&mut *tx, org_id, user_id, date).await?;
    let resolved = svc::resolve_start_point(&mut tx, org_id, user_id, date).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(JourneyStartView {
        user_id,
        date,
        resolved: resolved.map(|s| StartPointView {
            kind: s.place.kind,
            label: s.place.label,
            source: s.source,
            site_id: s.place.site_id,
            latitude: s.place.latitude,
            longitude: s.place.longitude,
        }),
        explicit: explicit.map(|row| StartPointView {
            kind: row.kind,
            label: row.label,
            source: "explicit".into(),
            site_id: row.site_id,
            latitude: row.latitude,
            longitude: row.longitude,
        }),
    }))
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SetJourneyStartRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
    pub date: NaiveDate,
    /// `home` | `office` | `site` | `address`.
    pub kind: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub site_id: Option<Uuid>,
    pub address: Option<String>,
    pub label: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct JourneyStartMutationResponse {
    pub start_point: StartPointView,
    #[schema(value_type = uuid::Uuid)]
    pub start_point_id: Uuid,
    pub mutation: instant::MutationMeta,
}

/// PUT /api/dispatch/journey-start — set (or replace) the technician's
/// start point for one day.
#[utoipa::path(operation_id = "dispatch_journey_start_set", put, path = "/api/dispatch/journey-start", tag = "scheduling", security(("bearer" = [])),
    request_body = SetJourneyStartRequest,
    responses((status = 200, body = JourneyStartMutationResponse),
              (status = 404, description = "site not found in scope"),
              (status = 409, description = "idempotency key reused"),
              (status = 422, description = "invalid start point")))]
pub async fn journey_start_set(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<SetJourneyStartRequest>,
) -> ApiResult<Json<JourneyStartMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::Edit))?;
    let user_id = target_user(&authed.principal, req.user_id, "journey starts")?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;

    let kind = req.kind.trim().to_ascii_lowercase();
    if !matches!(kind.as_str(), "home" | "office" | "site" | "address") {
        return Err(ApiError::Validation(
            "kind must be home, office, site, or address".into(),
        ));
    }
    if kind == "site" && req.site_id.is_none() {
        return Err(ApiError::Validation(
            "a site start point needs site_id".into(),
        ));
    }
    if let (Some(latitude), Some(longitude)) = (req.latitude, req.longitude) {
        if !GeoPoint::new(latitude, longitude).is_plausible() {
            return Err(ApiError::Validation(
                "coordinates are outside the valid range".into(),
            ));
        }
    }
    if kind != "site"
        && req.site_id.is_none()
        && req.address.as_deref().unwrap_or("").trim().is_empty()
        && req.latitude.is_none()
    {
        return Err(ApiError::Validation(
            "give an address or coordinates for this start point".into(),
        ));
    }
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut ctx| async move {
        let existing =
            core_db::routing::get_journey_start_point(&mut *ctx.tx, org_id, user_id, req.date)
                .await?;
        let row_id = existing.as_ref().map(|row| row.id).unwrap_or_else(Uuid::now_v7);
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "journey_start_point",
                entity_id: Some(row_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        if let Some(site_id) = req.site_id {
            core_db::clients::get_site(&mut *ctx.tx, org_id, site_id)
                .await
                .map_err(|_| ApiError::NotFound)?;
        }
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "journey_start_point",
                entity_id: row_id,
                version: req.instant.version,
            },
        )
        .await?;
        let row = core_db::routing::JourneyStartPointRow {
            id: row_id,
            org_id,
            user_id,
            day: req.date,
            kind: kind.clone(),
            site_id: req.site_id,
            address: req.address.clone(),
            label: req.label.clone(),
            latitude: req.latitude,
            longitude: req.longitude,
            created_by: actor,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let saved = core_db::routing::upsert_journey_start_point(&mut ctx.tx, &row).await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(actor),
                action: "dispatch.journey_start_set".into(),
                entity_type: Some("journey_start_point".into()),
                entity_id: Some(saved.id.to_string()),
                detail: serde_json::json!({
                    "user_id": user_id,
                    "day": saved.day,
                    "kind": saved.kind,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::JOURNEY_START_POINT_SET,
            "journey_start_point",
            Some(saved.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::JourneyStartPointSet {
                start_point_id: saved.id,
                user_id: saved.user_id,
                day: saved.day,
                kind: saved.kind.clone(),
                site_id: saved.site_id,
            })
            .unwrap_or_default(),
        )
        .await?;
        let view = StartPointView {
            kind: saved.kind.clone(),
            label: saved.label.clone(),
            source: "explicit".into(),
            site_id: saved.site_id,
            latitude: saved.latitude,
            longitude: saved.longitude,
        };
        let response = JourneyStartMutationResponse {
            start_point: view,
            start_point_id: saved.id,
            mutation: instant::MutationMeta {
                seq,
                version: seq,
                updated_at: saved.updated_at,
                before: existing
                    .as_ref()
                    .map(|row| serde_json::to_value(row).unwrap_or(serde_json::Value::Null)),
                after: serde_json::to_value(&saved).unwrap_or(serde_json::Value::Null),
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "journey_start_point",
                entity_id: saved.id,
                op: "journey_start_set",
                seq,
                before: response.mutation.before.clone(),
                after: serde_json::to_value(&saved).unwrap_or(serde_json::Value::Null),
                updated_at: saved.updated_at,
                key: key.as_deref(),
                payload: &response,
                provenance: instant::ProvenanceContext::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(JourneyStartMutationResponse {
                start_point: response.start_point,
                start_point_id: response.start_point_id,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

// --- mileage / time / expense reconciliation ---------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct ReconciliationQuery {
    /// Inclusive window start (default: 14 days before `to`).
    #[schema(value_type = Option<String>, format = "date-time")]
    pub from: Option<DateTime<Utc>>,
    /// Exclusive window end (default: now). Capped at 366 days.
    #[schema(value_type = Option<String>, format = "date-time")]
    pub to: Option<DateTime<Utc>>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ReconciliationRow {
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    pub display_name: String,
    pub draft_count: i64,
    pub submitted_count: i64,
    pub expensed_count: i64,
    /// Distance normalized to `mileage_rate_unit`.
    pub distance: f64,
    pub distance_unit: String,
    pub minutes: i64,
    /// Linked expenses total in the org's base currency.
    pub expense_minor: i64,
    /// Mileage reimbursement owed to the technician at the org rate.
    pub reimbursement_minor: i64,
    /// Linked expenses flagged billable (client-facing).
    pub billable_minor: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReconciliationView {
    #[schema(value_type = String, format = "date-time")]
    pub from: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub to: DateTime<Utc>,
    pub currency: String,
    pub mileage_rate_minor: i64,
    pub mileage_rate_unit: String,
    pub reimburse_techs: bool,
    pub rows: Vec<ReconciliationRow>,
    pub totals: ReconciliationRow,
}

const KM_PER_MILE: f64 = 1.609_344;

/// GET /api/dispatch/reconciliation — planned-vs-actual rollup over the
/// existing travel drafts and their linked expenses: mileage, time, and
/// reimbursements per technician, in the org's base currency.
#[utoipa::path(operation_id = "dispatch_reconciliation", get, path = "/api/dispatch/reconciliation", tag = "scheduling", security(("bearer" = [])),
    params(ReconciliationQuery),
    responses((status = 200, body = ReconciliationView)))]
pub async fn reconciliation(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ReconciliationQuery>,
) -> ApiResult<Json<ReconciliationView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let to = q.to.unwrap_or_else(Utc::now);
    let from = q.from.unwrap_or(to - Duration::days(14));
    if to <= from || to - from > Duration::days(366) {
        return Err(ApiError::Validation("range must be within 366 days".into()));
    }
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let policy = svc::travel_policy_for(&mut tx, org_id).await?;
    let currency = svc::base_currency(&mut tx, org_id).await?;
    let drafts = core_db::dispatch::list_travel_drafts_for_reconciliation(
        &mut *tx,
        org_id,
        q.user_id,
        from,
        to,
    )
    .await?;
    let expense_ids: Vec<Uuid> = drafts.iter().filter_map(|d| d.expense_id).collect();
    let expenses = core_db::costs::list_expenses_by_ids(&mut *tx, org_id, &expense_ids).await?;
    let members = core_db::users::list_users(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let expense_map: HashMap<Uuid, &TicketExpense> =
        expenses.iter().map(|e| (e.id, e)).collect();
    let name_map: HashMap<Uuid, String> = members
        .iter()
        .map(|u| (u.id, u.display_name.clone()))
        .collect();

    let mut rows: Vec<ReconciliationRow> = Vec::new();
    for user in members.iter() {
        let user_drafts: Vec<&core_db::dispatch::TravelDraftRow> =
            drafts.iter().filter(|d| d.user_id == user.id).collect();
        if user_drafts.is_empty() {
            continue;
        }
        rows.push(rollup_row(
            user.id,
            name_map
                .get(&user.id)
                .cloned()
                .unwrap_or_else(|| user.id.to_string()),
            &user_drafts,
            &expense_map,
            &currency,
            &policy,
        ));
    }
    let all: Vec<&core_db::dispatch::TravelDraftRow> = drafts.iter().collect();
    let totals = rollup_row(
        Uuid::nil(),
        "Total".into(),
        &all,
        &expense_map,
        &currency,
        &policy,
    );

    Ok(Json(ReconciliationView {
        from,
        to,
        currency,
        mileage_rate_minor: policy.mileage_rate_minor,
        mileage_rate_unit: policy.mileage_rate_unit.clone(),
        reimburse_techs: policy.reimburse_techs,
        rows,
        totals,
    }))
}

fn rollup_row(
    user_id: Uuid,
    display_name: String,
    drafts: &[&core_db::dispatch::TravelDraftRow],
    expenses: &HashMap<Uuid, &TicketExpense>,
    currency: &str,
    policy: &core_domain::settings::TravelPolicy,
) -> ReconciliationRow {
    let mut distance = 0.0_f64;
    let mut minutes = 0_i64;
    let mut submitted = 0_i64;
    let mut expensed = 0_i64;
    let mut expense_minor = 0_i64;
    let mut billable_minor = 0_i64;
    let mut reimbursement_minor = 0_i64;
    for draft in drafts {
        // Normalize every distance to the rate's unit.
        let normalized = if draft.distance_unit == policy.mileage_rate_unit {
            draft.distance
        } else if draft.distance_unit == "mi" {
            // km rate, miles logged.
            draft.distance * KM_PER_MILE
        } else {
            draft.distance / KM_PER_MILE
        };
        distance += normalized;
        minutes += (draft.ended_at - draft.started_at).num_minutes().max(0);
        if draft.status == "submitted" {
            submitted += 1;
        }
        if policy.reimburse_techs {
            reimbursement_minor +=
                (normalized * policy.mileage_rate_minor as f64).round() as i64;
        }
        if let Some(expense_id) = draft.expense_id {
            if let Some(expense) = expenses.get(&expense_id) {
                if expense.currency == currency {
                    expense_minor += expense.amount_minor;
                    if expense.billable {
                        billable_minor += expense.amount_minor;
                    }
                }
                expensed += 1;
            }
        }
    }
    ReconciliationRow {
        user_id,
        display_name,
        draft_count: drafts.len() as i64,
        submitted_count: submitted,
        expensed_count: expensed,
        distance: (distance * 100.0).round() / 100.0,
        distance_unit: policy.mileage_rate_unit.clone(),
        minutes,
        expense_minor,
        reimbursement_minor,
        billable_minor,
    }
}

// --- expense from travel draft ----------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateTravelExpenseRequest {
    #[serde(flatten)]
    pub instant: instant::MutationParams,
    /// Overrides the computed reimbursement amount when present.
    pub amount_minor: Option<i64>,
    /// Overrides the description when present.
    pub description: Option<String>,
}

/// POST /api/dispatch/travel-drafts/{id}/expense — book the draft's
/// mileage as a ticket expense at the org rate (the reimbursement the
/// business owes the technician or bills the client). One expense per
/// draft, idempotent through the standard envelope.
#[utoipa::path(operation_id = "dispatch_travel_draft_expense", post, path = "/api/dispatch/travel-drafts/{id}/expense", tag = "scheduling", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = CreateTravelExpenseRequest,
    responses((status = 200, body = crate::routes::costs::ExpenseMutationResponse),
              (status = 404, description = "draft not found in scope"),
              (status = 409, description = "draft already expensed or key reused"),
              (status = 422, description = "invalid amount")))]
pub async fn travel_draft_expense(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateTravelExpenseRequest>,
) -> ApiResult<Json<crate::routes::costs::ExpenseMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ViewFinancials)?;
    let org_id = authed.principal.user.org_id;
    let actor = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, actor, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "travel_draft_expense",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let draft = core_db::dispatch::get_travel_draft(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if draft.expense_id.is_some() {
            return Err(ApiError::Conflict(
                "this travel draft already has an expense".into(),
            ));
        }
        let policy = svc::travel_policy_for(&mut ctx.tx, org_id).await?;
        let currency = svc::base_currency(&mut ctx.tx, org_id).await?;
        let normalized = if draft.distance_unit == policy.mileage_rate_unit {
            draft.distance
        } else if draft.distance_unit == "mi" {
            draft.distance * KM_PER_MILE
        } else {
            draft.distance / KM_PER_MILE
        };
        let computed = (normalized * policy.mileage_rate_minor as f64).round() as i64;
        let amount_minor = req.amount_minor.unwrap_or(computed);
        if amount_minor < 0 {
            return Err(ApiError::Validation("amount_minor must be >= 0".into()));
        }
        let description = req.description.clone().unwrap_or_else(|| {
            format!(
                "Mileage: {:.1} {} ({}) — travel_draft:{}",
                draft.distance, draft.distance_unit, draft.vehicle, draft.id
            )
        });
        let today = Utc::now().date_naive();
        let snap =
            crate::routes::fx::lock_snapshot(&mut ctx.tx, org_id, amount_minor, &currency, today)
                .await?;
        let expense = TicketExpense {
            id: Uuid::now_v7(),
            org_id,
            ticket_id: draft.ticket_id,
            description,
            amount_minor,
            currency: currency.clone(),
            billable: draft.billable,
            fx_rate_used: snap.as_ref().map(|s| s.rate_used.clone()),
            fx_rate_date: snap.as_ref().map(|s| s.rate_date),
            fx_source: snap.as_ref().map(|s| s.source.clone()),
            base_amount_minor: snap.as_ref().map(|s| s.base_amount_minor),
            invoice_line_id: None,
            incurred_by: Some(draft.user_id),
            incurred_at: draft.ended_at,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        core_db::costs::insert_expense(&mut *ctx.tx, &expense).await?;
        core_db::dispatch::attach_travel_draft_expense(&mut ctx.tx, org_id, id, expense.id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(actor),
                action: "travel_draft.expensed".into(),
                entity_type: Some("travel_draft".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "expense_id": expense.id,
                    "amount_minor": expense.amount_minor,
                    "currency": expense.currency,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::TICKET_EXPENSE_RECORDED,
            "ticket_expense",
            Some(expense.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::TicketExpenseRecorded {
                expense: expense.clone(),
            })
            .unwrap_or_default(),
        )
        .await?;
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "ticket_expense",
                entity_id: expense.id,
                version: None,
            },
        )
        .await?;
        let stored = crate::routes::costs::ExpenseMutationResponse {
            expense: expense.clone(),
            mutation: instant::MutationMeta {
                seq,
                version: seq,
                updated_at: Utc::now(),
                before: None,
                after: serde_json::Value::Null,
                idempotency_key: None,
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "ticket_expense",
                entity_id: expense.id,
                op: "travel_draft_expense",
                seq,
                before: None,
                after: serde_json::to_value(&expense).unwrap_or(serde_json::Value::Null),
                updated_at: Utc::now(),
                key: key.as_deref(),
                payload: &stored,
                provenance: instant::ProvenanceContext::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(crate::routes::costs::ExpenseMutationResponse {
                expense,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}
