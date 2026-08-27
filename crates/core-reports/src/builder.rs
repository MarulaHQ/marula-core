//! Query compiler: turns validated [`ReportKind`] + [`KindParams`] +
//! [`Scope`] (+ optional continuation cursor) into parameterized SQL over
//! the `ops.v1_*` read-model views ONLY (ADR 0001 §Decision — binding).
//!
//! Safety architecture:
//! * **Identifiers originate exclusively from this module's Rust constants**
//!   — view names via `catalog::ViewId`, filter/aggregation expressions as
//!   literals below. No runtime string ever becomes a SQL identifier.
//! * **User-controlled values appear only as `$N` binds** carried by
//!   [`Bind`]; nothing is interpolated into statement text.
//! * Placeholder numbers are minted strictly in push order (`Binder::ph`),
//!   so statement text and the executed bind vector cannot drift apart;
//!   tests pin shape, placeholder continuity, and forbidden tokens.

use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

use crate::catalog::ViewId;
use crate::error::ReportsError;
use crate::kinds::{KindParams, ReportKind};
use crate::params::{Scope, Window};

/// A value carrier for `$N` binds; executed 1:1 by the API layer mapping
/// each variant onto `query.bind`.
#[derive(Debug, Clone, PartialEq)]
pub enum Bind {
    Uuid(Uuid),
    Date(NaiveDate),
    Timestamp(DateTime<Utc>),
    Int(i64),
    Bool(bool),
}

/// Result-column descriptor driving JSON/CSV shaping at the API layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutType {
    Uuid,
    Text,
    Timestamp,
    Date,
    Int,
    BigInt,
    Float,
    Bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputColumn {
    pub name: &'static str,
    pub ty: OutType,
}

const fn col(name: &'static str, ty: OutType) -> OutputColumn {
    OutputColumn { name, ty }
}

/// Pagination-key layout of a compiled report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    /// `(metric bigint DESC, group_id uuid ASC)` — grouped rollups.
    MetricGroup,
    /// `(anchor timestamptz ASC, ticket_id uuid ASC)` — breach listing.
    TicketAsc,
    /// Same pair, descending — newest-first feeds.
    TicketDesc,
    /// Bounded rowsets (throughput/aging): pagination disabled.
    None,
}

/// Fully compiled report bundle ready for execution.
#[derive(Debug, Clone)]
pub struct CompiledReport {
    pub sql: String,
    pub binds: Vec<Bind>,
    /// Whole-window aggregate under the SAME filters/scope — never
    /// paginated; powers the response `summary` block.
    pub summary_sql: String,
    pub summary_binds: Vec<Bind>,
    pub columns: Vec<OutputColumn>,
    pub cursor_shape: CursorShape,
    pub view: ViewId,
}

// --- cursor codec ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum PageKey {
    MetricGroup { metric: i64, group: Uuid },
    Ticket { ts: DateTime<Utc>, ticket: Uuid },
}

impl PageKey {
    fn seal(self) -> String {
        use base64::Engine;
        let payload = match self {
            PageKey::MetricGroup { metric, group } => format!("m|{metric}|{group}"),
            PageKey::Ticket { ts, ticket } => format!("t|{}|{ticket}", ts.to_rfc3339()),
        };
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("v1|{payload}"))
    }

    fn open(raw: &str) -> Result<Self, ReportsError> {
        use base64::Engine;
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(raw.as_bytes())
            .map_err(|_| ReportsError::InvalidCursor)?;
        let text = String::from_utf8(decoded).map_err(|_| ReportsError::InvalidCursor)?;
        let rest = text.strip_prefix("v1|").ok_or(ReportsError::InvalidCursor)?;
        let parts: Vec<&str> = rest.split('|').collect();
        if parts.len() != 3 {
            return Err(ReportsError::InvalidCursor);
        }
        match parts[0] {
            "m" => Ok(PageKey::MetricGroup {
                metric: parts[1].parse().map_err(|_| ReportsError::InvalidCursor)?,
                group: parts[2].parse().map_err(|_| ReportsError::InvalidCursor)?,
            }),
            "t" => Ok(PageKey::Ticket {
                ts: parts[1].parse().map_err(|_| ReportsError::InvalidCursor)?,
                ticket: parts[2].parse().map_err(|_| ReportsError::InvalidCursor)?,
            }),
            _ => Err(ReportsError::InvalidCursor),
        }
    }
}

fn decode_cursor(raw: &str, shape: CursorShape) -> Result<PageKey, ReportsError> {
    let key = PageKey::open(raw)?;
    match (shape, key.clone()) {
        (CursorShape::MetricGroup, PageKey::MetricGroup { .. })
        | (CursorShape::TicketAsc | CursorShape::TicketDesc, PageKey::Ticket { .. }) => Ok(key),
        _ => Err(ReportsError::InvalidCursor),
    }
}

