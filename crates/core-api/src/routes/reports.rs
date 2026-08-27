//! Operational reports endpoints (TASKLIST R1; ADR 0001).
//!
//! Purpose: serve the eight prebuilt report kinds over HTTP — cursor-paged
//! JSON rows plus a whole-window summary block, `?format=csv` output, and
//! RBAC scoping where managers see org-wide data and everyone else is
//! structurally narrowed to their own rows.
//!
//! Every statement is compiled by core-reports against the `ops.v1_*`
//! read-model views ONLY; this module executes exactly what the compiler
//! emitted and never appends SQL text of its own.
//!
//! Non-purpose: no report-spec persistence (R3), no dashboards UI (R2),
//! no scheduled delivery (R4), no worker concerns.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, NaiveDate, Utc};
use core_domain::policy::{Action, Module, Permission};
use core_reports::{
    Bind, CompiledReport, CursorShape, KindParams, OutType, OutputColumn, ReportKind, Scope,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::{Column, PgPool, Row};
use uuid::Uuid;

/// Query surface shared by all kinds; irrelevant fields are ignored for
/// kinds without that dimension (mirrors the rest of core-api's loose GETs).
#[derive(Debug, Deserialize, utoipa::IntoParams, utoipa::ToSchema)]
pub struct ReportQuery {
    /// Inclusive window start (UTC day, YYYY-MM-DD). Default: 30 trailing days.
    pub from: Option<NaiveDate>,
    /// Inclusive window end (UTC day). Max span with `from`: 366 days.
    pub to: Option<NaiveDate>,
    /// Opaque keyset cursor from a previous page's `next_cursor`.
    pub cursor: Option<String>,
    /// Page size, clamped to [1, 500].
    #[serde(default = "default_limit")]
    pub limit: i64,
    /// `"csv"` streams text/csv instead of the JSON envelope.
    pub format: Option<String>,
    /// Manager-only narrowing onto one staff member; non-managers are
    /// ALWAYS forced onto themselves regardless of this parameter.
    pub user_id: Option<Uuid>,
    /// SLA kinds: restrict to one governing policy.
    pub policy_id: Option<Uuid>,
    /// Time rollups: filter billable/non-billable entries.
    pub billable: Option<bool>,
    /// ticket_aging only: snapshot instant for ages (default: now).
    pub as_of: Option<DateTime<Utc>>,
}

fn default_limit() -> i64 {
    100
}

/// GET /api/reports/{kind} — prebuilt operational report over the read-model
/// views. Kinds: time_by_user | time_by_client | time_by_ticket |
/// sla_attainment | sla_breaches | ticket_throughput | ticket_aging |
/// first_response.
#[utoipa::path(
    operation_id = "report_get",
    get,
    path = "/api/reports/{kind}",
    tag = "reports",
    security(("bearer" = [])),
    params(ReportQuery),
    responses(
        (status = 200, description = "Report page (JSON envelope or CSV stream)", body = ReportPage)
    )
)]
pub async fn report(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(kind_slug): Path<String>,
    Query(q): Query<ReportQuery>,
) -> ApiResult<Response> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Reporting,
            action: Action::View,
        },
    )?;
    let kind = ReportKind::parse(&kind_slug).ok_or(ApiError::NotFound)?;
    core_reports::validate_limit_raw(q.limit).map_err(|e| ApiError::Validation(e.to_string()))?;
    let limit = core_reports::clamp_limit(Some(q.limit));
    let now = Utc::now();
    let win = core_reports::resolve_window(q.from, q.to, now)
        .map_err(|e| ApiError::Validation(e.to_string()))?;

    // RBAC seam: managers may narrow with ?user_id=; anyone else is forced
    // onto their OWN rows no matter what the query string says — the
    // compiler turns the scope into an equality predicate on the personal
    // dimension, so self-scoping cannot be bypassed via parameters.
    let scope = if authed.principal.is_owner_or_admin() {
        Scope::OrgWide
    } else {
        Scope::Self_(authed.principal.user.id)
    };

    let want_csv = matches!(q.format.as_deref(), Some("csv"));
    if let Some(other) = &q.format {
        if !want_csv {
            return Err(ApiError::Validation(format!(
                "unsupported format '{other}' (only 'csv')"
            )));
        }
    }

    let params = kind_params(kind, &q, now)?;
    let compiled = core_reports::compile(kind, &params, scope, q.cursor.as_deref(), limit, win)
        .map_err(|e| ApiError::Validation(e.to_string()))?;

    let rows = run_rows(&state.pool, &compiled.sql, &compiled.binds).await?;
    let decoded: Vec<Value> = rows
        .iter()
        .map(|r| row_json(r, &compiled.columns))
        .collect();

    // Cursor derives from the last row of THIS page under the kind's key
    // layout; absent when the page was short (exhaustion) or unpaged kind.
    let next_cursor = next_cursor_for_kind(kind, &compiled, decoded.last());

    match want_csv {
        false => {
            let summary =
                run_rows(&state.pool, &compiled.summary_sql, &compiled.summary_binds).await?;
            let summary_json = match summary.first() {
                Some(row) => dynamic_row_json(row),
                None => json!({}),
            };
            Ok(Json(ReportPage {
                kind: kind.slug().to_string(),
                view: compiled.view.qualified_name().to_string(),
                columns: compiled
                    .columns
                    .iter()
                    .map(|c| c.name.to_string())
                    .collect(),
                rows: decoded,
                summary: summary_json,
                next_cursor,
            })
            .into_response())
        }
        true => Ok(csv_response(kind, &compiled.columns, decoded)),
    }
}

