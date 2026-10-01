//! Day-pack API (Wave 32 lane 8, `p32-today-day-pack`; migration 0107).
//!
//! Public surface:
//!   * `GET /day-pack?date=` — the tech's trip-preparation read model: the
//!     day's scheduled stops split into route-aware TRIP segments (a stop
//!     whose recorded leg left the office opens a new segment: re-collect
//!     there), the consolidated grab list per trip with per-line provenance
//!     (manual pick / org rule / checklist mention), where to grab each item
//!     (the tech's van first, then a pickup location), honest availability
//!     with holders and due dates, plus the tomorrow preview when the van
//!     does not return to the office first.
//!   * `/day-pack/picks` — manual per-ticket picks (the "this job needs X"
//!     intent lane 6's panel can now pin). A pick moves nothing.
//!   * `/day-pack/ticks` — tick-off-to-collect plan notes, one row per
//!     (user, day, line key). Ticking never touches the movement ledger.
//!   * `/day-pack/rules` — the typed org mapping behind recommendations and
//!     default kits (scope = ticket type / ticket category / board).
//!
//! Non-purpose: any stock movement. Collection stays with `POST
//! /tool-checkouts` and the consume/transfer legs; this surface plans.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationMeta, MutationOutcome, MutationParams};
use crate::org_tx;
use crate::routes::routing::StartPointView;
use crate::routes::scheduling::sched;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::Extensions;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use core_domain::day_pack as pack;
use core_domain::entities::{Actor, AuditEvent};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

const PICK_ENTITY: &str = "ticket_stock_pick";
const RULE_ENTITY: &str = "dispatch_pack_rule";
const TICK_ENTITY: &str = "day_pack_tick";
const CATALOG_LIMIT: i64 = 500;

/// Admin-module shorthand (the toolbox config surface is admin work).
fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

/// Ticket-module shorthand (pinning an item to a job is ticket work).
fn tickets(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action,
    }
}

pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(&format!("{base}/day-pack"), get(day_pack_read))
        .route(
            &format!("{base}/day-pack/picks"),
            get(list_picks).post(create_pick),
        )
        .route(
            &format!("{base}/day-pack/picks/{{id}}"),
            axum::routing::delete(delete_pick),
        )
        .route(
            &format!("{base}/day-pack/ticks"),
            post(create_tick).delete(delete_tick),
        )
        .route(
            &format!("{base}/day-pack/rules"),
            get(list_rules).post(create_rule),
        )
        .route(
            &format!("{base}/day-pack/rules/{{id}}"),
            axum::routing::patch(patch_rule).delete(delete_rule),
        )
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct DayPackQuery {
    /// The day to assemble; defaults to today (UTC). Callers with a local
    /// timezone pass their own date so the panel agrees with the wall clock.
    pub date: Option<NaiveDate>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DayPackView {
    pub date: NaiveDate,
    #[schema(value_type = uuid::Uuid)]
    pub user_id: Uuid,
    pub display_name: String,
    pub timezone: String,
    pub has_stops: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_point: Option<StartPointView>,
    pub trips: Vec<DayPackTripView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tomorrow: Option<DayPackTomorrowView>,
    pub counts: DayPackCounts,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DayPackCounts {
    pub stops: i64,
    pub lines: i64,
    pub collected: i64,
    /// Lines whose availability is `unavailable` or `not_configured`.
    pub unavailable: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DayPackTripView {
    pub index: i32,
    #[schema(value_type = String, format = "date-time")]
    pub opens_at: DateTime<Utc>,
    /// The trip opens where the tech can re-collect: the office when the day
    /// starts there or a logged office return precedes it.
    pub opens_at_office: bool,
    pub stops: Vec<DayPackStopView>,
    pub lines: Vec<DayPackLineView>,
    pub kits: Vec<DayPackKitView>,
    #[serde(default)]
    pub reminders: Vec<DayPackReminderView>,
    /// Typed sentences: a ticked quantity that cannot cover the remaining
    /// stops of this trip.
    #[serde(default)]
    pub coverage_warnings: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DayPackStopView {
    #[schema(value_type = uuid::Uuid)]
    pub appointment_id: Uuid,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    #[schema(nullable = true)]
    pub ticket_number: Option<String>,
    pub title: String,
    #[schema(value_type = String, format = "date-time")]
    pub starts_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub ends_at: DateTime<Utc>,
    pub kind: String,
    #[schema(nullable = true)]
    pub site_label: Option<String>,
    #[schema(nullable = true)]
    pub client_name: Option<String>,
    pub travel_minutes: i64,
    #[schema(nullable = true)]
    pub travel_from_kind: Option<String>,
    #[schema(nullable = true)]
    pub travel_quality: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DayPackLineView {
    /// Stable identity for ticks: `pick:<id>` / `rule:<id>:<ticket>` /
    /// `checklist:<id>`.
    pub line_key: String,
    /// `pick` | `rule` | `checklist`.
    pub provenance: String,
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: Uuid,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub appointment_id: Option<Uuid>,
    #[schema(value_type = uuid::Uuid)]
    pub stock_item_id: Uuid,
    pub item_name: String,
    #[schema(nullable = true)]
    pub secondary: Option<String>,
    pub unit: String,
    pub is_tool: bool,
    pub qty_milli: i64,
    #[schema(nullable = true)]
    pub note: Option<String>,
    pub source: Option<DayPackSourceView>,
    pub availability: DayPackAvailabilityView,
    pub tick: Option<DayPackTickView>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DayPackSourceView {
    #[schema(value_type = uuid::Uuid)]
    pub location_id: Uuid,
    pub name: String,
    pub kind: String,
    pub available_units: i64,
    /// True when this is the tech's own van/truck: gear already aboard.
    pub is_tech_vehicle: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DayPackAvailabilityView {
    /// `available` | `partial` | `unavailable` | `not_configured`.
    pub status: String,
    /// Tools count whole units; consumables carry milli-units.
    pub required_units: i64,
    pub available_units: i64,
    #[serde(default)]
    pub holders: Vec<DayPackHolderView>,
    #[schema(nullable = true)]
    pub next_due_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DayPackHolderView {
    #[schema(value_type = Option<uuid::Uuid>)]
    pub user_id: Option<Uuid>,
    #[schema(nullable = true)]
    pub display_name: Option<String>,
    #[schema(nullable = true)]
    pub client_name: Option<String>,
    /// `checkout` | `loan` | `out`.
    pub kind: String,
    #[schema(nullable = true)]
    pub due_at: Option<DateTime<Utc>>,
    #[schema(nullable = true)]
    pub expected_return_at: Option<NaiveDate>,
    pub overdue: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DayPackKitView {
    pub line_key: String,
    #[schema(value_type = uuid::Uuid)]
    pub kit_id: Uuid,
    pub name: String,
    #[schema(nullable = true)]
    pub code: Option<String>,
    pub member_count: i64,
    pub ready: bool,
    pub required_units: i64,
    pub available_units: i64,
    #[serde(default)]
    pub missing: Vec<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub appointment_id: Option<Uuid>,
    pub tick: Option<DayPackTickView>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DayPackReminderView {
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: Uuid,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DayPackTickView {
    pub line_key: String,
    #[schema(nullable = true)]
    pub note: Option<String>,
    #[schema(value_type = String, format = "date-time")]
    pub ticked_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DayPackTomorrowView {
    pub date: NaiveDate,
    /// `direct_to_site` | `start_point_unknown` when the list shows;
    /// `office_start` / `no_stops` never reach this view.
    pub reason: String,
    #[schema(nullable = true)]
    pub start_point_kind: Option<String>,
    #[schema(nullable = true)]
    pub start_point_label: Option<String>,
    pub stops: Vec<DayPackStopView>,
    pub lines: Vec<DayPackLineView>,
    pub kits: Vec<DayPackKitView>,
    #[serde(default)]
    pub reminders: Vec<DayPackReminderView>,
}

// ---------------------------------------------------------------------------
// Read model
// ---------------------------------------------------------------------------

#[utoipa::path(operation_id = "day_pack_read", get, path = "/api/day-pack",
    tag = "scheduling", security(("bearer" = [])), params(DayPackQuery),
    responses((status = 200, body = DayPackView)))]
pub async fn day_pack_read(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<DayPackQuery>,
) -> ApiResult<Json<DayPackView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let day = q.date.unwrap_or_else(|| Utc::now().date_naive());
    let tomorrow = day + Duration::days(1);

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;

    let view = compose_day_pack(&mut tx, org_id, user_id, day, tomorrow).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(view))
}

/// One ticket's rule scopes + the stop it first appears on.
#[derive(Debug, Clone, Default)]
struct TicketScope {
    board_id: Option<Uuid>,
    type_id: Option<Uuid>,
    category_id: Option<Uuid>,
}

fn rule_matches(rule: &core_db::day_pack::PackRuleRow, scope: &TicketScope) -> bool {
    let target = match rule.scope_kind.as_str() {
        "ticket_type" => scope.type_id,
        "ticket_category" => scope.category_id,
        "board" => scope.board_id,
        _ => None,
    };
    target == Some(rule.scope_id)
}

#[derive(Clone)]
struct LineSeed {
    line_key: String,
    provenance: &'static str,
    ticket_id: Uuid,
    appointment_id: Option<Uuid>,
    stock_item_id: Uuid,
    qty_milli: i64,
    note: Option<String>,
}

#[derive(Clone)]
struct KitSeed {
    line_key: String,
    kit_id: Uuid,
    ticket_id: Uuid,
    appointment_id: Option<Uuid>,
}

#[derive(Clone)]
struct ReminderSeed {
    ticket_id: Uuid,
    text: String,
}

/// The stock lookups a set of lines needs: levels for consumables, serial
/// units for tools, and the tech's vehicles.
struct Availability {
    levels: HashMap<Uuid, Vec<core_db::day_pack::ConsumableLevelRow>>,
    units: HashMap<Uuid, Vec<core_db::day_pack::ToolUnitRow>>,
    vehicles: Vec<Uuid>,
    now: DateTime<Utc>,
}

fn vehicle_rank(location_id: Uuid, vehicles: &[Uuid]) -> u8 {
    if vehicles.contains(&location_id) {
        0
    } else {
        1
    }
}

fn location_rank(kind: &str) -> u8 {
    match kind {
        "warehouse" => 0,
        "shelf" => 1,
        _ => 2,
    }
}

impl Availability {
    fn source_for(
        &self,
        item: &core_db::day_pack::PackItemRow,
        qty_milli: i64,
    ) -> (Option<DayPackSourceView>, DayPackAvailabilityView) {
        if item.is_tool {
            let units = self.units.get(&item.id).cloned().unwrap_or_default();
            let required = (qty_milli.max(1) + 999) / 1000;
            let available = units.iter().filter(|u| u.status == "in_stock").count() as i64;
            // Group in-stock units per location; van first, then warehouse,
            // then shelf, then whatever has stock.
            let mut by_location: HashMap<Uuid, (String, String, i64)> = HashMap::new();
            for unit in units.iter().filter(|u| u.status == "in_stock") {
                if let Some(location_id) = unit.location_id {
                    let entry = by_location.entry(location_id).or_insert_with(|| {
                        (
                            unit.location_name.clone().unwrap_or_default(),
                            unit.location_kind.clone().unwrap_or_default(),
                            0,
                        )
                    });
                    entry.2 += 1;
                }
            }
            let mut candidates: Vec<(Uuid, String, String, i64)> = by_location
                .into_iter()
                .map(|(id, (name, kind, count))| (id, name, kind, count))
                .collect();
            candidates.sort_by_key(|(id, _, kind, _)| {
                (vehicle_rank(*id, &self.vehicles), location_rank(kind))
            });
            let source = candidates
                .first()
                .map(|(id, name, kind, count)| DayPackSourceView {
                    location_id: *id,
                    name: if name.is_empty() {
                        kind.clone()
                    } else {
                        name.clone()
                    },
                    kind: kind.clone(),
                    available_units: *count,
                    is_tech_vehicle: self.vehicles.contains(id),
                });
            let status = if available >= required {
                "available"
            } else if available > 0 {
                "partial"
            } else if units.is_empty() {
                "not_configured"
            } else {
                "unavailable"
            };
            let mut holders: Vec<DayPackHolderView> = Vec::new();
            let mut next_due: Option<DateTime<Utc>> = None;
            for unit in units.iter().filter(|u| u.status != "in_stock") {
                let kind = if unit.holder_user_id.is_some() {
                    "checkout"
                } else if unit.loan_client_name.is_some() {
                    "loan"
                } else {
                    "out"
                };
                let due_at = unit.due_at;
                if let Some(due) = due_at {
                    next_due = Some(next_due.map_or(due, |current| current.min(due)));
                }
                let overdue = pack::is_overdue(due_at, self.now)
                    || unit
                        .expected_return_at
                        .is_some_and(|date| date < self.now.date_naive());
                let holder = DayPackHolderView {
                    user_id: unit.holder_user_id,
                    display_name: unit.holder_name.clone(),
                    client_name: unit.loan_client_name.clone(),
                    kind: kind.to_string(),
                    due_at,
                    expected_return_at: unit.expected_return_at,
                    overdue,
                };
                if !holders.iter().any(|existing| {
                    existing.user_id == holder.user_id
                        && existing.client_name == holder.client_name
                        && existing.kind == holder.kind
                }) {
                    holders.push(holder);
                }
            }
            (
                source,
                DayPackAvailabilityView {
                    status: status.to_string(),
                    required_units: required,
                    available_units: available,
                    holders,
                    next_due_at: next_due,
                },
            )
        } else {
            let levels = self.levels.get(&item.id).cloned().unwrap_or_default();
            let available: i64 = levels.iter().map(|l| l.qty_milli).sum();
            let required = qty_milli;
            let mut candidates: Vec<&core_db::day_pack::ConsumableLevelRow> =
                levels.iter().collect();
            candidates.sort_by_key(|l| {
                (
                    vehicle_rank(l.location_id, &self.vehicles),
                    location_rank(&l.location_kind),
                )
            });
            let source = candidates.first().map(|l| DayPackSourceView {
                location_id: l.location_id,
                name: l.location_name.clone(),
                kind: l.location_kind.clone(),
                available_units: l.qty_milli,
                is_tech_vehicle: self.vehicles.contains(&l.location_id),
            });
            let status = if available >= required {
                "available"
            } else if available > 0 {
                "partial"
            } else if levels.is_empty() {
                "not_configured"
            } else {
                "unavailable"
            };
            (
                source,
                DayPackAvailabilityView {
                    status: status.to_string(),
                    required_units: required,
                    available_units: available,
                    holders: Vec::new(),
                    next_due_at: None,
                },
            )
        }
    }
}

fn line_view(
    seed: &LineSeed,
    item: &core_db::day_pack::PackItemRow,
    availability: &Availability,
    ticks: &HashMap<String, DayPackTickView>,
) -> DayPackLineView {
    let (source, availability_view) = availability.source_for(item, seed.qty_milli);
    DayPackLineView {
        line_key: seed.line_key.clone(),
        provenance: seed.provenance.to_string(),
        ticket_id: seed.ticket_id,
        appointment_id: seed.appointment_id,
        stock_item_id: seed.stock_item_id,
        item_name: item.name.clone(),
        secondary: item.sku.clone().or_else(|| item.part_number.clone()),
        unit: item.unit.clone(),
        is_tool: item.is_tool,
        qty_milli: seed.qty_milli,
        note: seed.note.clone(),
        source,
        availability: availability_view,
        tick: ticks.get(&seed.line_key).cloned(),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_ticket_seeds(
    ticket_id: Uuid,
    appointment_id: Option<Uuid>,
    scope: &TicketScope,
    picks: &[&core_db::day_pack::TicketPickRow],
    rules: &[&core_db::day_pack::PackRuleRow],
    reminders: &[&core_db::day_pack::ChecklistReminderRow],
    catalog: &[core_db::day_pack::PackItemRow],
    lines: &mut Vec<LineSeed>,
    kits: &mut Vec<KitSeed>,
    plain_reminders: &mut Vec<ReminderSeed>,
) {
    let mut seen: HashMap<Uuid, &'static str> = HashMap::new();
    for pick in picks {
        lines.push(LineSeed {
            line_key: pack::line_key("pick", pick.id),
            provenance: "pick",
            ticket_id,
            appointment_id,
            stock_item_id: pick.stock_item_id,
            qty_milli: pick.qty_milli,
            note: pick.note.clone(),
        });
        seen.insert(pick.stock_item_id, "pick");
    }
    for rule in rules.iter().filter(|rule| rule_matches(rule, scope)) {
        match rule.target_kind.as_str() {
            "item" => {
                let Some(item_id) = rule.stock_item_id else {
                    continue;
                };
                if seen.contains_key(&item_id) {
                    continue;
                }
                lines.push(LineSeed {
                    line_key: format!("rule:{}:{}", rule.id, ticket_id),
                    provenance: "rule",
                    ticket_id,
                    appointment_id,
                    stock_item_id: item_id,
                    qty_milli: rule.qty_milli,
                    note: rule.note.clone(),
                });
                seen.insert(item_id, "rule");
            }
            "kit" => {
                let Some(kit_id) = rule.kit_id else {
                    continue;
                };
                kits.push(KitSeed {
                    line_key: format!("kit:{}:{}", rule.id, ticket_id),
                    kit_id,
                    ticket_id,
                    appointment_id,
                });
            }
            _ => {}
        }
    }
    for reminder in reminders {
        let candidates: Vec<pack::ChecklistCandidate<'_>> = catalog
            .iter()
            .map(|item| pack::ChecklistCandidate {
                name: item.name.as_str(),
                sku: item.sku.as_deref(),
                part_number: item.part_number.as_deref(),
            })
            .collect();
        match pack::checklist_match(&reminder.text, &candidates) {
            Some(index) => {
                let item = &catalog[index];
                if seen.contains_key(&item.id) {
                    continue;
                }
                lines.push(LineSeed {
                    line_key: pack::line_key("checklist", reminder.id),
                    provenance: "checklist",
                    ticket_id,
                    appointment_id,
                    stock_item_id: item.id,
                    qty_milli: 1000,
                    note: Some(reminder.text.clone()),
                });
                seen.insert(item.id, "checklist");
            }
            None => plain_reminders.push(ReminderSeed {
                ticket_id,
                text: reminder.text.clone(),
            }),
        }
    }
}

fn stop_view(row: &core_db::day_pack::ScheduledStopRow) -> DayPackStopView {
    let travel_from_kind = row
        .travel_from
        .as_ref()
        .and_then(|value| value.get("kind"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    DayPackStopView {
        appointment_id: row.appointment_id,
        ticket_id: row.ticket_id,
        ticket_number: row.ticket_number.clone(),
        title: row.title.clone(),
        starts_at: row.starts_at,
        ends_at: row.ends_at,
        kind: row.kind.clone(),
        site_label: row.site_label.clone(),
        client_name: row.client_name.clone(),
        travel_minutes: i64::from(row.travel_minutes),
        travel_from_kind,
        travel_quality: row.travel_quality.clone(),
    }
}

fn coverage_warnings(lines: &[DayPackLineView]) -> Vec<String> {
    let mut required: HashMap<Uuid, (bool, Vec<i64>)> = HashMap::new();
    let mut collected: HashMap<Uuid, i64> = HashMap::new();
    let mut names: HashMap<Uuid, String> = HashMap::new();
    for line in lines {
        let entry = required
            .entry(line.stock_item_id)
            .or_insert_with(|| (line.is_tool, Vec::new()));
        entry.1.push(line.qty_milli);
        names.insert(line.stock_item_id, line.item_name.clone());
        if line.tick.is_some() {
            *collected.entry(line.stock_item_id).or_insert(0) += line.qty_milli;
        }
    }
    let mut warnings = Vec::new();
    for (item_id, (is_tool, per_stop)) in required {
        let need = pack::required_milli(is_tool, &per_stop);
        let have = collected.get(&item_id).copied().unwrap_or(0);
        let short = pack::coverage_shortfall(need, have);
        if have > 0 && short > 0 {
            let name = names.get(&item_id).cloned().unwrap_or_default();
            warnings.push(format!(
                "{} — {} short for the remaining stops on this trip.",
                name,
                short as f64 / 1000.0
            ));
        }
    }
    warnings
}

async fn compose_day_pack(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    user_id: Uuid,
    day: NaiveDate,
    tomorrow: NaiveDate,
) -> ApiResult<DayPackView> {
    let (display_name, timezone) = {
        let user = core_db::users::get_user(&mut *conn, org_id, user_id).await?;
        (user.display_name.clone(), user.timezone.clone())
    };
    let from = day.and_hms_opt(0, 0, 0).expect("midnight").and_utc();
    let to = from + Duration::days(1);
    let tomorrow_from = tomorrow.and_hms_opt(0, 0, 0).expect("midnight").and_utc();
    let tomorrow_to = tomorrow_from + Duration::days(1);

    let stops = core_db::day_pack::scheduled_stops_between(conn, org_id, user_id, from, to).await?;
    let tomorrow_stops = core_db::day_pack::scheduled_stops_between(
        conn,
        org_id,
        user_id,
        tomorrow_from,
        tomorrow_to,
    )
    .await?;

    let start_point = crate::routing::resolve_start_point(conn, org_id, user_id, day).await?;
    let tomorrow_start =
        crate::routing::resolve_start_point(conn, org_id, user_id, tomorrow).await?;

    let mut ticket_ids: Vec<Uuid> = stops
        .iter()
        .filter_map(|stop| stop.ticket_id)
        .chain(tomorrow_stops.iter().filter_map(|stop| stop.ticket_id))
        .collect();
    ticket_ids.sort();
    ticket_ids.dedup();

    let picks = core_db::day_pack::list_ticket_picks(conn, org_id, &ticket_ids).await?;
    let rules: Vec<core_db::day_pack::PackRuleRow> =
        core_db::day_pack::list_pack_rules(conn, org_id)
            .await?
            .into_iter()
            .filter(|rule| rule.active)
            .collect();
    let reminders = core_db::day_pack::open_checklist_items(conn, org_id, &ticket_ids).await?;
    let catalog = if reminders.is_empty() {
        Vec::new()
    } else {
        core_db::stock::list_stock_items(&mut *conn, org_id, None, true, CATALOG_LIMIT)
            .await?
            .into_iter()
            .map(|item| core_db::day_pack::PackItemRow {
                id: item.id,
                name: item.name,
                sku: item.sku,
                part_number: item.part_number,
                unit: item.unit,
                kind: item.kind,
                is_tool: item.is_tool,
                active: item.active,
            })
            .collect()
    };

    // Scope per ticket (rules match on board / type / category).
    let mut scopes: HashMap<Uuid, TicketScope> = HashMap::new();
    for stop in stops.iter().chain(tomorrow_stops.iter()) {
        if let Some(ticket_id) = stop.ticket_id {
            scopes.entry(ticket_id).or_insert_with(|| TicketScope {
                board_id: stop.board_id,
                type_id: stop.type_id,
                category_id: stop.category_id,
            });
        }
    }

    // Seeds: the first stop a ticket appears on carries its lines.
    let mut seen_ticket: Vec<Uuid> = Vec::new();
    let mut stop_lines: Vec<Vec<LineSeed>> = vec![Vec::new(); stops.len()];
    let mut stop_kits: Vec<Vec<KitSeed>> = vec![Vec::new(); stops.len()];
    let mut stop_reminders: Vec<Vec<ReminderSeed>> = vec![Vec::new(); stops.len()];
    for (index, stop) in stops.iter().enumerate() {
        let Some(ticket_id) = stop.ticket_id else {
            continue;
        };
        if seen_ticket.contains(&ticket_id) {
            continue;
        }
        seen_ticket.push(ticket_id);
        let scope = scopes.get(&ticket_id).cloned().unwrap_or_default();
        let ticket_picks: Vec<&core_db::day_pack::TicketPickRow> =
            picks.iter().filter(|p| p.ticket_id == ticket_id).collect();
        let ticket_rules: Vec<&core_db::day_pack::PackRuleRow> = rules.iter().collect();
        let ticket_reminders: Vec<&core_db::day_pack::ChecklistReminderRow> = reminders
            .iter()
            .filter(|r| r.ticket_id == ticket_id)
            .collect();
        build_ticket_seeds(
            ticket_id,
            Some(stop.appointment_id),
            &scope,
            &ticket_picks,
            &ticket_rules,
            &ticket_reminders,
            &catalog,
            &mut stop_lines[index],
            &mut stop_kits[index],
            &mut stop_reminders[index],
        );
    }

    let mut tomorrow_lines_seed: Vec<LineSeed> = Vec::new();
    let mut tomorrow_kits_seed: Vec<KitSeed> = Vec::new();
    let mut tomorrow_reminders_seed: Vec<ReminderSeed> = Vec::new();
    let mut seen_tomorrow: Vec<Uuid> = Vec::new();
    for stop in tomorrow_stops.iter() {
        let Some(ticket_id) = stop.ticket_id else {
            continue;
        };
        if seen_tomorrow.contains(&ticket_id) {
            continue;
        }
        seen_tomorrow.push(ticket_id);
        let scope = scopes.get(&ticket_id).cloned().unwrap_or_default();
        let ticket_picks: Vec<&core_db::day_pack::TicketPickRow> =
            picks.iter().filter(|p| p.ticket_id == ticket_id).collect();
        let ticket_rules: Vec<&core_db::day_pack::PackRuleRow> = rules.iter().collect();
        let ticket_reminders: Vec<&core_db::day_pack::ChecklistReminderRow> = reminders
            .iter()
            .filter(|r| r.ticket_id == ticket_id)
            .collect();
        build_ticket_seeds(
            ticket_id,
            None,
            &scope,
            &ticket_picks,
            &ticket_rules,
            &ticket_reminders,
            &catalog,
            &mut tomorrow_lines_seed,
            &mut tomorrow_kits_seed,
            &mut tomorrow_reminders_seed,
        );
    }

    // Availability lookups once for both days.
    let mut item_ids: Vec<Uuid> = stop_lines
        .iter()
        .flatten()
        .map(|line| line.stock_item_id)
        .chain(tomorrow_lines_seed.iter().map(|line| line.stock_item_id))
        .collect();
    item_ids.sort();
    item_ids.dedup();
    let items: Vec<core_db::day_pack::PackItemRow> =
        core_db::day_pack::items_by_ids(conn, org_id, &item_ids).await?;
    let by_id: HashMap<Uuid, &core_db::day_pack::PackItemRow> =
        items.iter().map(|item| (item.id, item)).collect();
    let tool_ids: Vec<Uuid> = items
        .iter()
        .filter(|item| item.is_tool)
        .map(|item| item.id)
        .collect();
    let consumable_ids: Vec<Uuid> = items
        .iter()
        .filter(|item| !item.is_tool)
        .map(|item| item.id)
        .collect();
    let mut levels: HashMap<Uuid, Vec<core_db::day_pack::ConsumableLevelRow>> = HashMap::new();
    if !consumable_ids.is_empty() {
        for row in core_db::day_pack::consumable_levels(conn, org_id, &consumable_ids).await? {
            levels.entry(row.stock_item_id).or_default().push(row);
        }
    }
    let mut units: HashMap<Uuid, Vec<core_db::day_pack::ToolUnitRow>> = HashMap::new();
    if !tool_ids.is_empty() {
        for row in core_db::day_pack::tool_units(conn, org_id, &tool_ids).await? {
            units.entry(row.stock_item_id).or_default().push(row);
        }
    }
    let vehicles: Vec<Uuid> = core_db::day_pack::tech_vehicles(conn, org_id, user_id)
        .await?
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let availability = Availability {
        levels,
        units,
        vehicles,
        now: Utc::now(),
    };

    let ticks_today: HashMap<String, DayPackTickView> =
        core_db::day_pack::list_ticks(conn, org_id, user_id, day)
            .await?
            .into_iter()
            .map(|tick| {
                (
                    tick.line_key.clone(),
                    DayPackTickView {
                        line_key: tick.line_key,
                        note: tick.note,
                        ticked_at: tick.created_at,
                    },
                )
            })
            .collect();
    let ticks_tomorrow: HashMap<String, DayPackTickView> =
        core_db::day_pack::list_ticks(conn, org_id, user_id, tomorrow)
            .await?
            .into_iter()
            .map(|tick| {
                (
                    tick.line_key.clone(),
                    DayPackTickView {
                        line_key: tick.line_key,
                        note: tick.note,
                        ticked_at: tick.created_at,
                    },
                )
            })
            .collect();

    // Trip segmentation from the travel model's recorded leg origins.
    let trip_stops: Vec<pack::TripStop> = stops
        .iter()
        .map(|stop| pack::TripStop {
            appointment_id: stop.appointment_id,
            leg_from_kind: stop
                .travel_from
                .as_ref()
                .and_then(|value| value.get("kind"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
        })
        .collect();
    let segments = pack::trip_segments(
        start_point.as_ref().map(|s| s.place.kind.as_str()),
        &trip_stops,
    );

    let mut lines_total = 0i64;
    let mut collected_total = 0i64;
    let mut unavailable_total = 0i64;
    let mut trips: Vec<DayPackTripView> = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        let mut lines: Vec<DayPackLineView> = Vec::new();
        let mut kits: Vec<DayPackKitView> = Vec::new();
        let mut reminders_view: Vec<DayPackReminderView> = Vec::new();
        for stop_index in segment.start..segment.end {
            for seed in &stop_lines[stop_index] {
                if let Some(item) = by_id.get(&seed.stock_item_id) {
                    lines.push(line_view(seed, item, &availability, &ticks_today));
                }
            }
            for seed in &stop_kits[stop_index] {
                if let Some(kit) = kit_view(conn, org_id, seed, &ticks_today).await? {
                    kits.push(kit);
                }
            }
            for reminder in &stop_reminders[stop_index] {
                reminders_view.push(DayPackReminderView {
                    ticket_id: reminder.ticket_id,
                    text: reminder.text.clone(),
                });
            }
        }
        lines_total += lines.len() as i64;
        collected_total += lines.iter().filter(|line| line.tick.is_some()).count() as i64
            + kits.iter().filter(|kit| kit.tick.is_some()).count() as i64;
        unavailable_total += lines
            .iter()
            .filter(|line| {
                line.availability.status == "unavailable"
                    || line.availability.status == "not_configured"
            })
            .count() as i64;
        let opens_at = stops
            .get(segment.start)
            .map(|stop| stop.starts_at)
            .unwrap_or_else(Utc::now);
        trips.push(DayPackTripView {
            index: index as i32 + 1,
            opens_at,
            opens_at_office: segment.opens_at_office,
            stops: stops[segment.start..segment.end]
                .iter()
                .map(stop_view)
                .collect(),
            coverage_warnings: coverage_warnings(&lines),
            lines,
            kits,
            reminders: reminders_view,
        });
    }

    // Tomorrow preview: only when the van does not return to the office
    // first. Availability for tomorrow's lines uses the same context.
    let reason = pack::tomorrow_preview_reason(
        !tomorrow_stops.is_empty(),
        tomorrow_start.as_ref().map(|s| s.place.kind.as_str()),
    );
    let tomorrow_view = if pack::tomorrow_preview_shows(reason) {
        let mut lines: Vec<DayPackLineView> = Vec::new();
        let mut kits: Vec<DayPackKitView> = Vec::new();
        for seed in &tomorrow_lines_seed {
            if let Some(item) = by_id.get(&seed.stock_item_id) {
                lines.push(line_view(seed, item, &availability, &ticks_tomorrow));
            }
        }
        for seed in &tomorrow_kits_seed {
            if let Some(kit) = kit_view(conn, org_id, seed, &ticks_tomorrow).await? {
                kits.push(kit);
            }
        }
        // Tomorrow's lines count toward the panel's totals: they are part
        // of tonight's pack.
        lines_total += lines.len() as i64;
        collected_total += lines.iter().filter(|line| line.tick.is_some()).count() as i64;
        unavailable_total += lines
            .iter()
            .filter(|line| {
                line.availability.status == "unavailable"
                    || line.availability.status == "not_configured"
            })
            .count() as i64;
        Some(DayPackTomorrowView {
            date: tomorrow,
            reason: reason.to_string(),
            start_point_kind: tomorrow_start.as_ref().map(|s| s.place.kind.clone()),
            start_point_label: tomorrow_start.as_ref().and_then(|s| s.place.label.clone()),
            stops: tomorrow_stops.iter().map(stop_view).collect(),
            lines,
            kits,
            reminders: tomorrow_reminders_seed
                .iter()
                .map(|reminder| DayPackReminderView {
                    ticket_id: reminder.ticket_id,
                    text: reminder.text.clone(),
                })
                .collect(),
        })
    } else {
        None
    };

    Ok(DayPackView {
        date: day,
        user_id,
        display_name,
        timezone,
        has_stops: !stops.is_empty(),
        start_point: start_point.as_ref().map(|start| StartPointView {
            kind: start.place.kind.clone(),
            label: start.place.label.clone(),
            source: start.source.clone(),
            site_id: start.place.site_id,
            latitude: start.place.latitude,
            longitude: start.place.longitude,
        }),
        trips,
        tomorrow: tomorrow_view,
        counts: DayPackCounts {
            stops: stops.len() as i64,
            lines: lines_total,
            collected: collected_total,
            unavailable: unavailable_total,
        },
    })
}

async fn kit_view(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    seed: &KitSeed,
    ticks: &HashMap<String, DayPackTickView>,
) -> ApiResult<Option<DayPackKitView>> {
    let Ok(kit) = core_db::tools::get_kit(&mut *conn, org_id, seed.kit_id).await else {
        return Ok(None);
    };
    let members = core_db::tools::list_kit_members(&mut *conn, org_id, seed.kit_id).await?;
    let readiness = core_db::tools::kit_availability(&mut *conn, org_id, seed.kit_id, None).await?;
    let required: i64 = readiness.iter().map(|r| r.required_units).sum();
    let available: i64 = readiness.iter().map(|r| r.available_units).sum();
    let missing: Vec<String> = readiness
        .iter()
        .filter(|r| r.available_units < r.required_units)
        .map(|r| r.item_name.clone())
        .collect();
    Ok(Some(DayPackKitView {
        line_key: seed.line_key.clone(),
        kit_id: kit.id,
        name: kit.name,
        code: kit.code,
        member_count: members.len() as i64,
        ready: missing.is_empty(),
        required_units: required,
        available_units: available,
        missing,
        ticket_id: Some(seed.ticket_id),
        appointment_id: seed.appointment_id,
        tick: ticks.get(&seed.line_key).cloned(),
    }))
}

// ---------------------------------------------------------------------------
// Picks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TicketPickView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub stock_item_id: Uuid,
    pub qty_milli: i64,
    #[schema(nullable = true)]
    pub note: Option<String>,
    #[schema(value_type = String, format = "date-time")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PickMutationResponse {
    pub pick: TicketPickView,
    pub mutation: MutationMeta,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct TicketPicksQuery {
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TicketPickRowView {
    #[serde(flatten)]
    pub pick: TicketPickView,
    #[schema(nullable = true)]
    pub item_name: Option<String>,
    pub is_tool: bool,
}

#[utoipa::path(operation_id = "day_pack_picks_list", get, path = "/api/day-pack/picks",
    tag = "scheduling", security(("bearer" = [])), params(TicketPicksQuery),
    responses((status = 200, body = [TicketPickRowView])))]
pub async fn list_picks(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<TicketPicksQuery>,
) -> ApiResult<Json<Vec<TicketPickRowView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let picks = core_db::day_pack::list_ticket_picks(&mut tx, org_id, &[q.ticket_id]).await?;
    let item_ids: Vec<Uuid> = picks.iter().map(|pick| pick.stock_item_id).collect();
    let items = core_db::day_pack::items_by_ids(&mut tx, org_id, &item_ids).await?;
    let names: HashMap<Uuid, (String, bool)> = items
        .into_iter()
        .map(|item| (item.id, (item.name, item.is_tool)))
        .collect();
    let rows = picks
        .into_iter()
        .map(|pick| {
            let (item_name, is_tool) = names
                .get(&pick.stock_item_id)
                .cloned()
                .unwrap_or((String::new(), false));
            TicketPickRowView {
                pick: TicketPickView {
                    id: pick.id,
                    ticket_id: pick.ticket_id,
                    stock_item_id: pick.stock_item_id,
                    qty_milli: pick.qty_milli,
                    note: pick.note.clone(),
                    created_at: pick.created_at,
                },
                item_name: Some(item_name),
                is_tool,
            }
        })
        .collect();
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreatePickRequest {
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub stock_item_id: Uuid,
    #[serde(default)]
    pub qty_milli: Option<i64>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(flatten)]
    pub instant: MutationParams,
}

#[utoipa::path(operation_id = "day_pack_pick_create", post, path = "/api/day-pack/picks",
    tag = "scheduling", security(("bearer" = [])), request_body = CreatePickRequest,
    responses((status = 200, body = PickMutationResponse), (status = 409), (status = 422)))]
pub async fn create_pick(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreatePickRequest>,
) -> ApiResult<Json<PickMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let qty_milli = req.qty_milli.unwrap_or(1000);
    if qty_milli <= 0 {
        return Err(ApiError::Validation("quantity must be positive".into()));
    }
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();
    let pick_id = Uuid::now_v7();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: PICK_ENTITY,
                entity_id: Some(pick_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Both anchors must exist in this org before an intent row lands.
        let ticket = core_db::tickets::get_ticket(&mut *ctx.tx, org_id, req.ticket_id).await?;
        let item = core_db::stock::get_stock_item(&mut *ctx.tx, org_id, req.stock_item_id).await?;
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: PICK_ENTITY,
                entity_id: pick_id,
                version: req.instant.version,
            },
        )
        .await?;
        core_db::day_pack::insert_ticket_pick(
            &mut ctx.tx,
            pick_id,
            org_id,
            req.ticket_id,
            req.stock_item_id,
            qty_milli,
            req.note.as_deref(),
            Some(user_id),
        )
        .await?;
        let now = Utc::now();
        let view = TicketPickView {
            id: pick_id,
            ticket_id: req.ticket_id,
            stock_item_id: req.stock_item_id,
            qty_milli,
            note: req.note.clone(),
            created_at: now,
        };
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "day_pack.pick_created".into(),
                entity_type: Some(PICK_ENTITY.into()),
                entity_id: Some(pick_id.to_string()),
                detail: serde_json::json!({
                    "ticket_id": req.ticket_id,
                    "ticket_number": ticket.number.0,
                    "stock_item_id": req.stock_item_id,
                    "item_name": item.name,
                    "qty_milli": qty_milli,
                }),
                ip: None,
                occurred_at: now,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: PICK_ENTITY,
                entity_id: pick_id,
                op: "create",
                seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(serde_json::Value::Null),
                updated_at: now,
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(PickMutationResponse {
                pick: view,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

#[utoipa::path(operation_id = "day_pack_pick_delete", delete, path = "/api/day-pack/picks/{id}",
    tag = "scheduling", security(("bearer" = [])),
    responses((status = 200, body = PickMutationResponse), (status = 404)))]
pub async fn delete_pick(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<PickMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, tickets(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let existing = core_db::day_pack::get_ticket_pick(&mut ctx.tx, org_id, id).await?;
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: PICK_ENTITY,
                entity_id: id,
                version: None,
            },
        )
        .await?;
        if !core_db::day_pack::delete_ticket_pick(&mut ctx.tx, org_id, id).await? {
            return Err(ApiError::NotFound);
        }
        let view = TicketPickView {
            id: existing.id,
            ticket_id: existing.ticket_id,
            stock_item_id: existing.stock_item_id,
            qty_milli: existing.qty_milli,
            note: existing.note.clone(),
            created_at: existing.created_at,
        };
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "day_pack.pick_removed".into(),
                entity_type: Some(PICK_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "ticket_id": existing.ticket_id,
                    "stock_item_id": existing.stock_item_id,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: PICK_ENTITY,
                entity_id: id,
                op: "delete",
                seq,
                before: serde_json::to_value(&view).ok(),
                after: serde_json::Value::Null,
                updated_at: Utc::now(),
                key: None,
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(PickMutationResponse {
                pick: view,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

// ---------------------------------------------------------------------------
// Ticks (plan notes)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TickMutationResponse {
    pub tick: DayPackTickView,
    pub mutation: MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateTickRequest {
    pub date: NaiveDate,
    pub line_key: String,
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(flatten)]
    pub instant: MutationParams,
}

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct DeleteTickQuery {
    pub date: NaiveDate,
    pub line_key: String,
}

#[utoipa::path(operation_id = "day_pack_tick_create", post, path = "/api/day-pack/ticks",
    tag = "scheduling", security(("bearer" = [])), request_body = CreateTickRequest,
    responses((status = 200, body = TickMutationResponse), (status = 422)))]
pub async fn create_tick(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateTickRequest>,
) -> ApiResult<Json<TickMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let line_key = req.line_key.trim().to_string();
    if line_key.is_empty() || line_key.chars().count() > 200 {
        return Err(ApiError::Validation(
            "line_key must be 1..=200 characters".into(),
        ));
    }
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: TICK_ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let (tick_id, ticked_at) = core_db::day_pack::upsert_tick(
            &mut ctx.tx,
            org_id,
            user_id,
            req.date,
            &line_key,
            req.ticket_id,
            req.note.as_deref(),
        )
        .await?;
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: TICK_ENTITY,
                entity_id: tick_id,
                version: req.instant.version,
            },
        )
        .await?;
        let view = DayPackTickView {
            line_key: line_key.clone(),
            note: req.note.clone(),
            ticked_at,
        };
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "day_pack.line_ticked".into(),
                entity_type: Some(TICK_ENTITY.into()),
                entity_id: Some(tick_id.to_string()),
                detail: serde_json::json!({
                    "date": req.date,
                    "line_key": line_key,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: TICK_ENTITY,
                entity_id: tick_id,
                op: "upsert",
                seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(serde_json::Value::Null),
                updated_at: ticked_at,
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TickMutationResponse {
                tick: view,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

#[utoipa::path(operation_id = "day_pack_tick_delete", delete, path = "/api/day-pack/ticks",
    tag = "scheduling", security(("bearer" = [])), params(DeleteTickQuery),
    responses((status = 200, body = TickMutationResponse), (status = 404)))]
pub async fn delete_tick(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<DeleteTickQuery>,
) -> ApiResult<Json<TickMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let line_key = q.line_key.trim().to_string();
    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let existing =
            core_db::day_pack::find_tick(&mut ctx.tx, org_id, user_id, q.date, &line_key)
                .await?
                .ok_or(ApiError::NotFound)?;
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: TICK_ENTITY,
                entity_id: existing.0,
                version: None,
            },
        )
        .await?;
        core_db::day_pack::delete_tick(&mut ctx.tx, org_id, user_id, q.date, &line_key).await?;
        let view = DayPackTickView {
            line_key: line_key.clone(),
            note: existing.2.clone(),
            ticked_at: existing.1,
        };
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "day_pack.line_unticked".into(),
                entity_type: Some(TICK_ENTITY.into()),
                entity_id: Some(existing.0.to_string()),
                detail: serde_json::json!({
                    "date": q.date,
                    "line_key": line_key,
                }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: TICK_ENTITY,
                entity_id: existing.0,
                op: "delete",
                seq,
                before: serde_json::to_value(&view).ok(),
                after: serde_json::Value::Null,
                updated_at: Utc::now(),
                key: None,
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(TickMutationResponse {
                tick: view,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

// ---------------------------------------------------------------------------
// Dispatch rules (recommendations + default kits)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PackRuleView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    pub scope_kind: String,
    #[schema(value_type = uuid::Uuid)]
    pub scope_id: Uuid,
    pub scope_name: String,
    pub target_kind: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub stock_item_id: Option<Uuid>,
    #[schema(nullable = true)]
    pub item_name: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub kit_id: Option<Uuid>,
    #[schema(nullable = true)]
    pub kit_name: Option<String>,
    pub qty_milli: i64,
    #[schema(nullable = true)]
    pub note: Option<String>,
    pub active: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RuleMutationResponse {
    pub rule: PackRuleView,
    pub mutation: MutationMeta,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateRuleRequest {
    pub scope_kind: String,
    #[schema(value_type = uuid::Uuid)]
    pub scope_id: Uuid,
    pub target_kind: String,
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub stock_item_id: Option<Uuid>,
    #[serde(default)]
    #[schema(value_type = Option<uuid::Uuid>)]
    pub kit_id: Option<Uuid>,
    #[serde(default)]
    pub qty_milli: Option<i64>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(flatten)]
    pub instant: MutationParams,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PatchRuleRequest {
    pub active: bool,
    #[serde(flatten)]
    pub instant: MutationParams,
}

#[utoipa::path(operation_id = "day_pack_rules_list", get, path = "/api/day-pack/rules",
    tag = "scheduling", security(("bearer" = [])),
    responses((status = 200, body = [PackRuleView])))]
pub async fn list_rules(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<PackRuleView>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, sched(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let rules = core_db::day_pack::list_pack_rules(&mut tx, org_id).await?;
    let views = enrich_rules(&mut tx, org_id, rules).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(views))
}

async fn enrich_rules(
    conn: &mut sqlx::PgConnection,
    org_id: Uuid,
    rules: Vec<core_db::day_pack::PackRuleRow>,
) -> ApiResult<Vec<PackRuleView>> {
    let item_ids: Vec<Uuid> = rules.iter().filter_map(|r| r.stock_item_id).collect();
    let items = core_db::day_pack::items_by_ids(conn, org_id, &item_ids).await?;
    let names: HashMap<Uuid, String> = items.into_iter().map(|i| (i.id, i.name)).collect();
    let mut views = Vec::with_capacity(rules.len());
    for rule in rules {
        let scope_name =
            core_db::day_pack::scope_name(conn, org_id, &rule.scope_kind, rule.scope_id)
                .await?
                .unwrap_or_else(|| "missing scope".to_string());
        let kit_name = match rule.kit_id {
            Some(kit_id) => core_db::tools::get_kit(&mut *conn, org_id, kit_id)
                .await
                .ok()
                .map(|kit| kit.name),
            None => None,
        };
        views.push(PackRuleView {
            id: rule.id,
            scope_kind: rule.scope_kind,
            scope_id: rule.scope_id,
            scope_name,
            target_kind: rule.target_kind,
            stock_item_id: rule.stock_item_id,
            item_name: rule.stock_item_id.and_then(|id| names.get(&id).cloned()),
            kit_id: rule.kit_id,
            kit_name,
            qty_milli: rule.qty_milli,
            note: rule.note,
            active: rule.active,
        });
    }
    Ok(views)
}

#[utoipa::path(operation_id = "day_pack_rule_create", post, path = "/api/day-pack/rules",
    tag = "scheduling", security(("bearer" = [])), request_body = CreateRuleRequest,
    responses((status = 200, body = RuleMutationResponse), (status = 409), (status = 422)))]
pub async fn create_rule(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateRuleRequest>,
) -> ApiResult<Json<RuleMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    if !pack::is_valid_scope_kind(&req.scope_kind) {
        return Err(ApiError::Validation(
            "scope_kind must be ticket_type, ticket_category, or board".into(),
        ));
    }
    if !pack::is_valid_target_kind(&req.target_kind) {
        return Err(ApiError::Validation(
            "target_kind must be item or kit".into(),
        ));
    }
    match req.target_kind.trim() {
        "item" if req.stock_item_id.is_none() => {
            return Err(ApiError::Validation(
                "an item rule needs stock_item_id".into(),
            ));
        }
        "kit" if req.kit_id.is_none() => {
            return Err(ApiError::Validation("a kit rule needs kit_id".into()));
        }
        _ => {}
    }
    let qty_milli = req.qty_milli.unwrap_or(1000);
    if qty_milli <= 0 {
        return Err(ApiError::Validation("quantity must be positive".into()));
    }
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();
    let rule_id = Uuid::now_v7();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: RULE_ENTITY,
                entity_id: Some(rule_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        if !core_db::day_pack::scope_exists(&mut ctx.tx, org_id, &req.scope_kind, req.scope_id)
            .await?
        {
            return Err(ApiError::Validation(
                "the chosen scope does not exist in this organization".into(),
            ));
        }
        if let Some(item_id) = req.stock_item_id {
            core_db::stock::get_stock_item(&mut *ctx.tx, org_id, item_id).await?;
        }
        if let Some(kit_id) = req.kit_id {
            core_db::tools::get_kit(&mut ctx.tx, org_id, kit_id).await?;
        }
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: RULE_ENTITY,
                entity_id: rule_id,
                version: req.instant.version,
            },
        )
        .await?;
        core_db::day_pack::insert_pack_rule(
            &mut ctx.tx,
            rule_id,
            org_id,
            req.scope_kind.trim(),
            req.scope_id,
            req.target_kind.trim(),
            req.stock_item_id,
            req.kit_id,
            qty_milli,
            req.note.as_deref(),
            Some(user_id),
        )
        .await?;
        let scope_name =
            core_db::day_pack::scope_name(&mut ctx.tx, org_id, req.scope_kind.trim(), req.scope_id)
                .await?
                .unwrap_or_else(|| "missing scope".to_string());
        let item_name = match req.stock_item_id {
            Some(item_id) => core_db::stock::get_stock_item(&mut *ctx.tx, org_id, item_id)
                .await
                .ok()
                .map(|item| item.name),
            None => None,
        };
        let kit_name = match req.kit_id {
            Some(kit_id) => core_db::tools::get_kit(&mut ctx.tx, org_id, kit_id)
                .await
                .ok()
                .map(|kit| kit.name),
            None => None,
        };
        let view = PackRuleView {
            id: rule_id,
            scope_kind: req.scope_kind.trim().to_string(),
            scope_id: req.scope_id,
            scope_name,
            target_kind: req.target_kind.trim().to_string(),
            stock_item_id: req.stock_item_id,
            item_name,
            kit_id: req.kit_id,
            kit_name,
            qty_milli,
            note: req.note.clone(),
            active: true,
        };
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "day_pack.rule_created".into(),
                entity_type: Some(RULE_ENTITY.into()),
                entity_id: Some(rule_id.to_string()),
                detail: serde_json::to_value(&view).unwrap_or(serde_json::Value::Null),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: RULE_ENTITY,
                entity_id: rule_id,
                op: "create",
                seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(serde_json::Value::Null),
                updated_at: Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(RuleMutationResponse {
                rule: view,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

#[utoipa::path(operation_id = "day_pack_rule_patch", patch, path = "/api/day-pack/rules/{id}",
    tag = "scheduling", security(("bearer" = [])), request_body = PatchRuleRequest,
    responses((status = 200, body = RuleMutationResponse), (status = 404)))]
pub async fn patch_rule(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchRuleRequest>,
) -> ApiResult<Json<RuleMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();
    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: RULE_ENTITY,
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: RULE_ENTITY,
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        if !core_db::day_pack::set_pack_rule_active(&mut ctx.tx, org_id, id, req.active).await? {
            return Err(ApiError::NotFound);
        }
        let rules = core_db::day_pack::list_pack_rules(&mut ctx.tx, org_id).await?;
        let Some(rule) = rules.into_iter().find(|rule| rule.id == id) else {
            return Err(ApiError::NotFound);
        };
        let views = enrich_rules(&mut ctx.tx, org_id, vec![rule]).await?;
        let view = views.into_iter().next().ok_or(ApiError::NotFound)?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "day_pack.rule_updated".into(),
                entity_type: Some(RULE_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({ "active": req.active }),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: RULE_ENTITY,
                entity_id: id,
                op: "patch",
                seq,
                before: None,
                after: serde_json::to_value(&view).unwrap_or(serde_json::Value::Null),
                updated_at: Utc::now(),
                key: key.as_deref(),
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(RuleMutationResponse {
                rule: view,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

#[utoipa::path(operation_id = "day_pack_rule_delete", delete, path = "/api/day-pack/rules/{id}",
    tag = "scheduling", security(("bearer" = [])),
    responses((status = 200, body = RuleMutationResponse), (status = 404)))]
pub async fn delete_rule(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<RuleMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, admin(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let rules = core_db::day_pack::list_pack_rules(&mut ctx.tx, org_id).await?;
        let Some(rule) = rules.into_iter().find(|rule| rule.id == id) else {
            return Err(ApiError::NotFound);
        };
        let views = enrich_rules(&mut ctx.tx, org_id, vec![rule]).await?;
        let view = views.into_iter().next().ok_or(ApiError::NotFound)?;
        let seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: RULE_ENTITY,
                entity_id: id,
                version: None,
            },
        )
        .await?;
        if !core_db::day_pack::delete_pack_rule(&mut ctx.tx, org_id, id).await? {
            return Err(ApiError::NotFound);
        }
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "day_pack.rule_removed".into(),
                entity_type: Some(RULE_ENTITY.into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::to_value(&view).unwrap_or(serde_json::Value::Null),
                ip: None,
                occurred_at: Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: RULE_ENTITY,
                entity_id: id,
                op: "delete",
                seq,
                before: serde_json::to_value(&view).ok(),
                after: serde_json::Value::Null,
                updated_at: Utc::now(),
                key: None,
                payload: &view,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(RuleMutationResponse {
                rule: view,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}