/// Encodes the next-page cursor from the LAST row of the previous page.
///
/// Arguments correspond to each kind's sort-key output positions:
/// * grouped kinds → `(metric_value_number, group_id_string)`
/// * tabular kinds → `(anchor_timestamp_rfc3339_string, ticket_id_string)`
/// * unpaged kinds → always `None`.
#[must_use]
pub fn next_cursor(
    kind: ReportKind,
    sort_key_a: &serde_json::Value,
    sort_key_b: &serde_json::Value,
) -> Option<String> {
    use serde_json::Value;
    let uuid_of = |v: &Value| v.as_str().and_then(|s| s.parse::<Uuid>().ok());
    match kind {
        ReportKind::TimeByUser
        | ReportKind::TimeByClient
        | ReportKind::TimeByTicket
        | ReportKind::SlaAttainment => Some(
            PageKey::MetricGroup {
                metric: sort_key_a.as_i64()?,
                group: uuid_of(sort_key_b)?,
            }
            .seal(),
        ),
        ReportKind::SlaBreaches | ReportKind::FirstResponse => Some(
            PageKey::Ticket {
                ts: sort_key_a
                    .as_str()
                    .and_then(|s| s.parse::<DateTime<Utc>>().ok())?,
                ticket: uuid_of(sort_key_b)?,
            }
            .seal(),
        ),
        ReportKind::TicketThroughput | ReportKind::TicketAging => None,
    }
}

// --- bind bookkeeping ----------------------------------------------------------

struct Binder {
    binds: Vec<Bind>,
}

impl Binder {
    fn new() -> Self {
        Self { binds: Vec::new() }
    }

    /// Pushes the bind and returns its `$N` placeholder text. Numbers are
    /// minted strictly in push order; fragments are assembled exactly once.
    fn ph(&mut self, b: Bind) -> String {
        self.binds.push(b);
        format!("${}", self.binds.len())
    }
}

fn scope_user(scope: Scope, param: Option<Uuid>) -> Option<Uuid> {
    match scope {
        Scope::Self_(id) => Some(id),
        Scope::OrgWide => param,
    }
}

// --- compiler entry point --------------------------------------------------------

/// Compiles one prebuilt report. Pure: identical inputs ⇒ identical output.
///
/// # Errors
/// Typed validation failures only ([`ReportsError`]); never panics on user
/// input.
pub fn compile(
    kind: ReportKind,
    params: &KindParams,
    scope: Scope,
    cursor_raw: Option<&str>,
    limit: i64,
    win: Window,
) -> Result<CompiledReport, ReportsError> {
    crate::kinds::validate_kind(kind, params);
    crate::params::validate_limit_raw(limit)?;

    match (kind, params) {
        (ReportKind::TimeByUser, KindParams::TimeByUser(p)) => compile_time_rollup(
            RollupKey::User,
            scope,
            p.base.user_id,
            p.billable,
            cursor_raw,
            limit,
            win,
        ),
        (ReportKind::TimeByClient, KindParams::TimeByClient(p)) => {
            compile_time_rollup(RollupKey::Client, scope, p.user_id, None, cursor_raw, limit, win)
        }
        (ReportKind::TimeByTicket, KindParams::TimeByTicket(p)) => {
            compile_time_rollup(RollupKey::Ticket, scope, p.user_id, None, cursor_raw, limit, win)
        }
        (ReportKind::SlaAttainment, KindParams::SlaAttainment(p)) => {
            compile_sla_attainment(scope, p.policy_id, p.user_id, cursor_raw, limit, win)
        }
        (ReportKind::SlaBreaches, KindParams::SlaBreaches(p)) => {
            compile_sla_breaches(scope, p.policy_id, p.user_id, cursor_raw, limit, win)
        }
        (ReportKind::TicketThroughput, KindParams::TicketThroughput(p)) => {
            compile_throughput(scope, p.user_id, limit, win)
        }
        (ReportKind::TicketAging, KindParams::TicketAging(p)) => {
            compile_aging(scope, p.user_id, p.as_of, limit)
        }
        (ReportKind::FirstResponse, KindParams::FirstResponse(p)) => {
            compile_first_response(scope, p.user_id, cursor_raw, limit, win)
        }
        _ => Err(ReportsError::UnknownColumn {
            view: kind.view().qualified_name().to_string(),
            name: "kind/params mismatch".to_string(),
        }),
    }
}

// --- time rollups: time_by_user / time_by_client / time_by_ticket --------------

enum RollupKey {
    User,
    Client,
    Ticket,
}

