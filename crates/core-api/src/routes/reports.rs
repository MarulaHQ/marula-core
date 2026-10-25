//! Operational reports endpoints (the private build tracker R1; ADR 0001).
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

    let rows = run_rows(
        &state.reporting,
        &compiled.sql,
        &compiled.binds,
        authed.user.org_id,
    )
    .await?;
    let decoded: Vec<Value> = rows
        .iter()
        .map(|r| row_json(r, &compiled.columns))
        .collect();

    // Cursor derives from the last row of THIS page under the kind's key
    // layout; absent when the page was short (exhaustion) or unpaged kind.
    let next_cursor = next_cursor_for_kind(
        kind,
        &compiled,
        decoded.last(),
        decoded.len() < limit as usize,
    );

    match want_csv {
        false => {
            let summary = run_summary(
                &state.reporting,
                &compiled.summary_sql,
                &compiled.summary_binds,
                authed.user.org_id,
            )
            .await?;
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
        true => Ok(csv_response(kind.slug(), &compiled.columns, decoded)),
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

/// Tags pool-side failures so logs name the failing phase during triage.
fn tag_pool_err(phase: &'static str) -> impl Fn(sqlx::Error) -> sqlx::Error {
    move |e| match &e {
        sqlx::Error::PoolTimedOut => {
            tracing::error!(phase, "reporting pool exhausted");
            e
        }
        _ => e,
    }
}

/// Executes a compiled statement binding every [`Bind`] in order, inside a
/// transaction explicitly bound to the caller's org (`app.org_id` GUC).
///
/// The generated SQL intentionally carries NO literal org filter — tenancy
/// flows through row-level security of the base tables under the
/// security-invoker views. Binding the GUC here makes that dependency
/// explicit and keeps correctness even for roles whose privileges later
/// drift. There is deliberately NO other execution mechanism — statements
/// go verbatim to Postgres.
pub(crate) async fn run_rows(
    pool: &PgPool,
    sql: &str,
    binds: &[Bind],
    org_id: Uuid,
) -> Result<Vec<sqlx::postgres::PgRow>, sqlx::Error> {
    let mut tx = pool.begin().await.map_err(tag_pool_err("begin"))?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(|e| sqlx::Error::Configuration(format!("set_org:{e}").into()))?;
    let mut q = sqlx::query(sql);
    for b in binds {
        q = match b {
            Bind::Uuid(u) => q.bind(*u),
            Bind::Text(s) => q.bind(s.as_str()),
            Bind::Date(d) => q.bind(*d),
            Bind::Timestamp(t) => q.bind(t),
            Bind::Int(n) => q.bind(*n),
            Bind::Bool(b) => q.bind(*b),
        };
    }
    let rows = q.fetch_all(&mut *tx).await?;
    // Read-only request: rollback is both correct and free.
    tx.rollback().await?;
    Ok(rows)
}

/// Same scoped pattern for the single-row summary statements.
async fn run_summary(
    pool: &PgPool,
    sql: &str,
    binds: &[Bind],
    org_id: Uuid,
) -> Result<Vec<sqlx::postgres::PgRow>, sqlx::Error> {
    run_rows(pool, sql, binds, org_id).await
}

/// Decodes a main-result row according to its declared output types. All
/// timestamps/dates ride their standard serde encodings (RFC3339 / YYYY-MM-DD).
pub(crate) fn row_json(row: &sqlx::postgres::PgRow, columns: &[OutputColumn]) -> Value {
    let mut map = Map::new();
    for (i, c) in columns.iter().enumerate() {
        let col_name = row.columns()[i].name();
        debug_assert_eq!(col_name, c.name.as_str(), "column layout drift");
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
        map.insert(c.name.clone(), v);
    }
    Value::Object(map)
}

/// Fallback decodes summary blocks dynamically off PG type OIDs/names so
/// per-kind summary column lists stay compiler-internal.
pub(crate) fn dynamic_row_json(row: &sqlx::postgres::PgRow) -> Value {
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
    page_shorter_than_limit: bool,
) -> Option<String> {
    if page_shorter_than_limit {
        return None;
    }
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
pub(crate) fn csv_response(
    kind_slug: &str,
    columns: &[OutputColumn],
    rows: Vec<Value>,
) -> Response {
    let header_vals: Vec<String> = columns.iter().map(|c| csv_cell(&c.name)).collect();

    let body_lines: Vec<String> = rows
        .iter()
        .map(|row| {
            let cells: Vec<String> = columns
                .iter()
                .map(|c| csv_cell_value(row.get(c.name.as_str())))
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
                format!("attachment; filename=\"{kind_slug}.csv\""),
            ),
        ],
        payload,
    )
        .into_response()
}

/// CSV cell: neutralise spreadsheet formulas, then quote per RFC 4180.
///
/// A report carries customer-supplied strings (client names, ticket titles,
/// group labels), so a value of `=cmd|...` would run as a formula in the
/// recipient's spreadsheet. Every cell whose first visible character is a
/// formula introducer gets a leading apostrophe, which readers that honour
/// the text marker (LibreOffice, Sheets) take as literal text and the rest
/// display as part of the cell. Quoting runs *after* the prefix so the
/// apostrophe sits inside the quotes: `=A1,B1` becomes `"'=A1,B1"`.
fn csv_cell(s: &str) -> String {
    let prefixed;
    let cell: &str = if csv_needs_text_prefix(s) {
        prefixed = format!("'{s}");
        &prefixed
    } else {
        s
    };
    if cell.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", cell.replace('"', "\"\""))
    } else {
        cell.to_string()
    }
}

/// Whether a spreadsheet would evaluate `s` as a formula.
///
/// The check runs on the trimmed value, so a leading tab or space cannot
/// hide the introducer (`\t=1+1` is the same attack as `=1+1`, with the
/// whitespace only there to slip past a naive first-byte check).
fn csv_needs_text_prefix(s: &str) -> bool {
    let candidate = s.trim();
    let Some(first) = candidate.chars().next() else {
        return false;
    };
    if !matches!(first, '=' | '+' | '-' | '@') {
        return false;
    }
    // Neither a metric nor a date is a formula: prefixing one would silently
    // turn a numeric or date column into text.
    !(is_plain_number(candidate) || is_iso_date(candidate))
}

/// `-1`, `-120.5`, `-1.5e-3` stay numbers. A leading `+` never qualifies —
/// spreadsheets read `+1` as a formula, not as the number 1.
fn is_plain_number(s: &str) -> bool {
    let body = s.strip_prefix('-').unwrap_or(s);
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    if let Some(exponent) = exponent {
        let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
    }
    !mantissa.is_empty()
        && mantissa.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && mantissa.bytes().filter(|b| *b == b'.').count() <= 1
        && mantissa.bytes().any(|b| b.is_ascii_digit())
}

/// `YYYY-MM-DD`, optionally with a time and offset (`2026-10-22T08:30:00Z`).
///
/// An ISO date can never begin with an introducer, so this half of the
/// exemption is structural today; stating it keeps a future, wider
/// introducer set from quietly mangling date columns.
fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    let date = b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit);
    if !date {
        return false;
    }
    let rest = &b[10..];
    if rest.is_empty() {
        return true;
    }
    if rest.len() < 9 || rest[0] != b'T' {
        return false;
    }
    // HH:MM:SS, then any fraction, then Z or ±HH:MM.
    let clock = &rest[1..];
    let clock_ok = clock.len() >= 8
        && clock[..2].iter().all(u8::is_ascii_digit)
        && clock[2] == b':'
        && clock[3..5].iter().all(u8::is_ascii_digit)
        && clock[5] == b':'
        && clock[6..8].iter().all(u8::is_ascii_digit);
    if !clock_ok {
        return false;
    }
    let tail = &clock[8..];
    let tail = match tail.first() {
        None => return true,
        Some(b'.') => {
            let digits = tail[1..].iter().take_while(|b| b.is_ascii_digit()).count();
            if digits == 0 {
                return false;
            }
            &tail[1 + digits..]
        }
        Some(_) => tail,
    };
    match tail.first() {
        None => true,
        Some(b'Z') => tail.len() == 1,
        Some(b'+') | Some(b'-') => {
            tail.len() == 6
                && tail[1..3].iter().all(u8::is_ascii_digit)
                && tail[3] == b':'
                && tail[4..6].iter().all(u8::is_ascii_digit)
        }
        Some(_) => false,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One row per rule in the escaping contract: the four formula
    /// introducers, the whitespace that tries to hide them, RFC 4180
    /// quoting, and the numbers and dates that must survive untouched.
    #[test]
    fn csv_cells_neutralise_formulas_but_keep_numbers_and_dates() {
        let cases: &[(&str, &str)] = &[
            // Formula introducers take a text prefix.
            ("=1+1", "'=1+1"),
            ("+1", "'+1"),
            ("-1+1", "'-1+1"),
            ("@SUM(A1)", "'@SUM(A1)"),
            ("=", "'="),
            (
                "=HYPERLINK(\"http://x\",\"y\")",
                "\"'=HYPERLINK(\"\"http://x\"\",\"\"y\"\")\"",
            ),
            // Leading whitespace is not a hiding place; the prefix lands
            // before it so the reader sees text first.
            ("\t=1+1", "'\t=1+1"),
            (" =1+1", "' =1+1"),
            (" \t-2+3", "' \t-2+3"),
            // RFC 4180 quoting still runs, after the prefix.
            ("=A1,B1", "\"'=A1,B1\""),
            ("report, \"final\"", "\"report, \"\"final\"\"\""),
            ("line\nbreak", "\"line\nbreak\""),
            // Numbers and dates stay numeric: they are data, not formulas.
            ("-1", "-1"),
            ("-120", "-120"),
            ("-120.5", "-120.5"),
            ("-1.5e-3", "-1.5e-3"),
            ("- 1", "'- 1"),
            ("-inf", "'-inf"),
            ("--1", "'--1"),
            ("2026-10-22", "2026-10-22"),
            ("2026-10-22T08:30:00Z", "2026-10-22T08:30:00Z"),
            (
                "2026-10-22T08:30:00.250+02:00",
                "2026-10-22T08:30:00.250+02:00",
            ),
            // Ordinary values are left alone.
            ("Acme", "Acme"),
            ("Acme, Inc", "\"Acme, Inc\""),
            ("", ""),
            ("  ", "  "),
        ];
        for (input, expected) in cases {
            assert_eq!(csv_cell(input), *expected, "input {input:?}");
        }
    }

    /// The JSON layer renders metrics as `Value::Number`; those never reach
    /// the string path, so a negative metric keeps its leading minus and a
    /// date keeps its shape while a label is neutralised.
    #[tokio::test]
    async fn csv_response_prefixes_string_cells_only() {
        let columns = vec![
            OutputColumn {
                name: "label".to_string(),
                ty: OutType::Text,
            },
            OutputColumn {
                name: "minutes".to_string(),
                ty: OutType::BigInt,
            },
            OutputColumn {
                name: "day".to_string(),
                ty: OutType::Date,
            },
        ];
        let rows = vec![json!({
            "label": "=1+1, and more",
            "minutes": -120,
            "day": "2026-10-22",
        })];
        let response = csv_response("time_by_client", &columns, rows);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert_eq!(
            text,
            "label,minutes,day\n\"'=1+1, and more\",-120,2026-10-22\n"
        );
    }
}