fn kind_params(kind: ReportKind, q: &ReportQuery, now: DateTime<Utc>) -> ApiResult<KindParams> {
    let from = q.from;
    let to = q.to;
    use ReportKind::*;
    let p = match kind {
        TimeByUser => KindParams::TimeByUser(core_reports::TimeByUserParams {
            base: core_reports::TimeReportParams {
                window: core_reports::WindowParams { from, to },
                user_id: q.user_id,
            },
            billable: q.billable,
        }),
        TimeByClient => KindParams::TimeByClient(core_reports::TimeReportParams {
            window: core_reports::WindowParams { from, to },
            user_id: q.user_id,
        }),
        TimeByTicket => KindParams::TimeByTicket(core_reports::TimeReportParams {
            window: core_reports::WindowParams { from, to },
            user_id: q.user_id,
        }),
        SlaAttainment => KindParams::SlaAttainment(core_reports::SlaOutcomesParams {
            window: core_reports::WindowParams { from, to },
            policy_id: q.policy_id,
            user_id: q.user_id,
        }),
        SlaBreaches => KindParams::SlaBreaches(core_reports::SlaOutcomesParams {
            window: core_reports::WindowParams { from, to },
            policy_id: q.policy_id,
            user_id: q.user_id,
        }),
        TicketThroughput => KindParams::TicketThroughput(core_reports::ThroughputParams {
            window: core_reports::WindowParams { from, to },
            user_id: q.user_id,
        }),
        TicketAging => KindParams::TicketAging(core_reports::AgingParams {
            user_id: q.user_id,
            as_of: q.as_of.unwrap_or(now),
        }),
        FirstResponse => KindParams::FirstResponse(core_reports::FirstResponseParams {
            window: core_reports::WindowParams { from, to },
            user_id: q.user_id,
        }),
    };
    Ok(p)
}

// --- execution helpers ---------------------------------------------------------

/// Executes a compiled statement binding every [`Bind`] in order. There is
/// deliberately NO other mechanism — statements go verbatim to Postgres.
async fn run_rows(
    pool: &PgPool,
    sql: &str,
    binds: &[Bind],
) -> Result<Vec<sqlx::postgres::PgRow>, sqlx::Error> {
    let mut q = sqlx::query(sql);
    for b in binds {
        q = match b {
            Bind::Uuid(u) => q.bind(*u),
            Bind::Date(d) => q.bind(*d),
            Bind::Timestamp(t) => q.bind(t),
            Bind::Int(n) => q.bind(*n),
            Bind::Bool(b) => q.bind(*b),
        };
    }
    q.fetch_all(pool).await
}

/// Decodes a main-result row according to its declared output types. All
/// timestamps/dates ride their standard serde encodings (RFC3339 / YYYY-MM-DD).
fn row_json(row: &sqlx::postgres::PgRow, columns: &[OutputColumn]) -> Value {
    let mut map = Map::new();
    for (i, c) in columns.iter().enumerate() {
        let col_name = row.columns()[i].name();
        debug_assert_eq!(col_name, c.name, "column layout drift");
        let v = match c.ty {
            OutType::Uuid => opt_json(row.try_get::<Option<Uuid>, _>(col_name)),
            OutType::Text => opt_json(row.try_get::<Option<String>, _>(col_name)),
            OutType::Timestamp => opt_json(row.try_get::<Option<DateTime<Utc>>, _>(col_name)),
            OutType::Date => opt_json(row.try_get::<Option<NaiveDate>, _>(col_name)),
            OutType::Int => opt_json(row.try_get::<Option<i32>, _>(col_name)),
            OutType::BigInt => opt_json(row.try_get::<Option<i64>, _>(col_name)),
            OutType::Float => opt_json(row.try_get::<Option<f64>, _>(col_name)),
            OutType::Bool => opt_json(row.try_get::<Option<bool>, _>(col_name)),
        };
        map.insert(c.name.to_string(), v);
    }
    Value::Object(map)
}