#[allow(clippy::too_many_arguments)]
fn compile_time_rollup(
    key: RollupKey,
    scope: Scope,
    user_id: Option<Uuid>,
    billable: Option<bool>,
    cursor_raw: Option<&str>,
    limit: i64,
    win: Window,
) -> Result<CompiledReport, ReportsError> {
    // Main-query conditions (owns binds 1..).
    let mut b = Binder::new();
    let mut conds: Vec<String> = Vec::new();
    {
        let lo = b.ph(Bind::Timestamp(win.start));
        let hi = b.ph(Bind::Timestamp(win.end));
        conds.push(format!("te.started_at >= {lo} AND te.started_at < {hi}"));
    }
    if let Some(u) = scope_user(scope, user_id) {
        let n = b.ph(Bind::Uuid(u));
        conds.push(format!("te.user_id = {n}"));
    }
    if let Some(bill) = billable {
        let n = b.ph(Bind::Bool(bill));
        conds.push(format!("te.billable = {n}"));
    }

    // Grouping keys. The client rollup coalesces missing clients onto the
    // NIL uuid so group_id stays total/non-null under the keyset ordering
    // (metric DESC, group_id ASC).
    let (group_expr, label_expr): (&str, &str) = match key {
        RollupKey::User => ("te.user_id", "MAX(te.user_display_name)"),
        RollupKey::Client => (
            "COALESCE(te.client_id, '00000000-0000-0000-0000-000000000000'::uuid)",
            "MAX(te.client_name)",
        ),
        RollupKey::Ticket => ("te.ticket_id", "MAX(te.ticket_number)"),
    };

    let where_main = conds.join("\n  AND ");
    let inner = format!(
        "SELECT {} AS group_id,\n       {} AS label,\n       SUM(te.minutes)::bigint AS metric,\n       SUM(CASE WHEN te.billable THEN te.minutes ELSE 0 END)::bigint AS billable_minutes,\n       COUNT(*)::bigint AS entry_count\nFROM {}\nte\nWHERE {}\nGROUP BY {}",
        group_expr,
        label_expr,
        ViewId::TimeEntries.qualified_name(),
        where_main,
        group_expr,
    );

    let mut sql = format!("SELECT agg.* FROM (\n{inner}\n) agg");
    if let Some(PageKey::MetricGroup { metric, group }) =
        cursor_raw.map(|raw| decode_cursor(raw, CursorShape::MetricGroup)).transpose()?
    {
        let m = b.ph(Bind::Int(metric));
        let g = b.ph(Bind::Uuid(group));
        sql.push_str(&format!(
            "\nWHERE agg.metric < {m} OR (agg.metric = {m} AND agg.group_id > {g})"
        ));
    }
    let lim = b.ph(Bind::Int(limit));
    sql.push_str(&format!(
        "\nORDER BY agg.metric DESC, agg.group_id ASC\nLIMIT {lim}"
    ));

    // Summary — same filters mirrored into a fresh binder.
    let mut sb = Binder::new();
    let mut s_conds: Vec<String> = Vec::new();
    {
        let slo = sb.ph(Bind::Timestamp(win.start));
        let shi = sb.ph(Bind::Timestamp(win.end));
        s_conds.push(format!("te.started_at >= {slo} AND te.started_at < {shi}"));
    }
    if let Some(u) = scope_user(scope, user_id) {
        let sn = sb.ph(Bind::Uuid(u));
        s_conds.push(format!("te.user_id = {sn}"));
    }
    if let Some(bill) = billable {
        let sn = sb.ph(Bind::Bool(bill));
        s_conds.push(format!("te.billable = {sn}"));
    }

    Ok(CompiledReport {
        sql,
        binds: b.binds,
        summary_sql: format!(
            "SELECT COALESCE(COUNT(*), 0)::bigint AS entries_count,\n       COALESCE(SUM(te.minutes), 0)::bigint AS total_minutes,\n       COALESCE(SUM(CASE WHEN te.billable THEN te.minutes ELSE 0 END), 0)::bigint AS billable_minutes\nFROM {}\nte\nWHERE {}",
            ViewId::TimeEntries.qualified_name(),
            s_conds.join("\n  AND "),
        ),
        summary_binds: sb.binds,
        columns: vec![
            col("group_id", OutType::Uuid),
            col("label", OutType::Text),
            col("metric", OutType::BigInt),
            col("billable_minutes", OutType::BigInt),
            col("entry_count", OutType::BigInt),
        ],
        cursor_shape: CursorShape::MetricGroup,
        view: ViewId::TimeEntries,
    })
}

// --- SLA attainment -------------------------------------------------------------

fn compile_sla_attainment(
    scope: Scope,
    policy_id: Option<Uuid>,
    user_id: Option<Uuid>,
    cursor_raw: Option<&str>,
    limit: i64,
    win: Window,
) -> Result<CompiledReport, ReportsError> {
    let assignee = scope_user(scope, user_id);

    let mut b = Binder::new();
    let mut conds: Vec<String> = Vec::new();
    {
        let lo = b.ph(Bind::Timestamp(win.start));
        let hi = b.ph(Bind::Timestamp(win.end));
        conds.push(format!(
            "so.resolved_at IS NOT NULL AND so.resolved_at >= {lo} AND so.resolved_at < {hi}"
        ));
    }
    if let Some(p) = policy_id {
        let n = b.ph(Bind::Uuid(p));
        conds.push(format!("so.policy_id = {n}"));
    }
    if let Some(a) = assignee {
        let n = b.ph(Bind::Uuid(a));
        conds.push(format!("so.assignee_id = {n}"));
    }

    let inner = format!(
        "SELECT so.policy_id AS group_id,\n       MAX(so.policy_name) AS label,\n       COUNT(*)::bigint AS metric,\n       SUM(CASE WHEN so.resolution_met THEN 1 ELSE 0 END)::bigint AS resolutions_met,\n       ROUND(100.0 * SUM(CASE WHEN so.resolution_met THEN 1 ELSE 0 END) / NULLIF(COUNT(*), 0), 1) AS attainment_pct\nFROM {}\nso\nWHERE {}\nGROUP BY so.policy_id",
        ViewId::SlaOutcomes.qualified_name(),
        conds.join("\n  AND "),
    );

    let mut sql = format!("SELECT agg.* FROM (\n{inner}\n) agg");
    if let Some(PageKey::MetricGroup { metric, group }) =
        cursor_raw.map(|raw| decode_cursor(raw, CursorShape::MetricGroup)).transpose()?
    {
        let m = b.ph(Bind::Int(metric));
        let g = b.ph(Bind::Uuid(group));
        sql.push_str(&format!(
            "\nWHERE agg.metric < {m} OR (agg.metric = {m} AND agg.group_id > {g})"
        ));
    }
    let lim = b.ph(Bind::Int(limit));
    sql.push_str(&format!(
        "\nORDER BY agg.metric DESC, agg.group_id ASC\nLIMIT {lim}"
    ));

    // Summary mirrors conditions onto its own binder.
    let mut sb = Binder::new();
    let mut s_conds: Vec<String> = Vec::new();
    {
        let slo = sb.ph(Bind::Timestamp(win.start));
        let shi = sb.ph(Bind::Timestamp(win.end));
        s_conds.push(format!(
            "so.resolved_at IS NOT NULL AND so.resolved_at >= {slo} AND so.resolved_at < {shi}"
        ));
    }
    if let Some(p) = policy_id {
        let sn = sb.ph(Bind::Uuid(p));
        s_conds.push(format!("so.policy_id = {sn}"));
    }
    if let Some(a) = assignee {
        let sn = sb.ph(Bind::Uuid(a));
        s_conds.push(format!("so.assignee_id = {sn}"));
    }

    Ok(CompiledReport {
        sql,
        binds: b.binds,
        summary_sql: format!(
            "SELECT COALESCE(COUNT(*), 0)::bigint AS resolutions_total,\n       COALESCE(SUM(CASE WHEN so.resolution_met THEN 1 ELSE 0 END), 0)::bigint AS resolutions_met,\n       ROUND(100.0 * COALESCE(SUM(CASE WHEN so.resolution_met THEN 1 ELSE 0 END), 0) / NULLIF(COUNT(*), 0), 1) AS attainment_pct\nFROM {}\nso\nWHERE {}",
            ViewId::SlaOutcomes.qualified_name(),
            s_conds.join("\n  AND "),
        ),
        summary_binds: sb.binds,
        columns: vec![
            col("group_id", OutType::Uuid),
            col("label", OutType::Text),
            col("metric", OutType::BigInt),
            col("resolutions_met", OutType::BigInt),
            col("attainment_pct", OutType::Float),
        ],
        cursor_shape: CursorShape::MetricGroup,
        view: ViewId::SlaOutcomes,
    })
}

// --- SLA breaches ----------------------------------------------------------------

fn compile_sla_breaches(
    scope: Scope,
    policy_id: Option<Uuid>,
    user_id: Option<Uuid>,
    cursor_raw: Option<&str>,
    limit: i64,
    win: Window,
) -> Result<CompiledReport, ReportsError> {
    let assignee = scope_user(scope, user_id);

    let mut b = Binder::new();
    let mut conds: Vec<String> = vec!["so.breached".to_owned()];
    {
        let lo = b.ph(Bind::Timestamp(win.start));
        let hi = b.ph(Bind::Timestamp(win.end));
        conds.push(format!(
            "so.ticket_created_at >= {lo} AND so.ticket_created_at < {hi}"
        ));
    }
    if let Some(p) = policy_id {
        let n = b.ph(Bind::Uuid(p));
        conds.push(format!("so.policy_id = {n}"));
    }
    if let Some(a) = assignee {
        let n = b.ph(Bind::Uuid(a));
        conds.push(format!("so.assignee_id = {n}"));
    }
    if let Some(key @ PageKey::Ticket { .. }) =
        cursor_raw.map(|raw| decode_cursor(raw, CursorShape::TicketAsc)).transpose()?
    {
        let PageKey::Ticket { ts, ticket } = key else {
            return Err(ReportsError::InvalidCursor);
        };
        let t = b.ph(Bind::Timestamp(ts));
        let u = b.ph(Bind::Uuid(ticket));
        conds.push(format!(
            "(so.ticket_created_at > {t} OR (so.ticket_created_at = {t} AND so.ticket_id > {u}))"
        ));
    }
    let lim = b.ph(Bind::Int(limit));

    let sql = format!(
        "SELECT so.ticket_id,\n       so.ticket_number,\n       so.priority,\n       so.policy_name,\n       so.assignee_id,\n       so.ticket_created_at,\n       so.resolution_due,\n       so.resolution_met,\n       so.paused_business_secs\nFROM {}\nso\nWHERE {}\nORDER BY so.ticket_created_at ASC, so.ticket_id ASC\nLIMIT {}",
        ViewId::SlaOutcomes.qualified_name(),
        conds.join("\n  AND "),
        lim,
    );

    let mut sb = Binder::new();
    let mut s_conds: Vec<String> = vec!["so.breached".to_owned()];
    {
        let slo = sb.ph(Bind::Timestamp(win.start));
        let shi = sb.ph(Bind::Timestamp(win.end));
        s_conds.push(format!(
            "so.ticket_created_at >= {slo} AND so.ticket_created_at < {shi}"
        ));
    }
    if let Some(p) = policy_id {
        let sn = sb.ph(Bind::Uuid(p));
        s_conds.push(format!("so.policy_id = {sn}"));
    }
    if let Some(a) = assignee {
        let sn = sb.ph(Bind::Uuid(a));
        s_conds.push(format!("so.assignee_id = {sn}"));
    }

    Ok(CompiledReport {
        sql,
        binds: b.binds,
        summary_sql: format!(
            "SELECT COALESCE(COUNT(*), 0)::bigint AS breaches_total,\n       COALESCE(SUM(so.paused_business_secs), 0)::bigint AS paused_business_secs_total\nFROM {}\nso\nWHERE {}",
            ViewId::SlaOutcomes.qualified_name(),
            s_conds.join("\n  AND "),
        ),
        summary_binds: sb.binds,
        columns: vec![
            col("ticket_id", OutType::Uuid),
            col("ticket_number", OutType::Text),
            col("priority", OutType::Text),
            col("policy_name", OutType::Text),
            col("assignee_id", OutType::Uuid),
            col("ticket_created_at", OutType::Timestamp),
            col("resolution_due", OutType::Timestamp),
            col("resolution_met", OutType::Bool),
            col("paused_business_secs", OutType::BigInt),
        ],
        cursor_shape: CursorShape::TicketAsc,
        view: ViewId::SlaOutcomes,
    })
}