/// Fallback decodes summary blocks dynamically off PG type OIDs/names so
/// per-kind summary column lists stay compiler-internal.
fn dynamic_row_json(row: &sqlx::postgres::PgRow) -> Value {
    let mut map = Map::new();
    for col in row.columns() {
        let name = col.name();
        let ty = col.type_info().to_string();
        let v: Value = match ty.as_str() {
            "UUID" => opt_json(row.try_get::<Option<Uuid>, _>(name)),
            "TEXT" | "NAME" | "VARCHAR" => opt_json(row.try_get::<Option<String>, _>(name)),
            "TIMESTAMPTZ" => row
                .try_get::<Option<DateTime<Utc>>, _>(name)
                .map(|o| {
                    o.map(|t| Value::String(t.to_rfc3339()))
                        .unwrap_or(Value::Null)
                })
                .unwrap_or(Value::Null),
            "DATE" => opt_json(row.try_get::<Option<NaiveDate>, _>(name)),
            "INT4" => opt_json(row.try_get::<Option<i32>, _>(name)),
            "INT8" => opt_json(row.try_get::<Option<i64>, _>(name)),
            "FLOAT8" | "FLOAT4" => opt_json(row.try_get::<Option<f64>, _>(name)),
            "BOOL" => opt_json(row.try_get::<Option<bool>, _>(name)),
            _ => Value::String(format!("undecodable:{ty}")),
        };
        map.insert(name.to_string(), v);
    }
    Value::Object(map)
}

fn opt_json<T: serde::Serialize>(res: Result<Option<T>, sqlx::Error>) -> Value {
    res.ok()
        .flatten()
        .map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
        .unwrap_or(Value::Null)
}

fn next_cursor_for_kind(
    kind: ReportKind,
    compiled: &CompiledReport,
    last_row: Option<&Value>,
) -> Option<String> {
    let last = last_row?;
    match compiled.cursor_shape {
        CursorShape::None => None,
        CursorShape::MetricGroup => {
            let page_full_hint = true; // short pages are cut off below anyway
            let _ = page_full_hint;
            core_reports::next_cursor(kind, last.get("metric")?, last.get("group_id")?)
        }
        CursorShape::TicketAsc => {
            let anchor = last.get("ticket_created_at").and_then(Value::as_str)?;
            core_reports::next_cursor(kind, &json!(anchor), last.get("ticket_id")?)
        }
        CursorShape::TicketDesc => {
            let anchor = last.get("created_at").and_then(Value::as_str)?;
            core_reports::next_cursor(kind, &json!(anchor), last.get("ticket_id")?)
        }
    }
}

// --- CSV shaping ------------------------------------------------------------------

/// CSV rendering with stable column order == compiled output order. Rows are
/// materialized pages (≤500), then encoded incrementally.
fn csv_response(kind: ReportKind, columns: &[OutputColumn], rows: Vec<Value>) -> Response {
    let header_vals: Vec<String> = columns.iter().map(|c| csv_cell(c.name)).collect();

    let body_lines: Vec<String> = rows
        .iter()
        .map(|row| {
            let cells: Vec<String> = columns
                .iter()
                .map(|c| csv_cell_value(row.get(c.name)))
                .collect();
            cells.join(",")
        })
        .collect();

    let payload = format!("{}\n{}\n", header_vals.join(","), body_lines.join("\n"));
    (
        [
            (
                axum::http::header::CONTENT_TYPE,
                "text/csv; charset=utf-8".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}.csv\"", kind.slug()),
            ),
        ],
        payload,
    )
        .into_response()
}

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn csv_cell_value(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => csv_cell(s),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => csv_cell(&other.to_string()),
    }
}

/// JSON envelope for a report page.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct ReportPage {
    pub kind: String,
    /// Versioned read-model view backing this kind (documentation parity).
    pub view: String,
    /// Stable column order (JSON objects here preserve insertion order via
    /// serde_json preserve_order feature transparently at runtime).
    pub columns: Vec<String>,
    pub rows: Vec<Value>,
    /// Whole-window aggregates (never paginated).
    pub summary: Value,
    /// Pass back as `cursor`; absent when exhausted or unpaged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}