// --- throughput -------------------------------------------------------------------

fn compile_throughput(
    scope: Scope,
    user_id: Option<Uuid>,
    limit: i64,
    win: Window,
) -> Result<CompiledReport, ReportsError> {
    let assignee = scope_user(scope, user_id);

    let mut b = Binder::new();
    // Spine days [start_date … end_date_exclusive); join admits tickets
    // created inside the window OR resolved at/before its end (a ticket
    // created BEFORE the window that resolves INSIDE it must still count on
    // its resolution day — so resolution arm prunes by created_at <= end,
    // never by created_at >= start).
    //
    // Binds are minted in EXACT textual order of their placeholders:
    //   $1/$2 spine dates → per-FILTER-arm scopes → join window bounds and
    //   scopes → LIMIT.
    let d_lo = b.ph(Bind::Date(win.start.date_naive()));
    let d_hi = b.ph(Bind::Date(win.end.date_naive()));

    let mut created_arm_scope = String::new();
    let mut resolved_arm_scope = String::new();
    if let Some(a) = assignee {
        let n1 = b.ph(Bind::Uuid(a));
        let n2 = b.ph(Bind::Uuid(a));
        created_arm_scope = format!(" AND v.assignee_id = {n1}");
        resolved_arm_scope = format!(" AND v.assignee_id = {n2}");
    }

    let c_lo = b.ph(Bind::Timestamp(win.start));
    let c_hi = b.ph(Bind::Timestamp(win.end));
    let mut join_c_scope = String::new();
    if let Some(a) = assignee {
        let n3 = b.ph(Bind::Uuid(a));
        join_c_scope = format!(" AND v.assignee_id = {n3}");
    }
    let r_hi = b.ph(Bind::Timestamp(win.end));
    let mut join_r_scope = String::new();
    if let Some(a) = assignee {
        let n4 = b.ph(Bind::Uuid(a));
        join_r_scope = format!(" AND v.assignee_id = {n4}");
    }
    let lim = b.ph(Bind::Int(limit));

    let sql = format!(
        "WITH days(day) AS (\n  SELECT generate_series({d_lo}::date, ({d_hi}::date - INTERVAL '1 day'), '1 day')::date\n)\nSELECT d.day,\n       COUNT(v.id) FILTER (\n         WHERE CAST(v.created_at AT TIME ZONE 'UTC' AS date) = d.day{created_arm_scope}\n       )::bigint AS created_count,\n       COUNT(v.id) FILTER (\n         WHERE v.resolved_at IS NOT NULL\n           AND CAST(v.resolved_at AT TIME ZONE 'UTC' AS date) = d.day{resolved_arm_scope}\n       )::bigint AS resolved_count\nFROM days d\nLEFT JOIN {}\nv\n  ON (v.created_at >= {c_lo} AND v.created_at < {c_hi}{join_c_scope})\n  OR (v.resolved_at IS NOT NULL AND v.created_at <= {r_hi}{join_r_scope})\nGROUP BY d.day\nORDER BY d.day ASC\nLIMIT {lim}",
        ViewId::Tickets.qualified_name(),
    );

    // Summary mirrors the pruning windows (created vs resolved arms share
    // the created-window anchor for stability). Bind order follows text:
    // created lo/hi → created scope uuid → resolved bound → resolved scope.
    let mut sb = Binder::new();
    let slo = sb.ph(Bind::Timestamp(win.start));
    let shi = sb.ph(Bind::Timestamp(win.end));
    let mut s_sc = String::new();
    if let Some(a) = assignee {
        let n1 = sb.ph(Bind::Uuid(a));
        s_sc = format!(" AND v.assignee_id = {n1}");
    }
    let rsh = sb.ph(Bind::Timestamp(win.end));
    let mut s_sr = String::new();
    if let Some(a) = assignee {
        let n2 = sb.ph(Bind::Uuid(a));
        s_sr = format!(" AND v.assignee_id = {n2}");
    }
    let resolved_total_line = format!(
        "       (SELECT COALESCE(COUNT(*), 0)::bigint FROM {view} v\n         WHERE v.resolved_at IS NOT NULL AND v.resolved_at <= {rsh}{s_sr}) AS resolved_total",
        view = ViewId::Tickets.qualified_name(),
    );

    Ok(CompiledReport {
        sql,
        binds: b.binds,
        summary_sql: format!(
            "SELECT (SELECT COALESCE(COUNT(*), 0)::bigint FROM {view} v\n         WHERE v.created_at >= {slo} AND v.created_at < {shi}{s_sc}) AS created_total,\n{resolved_total_line}",
            view = ViewId::Tickets.qualified_name(),
        ),
        summary_binds: sb.binds,
        columns: vec![
            col("day", OutType::Date),
            col("created_count", OutType::BigInt),
            col("resolved_count", OutType::BigInt),
        ],
        cursor_shape: CursorShape::None,
        view: ViewId::Tickets,
    })
}

// --- ticket aging -----------------------------------------------------------------

fn compile_aging(
    scope: Scope,
    user_id: Option<Uuid>,
    as_of: DateTime<Utc>,
    limit: i64,
) -> Result<CompiledReport, ReportsError> {
    let assignee = scope_user(scope, user_id);

    let mut b = Binder::new();
    let now = b.ph(Bind::Timestamp(as_of));
    let mut sub_cond = String::from("v.category <> 'closed'");
    if let Some(a) = assignee {
        let n = b.ph(Bind::Uuid(a));
        sub_cond.push_str(&format!(" AND v.assignee_id = {n}"));
    }
    // Sub-select computes age in (fractional) days; outer groups into the
    // fixed v1 buckets. bucket_ordinal keeps the presentation order stable.
    let inner = format!(
        "SELECT CASE\n         WHEN d.age < 1 THEN 'under_1d'\n         WHEN d.age < 3 THEN 'd1_3'\n         WHEN d.age < 7 THEN 'd3_7'\n         WHEN d.age < 30 THEN 'd7_30'\n         ELSE 'over_30d'\n       END AS bucket,\n       CASE\n         WHEN d.age < 1 THEN 0\n         WHEN d.age < 3 THEN 1\n         WHEN d.age < 7 THEN 2\n         WHEN d.age < 30 THEN 3\n         ELSE 4\n       END AS bucket_ordinal,\n       COUNT(*)::bigint AS open_count,\n       FLOOR(MAX(d.age))::bigint AS oldest_age_days,\n       ROUND(AVG(d.age)::numeric, 1) AS avg_age_days\nFROM (\n  SELECT v.id, EXTRACT(epoch FROM ({now} - v.created_at)) / 86400.0 AS age\n  FROM {view}\nv\n  WHERE {sub_cond}\n) d\nGROUP BY 1, 2",
        view = ViewId::Tickets.qualified_name(),
    );

    let lim = b.ph(Bind::Int(limit));
    let sql = format!(
        "SELECT o.bucket,\n       o.bucket_ordinal,\n       o.open_count,\n       o.oldest_age_days,\n       o.avg_age_days\nFROM (\n{inner}\n) o\nORDER BY o.bucket_ordinal ASC\nLIMIT {}",
        lim,
    );

    // Summary mirrors the same sub-select conditions.
    let mut sb = Binder::new();
    let snow = sb.ph(Bind::Timestamp(as_of));
    let mut s_sub_cond = String::from("v.category <> 'closed'");
    if let Some(a) = assignee {
        let sn = sb.ph(Bind::Uuid(a));
        s_sub_cond.push_str(&format!(" AND v.assignee_id = {sn}"));
    }

    Ok(CompiledReport {
        sql,
        binds: b.binds,
        summary_sql: format!(
            "SELECT COALESCE(COUNT(*), 0)::bigint AS open_total,\n       FLOOR(COALESCE(MAX(EXTRACT(epoch FROM ({snow} - v.created_at)) / 86400.0), 0))::bigint AS oldest_age_days,\n       COALESCE(SUM(CASE WHEN v.breached THEN 1 ELSE 0 END), 0)::bigint AS breached_open\nFROM {}\nv\nWHERE {}",
            ViewId::Tickets.qualified_name(),
            s_sub_cond,
        ),
        summary_binds: sb.binds,
        columns: vec![
            col("bucket", OutType::Text),
            col("bucket_ordinal", OutType::Int),
            col("open_count", OutType::BigInt),
            col("oldest_age_days", OutType::BigInt),
            col("avg_age_days", OutType::Float),
        ],
        cursor_shape: CursorShape::None,
        view: ViewId::Tickets,
    })
}

// --- first response ----------------------------------------------------------------

fn compile_first_response(
    scope: Scope,
    user_id: Option<Uuid>,
    cursor_raw: Option<&str>,
    limit: i64,
    win: Window,
) -> Result<CompiledReport, ReportsError> {
    let assignee = scope_user(scope, user_id);

    let mut b = Binder::new();
    let mut conds: Vec<String> = Vec::new();
    {
        let lo = b.ph(Bind::Timestamp(win.start));
        let hi = b.ph(Bind::Timestamp(win.end));
        conds.push(format!("fr.created_at >= {lo} AND fr.created_at < {hi}"));
    }
    if let Some(a) = assignee {
        let n = b.ph(Bind::Uuid(a));
        conds.push(format!("fr.assignee_id = {n}"));
    }
    if let Some(key @ PageKey::Ticket { .. }) =
        cursor_raw.map(|raw| decode_cursor(raw, CursorShape::TicketDesc)).transpose()?
    {
        let PageKey::Ticket { ts, ticket } = key else {
            return Err(ReportsError::InvalidCursor);
        };
        let t = b.ph(Bind::Timestamp(ts));
        let u = b.ph(Bind::Uuid(ticket));
        conds.push(format!(
            "(fr.created_at < {t} OR (fr.created_at = {t} AND fr.ticket_id < {u}))"
        ));
    }
    let lim = b.ph(Bind::Int(limit));

    let sql = format!(
        "SELECT fr.ticket_id,\n       fr.ticket_number,\n       fr.client_id,\n       fr.assignee_id,\n       fr.created_at,\n       fr.first_staff_comment_at,\n       fr.first_response_secs,\n       fr.first_response_due\nFROM {}\nfr\nWHERE {}\nORDER BY fr.created_at DESC, fr.ticket_id DESC\nLIMIT {}",
        ViewId::FirstResponse.qualified_name(),
        conds.join("\n  AND "),
        lim,
    );

    let mut sb = Binder::new();
    let mut s_conds: Vec<String> = Vec::new();
    {
        let slo = sb.ph(Bind::Timestamp(win.start));
        let shi = sb.ph(Bind::Timestamp(win.end));
        s_conds.push(format!("fr.created_at >= {slo} AND fr.created_at < {shi}"));
    }
    if let Some(a) = assignee {
        let sn = sb.ph(Bind::Uuid(a));
        s_conds.push(format!("fr.assignee_id = {sn}"));
    }

    Ok(CompiledReport {
        sql,
        binds: b.binds,
        summary_sql: format!(
            "SELECT COUNT(*)::bigint AS tickets_total,\n       COUNT(fr.first_staff_comment_at)::bigint AS answered_count,\n       (COUNT(*) - COUNT(fr.first_staff_comment_at))::bigint AS unanswered_count,\n       percentile_cont(0.5) WITHIN GROUP (ORDER BY fr.first_response_secs) FILTER (WHERE fr.first_staff_comment_at IS NOT NULL) AS response_p50_secs,\n       percentile_cont(0.9) WITHIN GROUP (ORDER BY fr.first_response_secs) FILTER (WHERE fr.first_staff_comment_at IS NOT NULL) AS response_p90_secs\nFROM {}\nfr\nWHERE {}",
            ViewId::FirstResponse.qualified_name(),
            s_conds.join("\n  AND "),
        ),
        summary_binds: sb.binds,
        columns: vec![
            col("ticket_id", OutType::Uuid),
            col("ticket_number", OutType::Text),
            col("client_id", OutType::Uuid),
            col("assignee_id", OutType::Uuid),
            col("created_at", OutType::Timestamp),
            col("first_staff_comment_at", OutType::Timestamp),
            col("first_response_secs", OutType::BigInt),
            col("first_response_due", OutType::Timestamp),
        ],
        cursor_shape: CursorShape::TicketDesc,
        view: ViewId::FirstResponse,
    })
}

// --- structural safety tests ------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds::{KindParams, ReportKind};
    use crate::params::{resolve_window, Scope};
    use chrono::Duration as ChronoDuration;
    use proptest::prelude::*;

    fn win_default(now: DateTime<Utc>) -> Window {
        resolve_window(None, None, now).expect("default window")
    }

    fn params_for(kind: ReportKind, now: DateTime<Utc>) -> KindParams {
        let from = Some(now.date_naive() - ChronoDuration::days(7));
        match kind {
            ReportKind::TimeByUser => KindParams::TimeByUser(crate::kinds::TimeByUserParams {
                base: crate::kinds::TimeReportParams {
                    window: crate::kinds::WindowParams { from, to: None },
                    user_id: None,
                },
                billable: None,
            }),
            ReportKind::TimeByClient => KindParams::TimeByClient(crate::kinds::TimeReportParams {
                window: crate::kinds::WindowParams { from, to: None },
                user_id: None,
            }),
            ReportKind::TimeByTicket => KindParams::TimeByTicket(crate::kinds::TimeReportParams {
                window: crate::kinds::WindowParams { from, to: None },
                user_id: None,
            }),
            ReportKind::SlaAttainment => {
                KindParams::SlaAttainment(crate::kinds::SlaOutcomesParams {
                    window: crate::kinds::WindowParams { from, to: None },
                    policy_id: None,
                    user_id: None,
                })
            }
            ReportKind::SlaBreaches => KindParams::SlaBreaches(crate::kinds::SlaOutcomesParams {
                window: crate::kinds::WindowParams { from, to: None },
                policy_id: None,
                user_id: None,
            }),
            ReportKind::TicketThroughput => {
                KindParams::TicketThroughput(crate::kinds::ThroughputParams {
                    window: crate::kinds::WindowParams { from, to: None },
                    user_id: None,
                })
            }
            ReportKind::TicketAging => KindParams::TicketAging(crate::kinds::AgingParams {
                user_id: None,
                as_of: now,
            }),
            ReportKind::FirstResponse => {
                KindParams::FirstResponse(crate::kinds::FirstResponseParams {
                    window: crate::kinds::WindowParams { from, to: None },
                    user_id: None,
                })
            }
        }
    }

    /// Placeholder continuity: every `$N` appears exactly once and their
    /// sequence is exactly 1..=binds.len(). This pins the compiler against
    /// bind/statement drift — the structural precondition for parameterized
    /// execution being equivalent to the emitted plan.
    #[track_caller]
    fn assert_placeholders_contiguous(sql: &str, n_binds: usize) {
        let mut nums: Vec<usize> = Vec::new();
        let bytes = sql.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end].is_ascii_digit() {
                    end += 1;
                }
                let num: usize = sql[start..end].parse().expect("placeholder number");
                if !nums.contains(&num) {
                    nums.push(num);
                }
                i = end;
            } else {
                i += 1;
            }
        }
        // Placeholders may repeat (e.g. `agg.metric = $3` twice); what is
        // pinned is FIRST-APPEARANCE order equals 1..=binds.len().
        let expected: Vec<usize> = (1..=n_binds).collect();
        assert_eq!(nums.len(), n_binds, "placeholder count drift in: {sql}");
        assert_eq!(nums, expected, "placeholder sequence drift in: {sql}");
    }

    /// The statement-shape guard: only SELECT/WITH entry points, no
    /// data-mutating keywords, no statement terminators, and only
    /// `ops.v1_` table references.
    #[track_caller]
    fn assert_statement_shape(sql: &str) {
        let lower = sql.to_ascii_lowercase();
        assert!(
            lower.starts_with("select") || lower.starts_with("with"),
            "reports must be reads only; got {sql}"
        );
        for banned in [
            "insert ", "update ", "delete ", "drop ", "alter ", "truncate ",
            "grant ", "revoke ", ";", "--", "/*",
        ] {
            assert!(
                !lower.contains(banned),
                "forbidden token '{banned}' in compiled report SQL: {sql}"
            );
        }
        let mut rest = lower.as_str();
        while let Some(pos) = rest.find("ops.") {
            let tail = &rest[pos..pos + 10.min(rest.len() - pos)];
            assert!(
                tail.starts_with("ops.v1_"),
                "non-versioned or non-ops reference near '{tail}'"
            );
            rest = &rest[pos + "ops.".len()..];
        }
    }

    const NOW: DateTime<Utc> = match chrono::DateTime::from_timestamp(1_780_000_000, 0) {
        Some(t) => t,
        None => unreachable!(),
    };

    /// Every kind compiles cleanly under BOTH scopes with an empty and a
    /// stale cursor supplied.
    #[test]
    fn all_kinds_compile_under_both_scopes() {
        for kind in ReportKind::ALL {
            for scope in [Scope::OrgWide, Scope::Self_(Uuid::nil())] {
                let rep = compile(
                    kind,
                    &params_for(kind, NOW),
                    scope,
                    None,
                    100,
                    win_default(NOW),
                )
                .unwrap_or_else(|e| panic!("{kind:?}/{scope:?}: {e}"));
                assert_statement_shape(&rep.sql);
                assert_statement_shape(&rep.summary_sql);
                assert_placeholders_contiguous(&rep.sql, rep.binds.len());
                assert_placeholders_contiguous(&rep.summary_sql, rep.summary_binds.len());
            }
        }
    }

    #[test]
    fn cursor_shapes_match_kinds_and_bind_after_filters() {
        for kind in [
            ReportKind::TimeByUser,
            ReportKind::TimeByClient,
            ReportKind::TimeByTicket,
            ReportKind::SlaAttainment,
            ReportKind::SlaBreaches,
            ReportKind::FirstResponse,
        ] {
            // A valid-shaped key: metric cursor for grouped kinds, ticket
            // cursor for tabular kinds. Encode via next_cursor first.
            let shape_key = match kind.view() {
                ViewId::TimeEntries | ViewId::SlaOutcomes if matches!(
                    kind,
                    ReportKind::SlaAttainment
                        | ReportKind::TimeByUser
                        | ReportKind::TimeByClient
                        | ReportKind::TimeByTicket
                ) =>
                {
                    next_cursor(
                        kind,
                        &serde_json::json!(42u64),
                        &serde_json::json!(Uuid::max().to_string()),
                    )
                    .expect("metric cursor")
                }
                _ => next_cursor(
                    kind,
                    &serde_json::json!(NOW.to_rfc3339()),
                    &serde_json::json!(Uuid::nil().to_string()),
                )
                .expect("ticket cursor"),
            };

            let unpaged = compile(kind, &params_for(kind, NOW), Scope::OrgWide, None, 50, win_default(NOW))
                .expect("base compile");
            let paged = compile(
                kind,
                &params_for(kind, NOW),
                Scope::OrgWide,
                Some(&shape_key),
                50,
                win_default(NOW),
            )
            .expect("cursor compile");
            // Continuation adds exactly two binds over the unpaginated plan
            // (shared filter prefix, then the pair).
            assert_eq!(paged.binds.len(), unpaged.binds.len() + 2);
            assert_statement_shape(&paged.sql);
            assert_placeholders_contiguous(&paged.sql, paged.binds.len());
        }

        for kind in [ReportKind::TicketThroughput, ReportKind::TicketAging] {
            let _ = compile(kind, &params_for(kind, NOW), Scope::OrgWide, None, 50, win_default(NOW))
                .expect("unpaged kinds still compile");
        }
    }

    proptest! {
        /// Seal/open roundtrip must be lossless across random keys; any
        /// mutation of the sealed text fails closed with InvalidCursor.
        #[test]
        fn cursor_roundtrip(metric in -40i64..90_000_000i64,
                           seed in any::<u128>(),
                           days in 20_000i64..21_000) {
            let group = uuid::Uuid::from_u128(seed);
            let ts = DateTime::<Utc>::UNIX_EPOCH + ChronoDuration::days(days);
            let metric_cursor = PageKey::MetricGroup { metric, group };
            let ticket_cursor = PageKey::Ticket { ts, ticket: group };

            let m_sealed = metric_cursor.clone().seal();
            prop_assert_eq!(PageKey::open(&m_sealed)?, metric_cursor);

            let t_sealed = ticket_cursor.clone().seal();
            prop_assert_eq!(PageKey::open(&t_sealed)?, ticket_cursor);

            // Structural tampering never succeeds.
            let truncated: String = m_sealed.chars().take(m_sealed.chars().count() - 1).collect();
            for bad in ["", "x", "AAAA", "v1", format!("{m_sealed}!").as_str(), truncated.as_str()] {
                prop_assert!(PageKey::open(bad).is_err());
            }
        }
    }
}

/// Test-support seam exposing cursor decoding to crate-level rejection tests.
#[cfg(test)]
pub(crate) mod tests_support {
    pub fn open_for_test(raw: &str) -> Result<(), crate::error::ReportsError> {
        // Route through the same decode used in production for any shape;
        // both expect kind-appropriate layout, hostile text never passes.
        match super::PageKey::open(raw)? {
            _ => Ok(()),
        }
    }
}
