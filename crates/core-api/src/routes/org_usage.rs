//! Org resource-usage + quota read API for hosted-tier monitoring (TASKLIST
//! Wave 47 lane A6, migration 0131).
//!
//! Two read-only endpoints:
//!   * `GET /api/v1/orgs/{org_id}/usage` — the org's materialized usage
//!     counts (active users, tickets by category, recorded object-store
//!     bytes, automation runs, webhook deliveries, managed devices, and
//!     the honest proxy for API activity the core actually records).
//!   * `GET /api/v1/orgs/{org_id}/quotas` — the same numbers set beside
//!     any limits the org has CONFIGURED, with percentages.
//!
//! THE POSTURE — REPORT, NEVER GATE. Nothing in this module (or anywhere
//! else in the core) reads a quota to limit behaviour. There is no
//! throttle, no rejection, no backpressure keyed on these numbers: a
//! percentage over 100 renders, it does not bite. Configured limits are
//! ordinary org-scope `settings` rows (`usage_quota.*`), which is why an
//! org with none configured gets an explicit "no limits configured"
//! answer instead of a default that looks like a limit.
//!
//! HONESTY. Every number here comes from the materialized row, which
//! migration 0131 computes from recorded rows. Two facts a shared database
//! cannot attribute per org are REPORTED AS UNAVAILABLE with the reason
//! beside them rather than filled in with an estimate: API-call volume in
//! the last 24h (requests ARE counted — as the OTLP metric
//! `marula.org.requests`, emitted at the auth boundary by
//! `crate::otel_http::record_org` — but a metric series cannot be read back
//! into this endpoint's recorded-row answer, so it is named here rather than
//! estimated) and per-org database bytes (a shared table's pages belong to
//! every org sharing them). The unavailable list is DATA, not an apology: a
//! monitoring surface can render it, and a future lane can fill the gap
//! honestly.
//!
//! AUTHORIZATION (TASKLIST A6 (D)):
//!   * A human member reads their OWN org. The org id in the path must be
//!     the caller's own org, or the answer is a 404 — the same
//!     indistinguishable answer a nonexistent org gets, so the endpoint
//!     never confirms that another org exists (RLS's posture, applied at
//!     the boundary).
//!   * A service account (machine API key) additionally holds the
//!     `org_usage:read` permission (`Permission::ViewOrgUsage`, under
//!     `Module::Reporting`), which is what the `org_usage.read` plugin
//!     capability resolves onto. A machine caller without it gets a typed
//!     403.
//!   * The database read is RLS-scoped on top of both, so even a future
//!     authorization slip reads zero rows rather than another org's. That
//!     backstop is real only because the RLS scope is bound from the
//!     AUTHENTICATED PRINCIPAL, never from the path value the caller sent —
//!     see [`authorize_usage_read`] and the binding in the handlers.
//!
//! Non-purpose: no write surface for limits (the hosted tier configures
//! them out of band — a report does not need a knob), no per-org request
//! counting (would need a counter table this lane deliberately does not
//! invent), and no UI (the SDK carries the shape; a settings-hub surface
//! is a follow-up).

use crate::auth::middleware::{authed, require_permission};
use crate::auth::AuthedUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::policy::Permission;
use core_domain::usage::{percent_of, UsageMetric};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

/// The permission a machine caller must hold to read org usage. The wire
/// key is `org_usage:read`; the plugin capability `org_usage.read`
/// resolves onto it.
fn usage_read_permission() -> Permission {
    Permission::ViewOrgUsage
}

/// The authorization gate shared by both endpoints: any human member of
/// the org, or a service account holding `ViewOrgUsage`.
///
/// A path org that is not the caller's own org is a 404 — deliberately
/// identical to "no such org", so a cross-org probe learns nothing.
///
/// This check is the ONLY authorization gate, and both handlers bind the
/// RLS scope from `authed.principal.user.org_id` rather than from the path
/// value. That is what makes RLS a real backstop instead of a comment: were
/// the scope bound from the path, a future edit that relaxed or reordered
/// this check would have the database happily scoped to the target org and
/// return its row. `principal.user.org_id` is authoritative either way — the
/// JWT path pins it to `claims.org` in `get_user`, and the machine-key path
/// takes it from the owner-scoped `api_key_auth_lookup` — so the caller
/// cannot influence the scope the read happens under.
fn authorize_usage_read(authed: &AuthedUser, org_id: Uuid) -> ApiResult<()> {
    if authed.via_api_key {
        // A machine key is read-only by construction and org-bound; the
        // permission is the only thing that widens what it may observe.
        require_permission(&authed.principal, usage_read_permission())?;
    }
    if authed.principal.user.org_id != org_id {
        return Err(ApiError::NotFound);
    }
    Ok(())
}

/// GET /api/v1/orgs/{org_id}/usage — the org's current usage counts.
///
/// `refreshed_at` is when the numbers were computed; a consumer that
/// needs fresh numbers reads the row's own timestamp instead of assuming.
/// `unavailable` names every metric this build cannot compute honestly,
/// with the reason — it is a real, renderable list, not a placeholder.
#[utoipa::path(operation_id = "org_usage_get", get,
    path = "/api/orgs/{org_id}/usage", tag = "org", security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "the org to report on; must be the caller's own org")),
    responses((status = 200, body = OrgUsageView),
              (status = 401, description = "not authenticated"),
              (status = 403, description = "service account without org_usage:read"),
              (status = 404, description = "no such org for this caller")))]
pub async fn get_org_usage(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(org_id): Path<Uuid>,
) -> ApiResult<Json<OrgUsageView>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    authorize_usage_read(&authed, org_id)?;
    // RLS-scoped read (pool carries no app.org_id GUC). The scope comes from
    // the AUTHENTICATED PRINCIPAL, not from the path value the caller sent:
    // the 404 gate above has already established that the two are equal, so
    // binding from the principal keeps the database scope independent of any
    // request-supplied input and makes the read fail closed by construction.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let row = core_db::org_usage::get_usage(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    // No row yet: the refresh sweep has not reached this org. That is a
    // real state, and it is reported as one rather than by inventing
    // zeros — a fresh org with zero events and an unrefreshed org read
    // very differently to an operator investigating a queue stall.
    Ok(Json(match row {
        Some(row) => OrgUsageView {
            org_id,
            refreshed_at: Some(row.refreshed_at),
            active_users: row.active_users,
            tickets_by_category: TicketsByCategory {
                open: row.tickets_open,
                in_progress: row.tickets_in_progress,
                waiting: row.tickets_waiting,
                closed: row.tickets_closed,
            },
            tickets_total: row.tickets_total(),
            managed_devices: row.managed_devices,
            object_store_bytes: row.object_store_bytes,
            automation_runs: row.automation_runs,
            automation_runs_24h: row.automation_runs_24h,
            webhook_deliveries: row.webhook_deliveries,
            webhook_deliveries_24h: row.webhook_deliveries_24h,
            audit_events_24h: row.audit_events_24h,
            unavailable: unavailable_metrics(),
        },
        None => OrgUsageView::unrefreshed(org_id),
    }))
}

/// The org's current usage, one field per recorded metric.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OrgUsageView {
    pub org_id: Uuid,
    /// When these numbers were computed (the materialized row's own
    /// timestamp). `None` while the refresh sweep has not reached the org
    /// yet, which is exactly when the counts should not be trusted. Every
    /// `_24h` field below is a window that ENDS at this instant; as the row
    /// ages, so does the window, so read this before trusting those fields.
    #[schema(value_type = Option<String>, format = "date-time")]
    pub refreshed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Staff accounts that can still sign in. A disabled account is not
    /// using anything, so it is not counted here.
    #[schema(example = 14)]
    pub active_users: i64,
    /// Tickets per workflow category (the closed vocabulary). Category,
    /// not board status name: names are per-board and repeat across boards,
    /// so a name-keyed map would collapse distinct statuses.
    pub tickets_by_category: TicketsByCategory,
    /// Tickets across every category — the total an inventory limit
    /// constrains.
    #[schema(example = 64)]
    pub tickets_total: i64,
    /// Active managed client devices. This is an ACTIVE-INVENTORY count:
    /// `device_assets` has no connectivity or last-seen signal, so it cannot
    /// measure "connected" clients. Status is connector-compatibility free
    /// text matched case-insensitively, so orgs whose connectors store a
    /// different status string under-report — this is a floor, not a census.
    #[schema(example = 9)]
    pub managed_devices: i64,
    /// Recorded object-store payload bytes (a floor; see the method note in
    /// migration 0131 — staged import chunks record no size).
    #[schema(example = json!(2147483648_i64))]
    pub object_store_bytes: i64,
    /// Ledgered automation execution attempts since the org's first run.
    /// This is an ALL-TIME total, so it does not carry the staleness rule
    /// the `_24h` fields do — but it is still only as current as
    /// `refreshed_at`, since the row is written once per refresh.
    #[schema(example = 120)]
    pub automation_runs: i64,
    /// Ledgered automation execution attempts within
    /// `[refreshed_at - 24h, refreshed_at]`. The window is CLOSED at
    /// refresh time, so it ages with the row rather than sliding to the
    /// moment of the read; compare against `refreshed_at` before treating
    /// this as current.
    #[schema(example = 12)]
    pub automation_runs_24h: i64,
    /// Ledgered webhook delivery attempts since the org's first. One row per
    /// ATTEMPT, so a retried delivery counts three times. All-time, so no
    /// window rule — only the row's own `refreshed_at` currency.
    #[schema(example = 540)]
    pub webhook_deliveries: i64,
    /// Ledgered webhook delivery attempts within
    /// `[refreshed_at - 24h, refreshed_at]` — the same CLOSED,
    /// refresh-time-frozen window as `automation_runs_24h`.
    #[schema(example = 40)]
    pub webhook_deliveries_24h: i64,
    /// Audit events within `[refreshed_at - 24h, refreshed_at]` — the closest
    /// honest per-org proxy for API activity the core records. NOT an
    /// API-call count: reads are not audited and writes are audited
    /// best-effort, so this counts the mutations the core chose to record,
    /// not the requests that arrived. The window is closed at refresh time,
    /// as with the other `_24h` fields.
    #[schema(example = 300)]
    pub audit_events_24h: i64,
    /// Metrics this build cannot compute honestly, each with the reason.
    /// Empty is not possible today; the list exists so a monitoring
    /// surface can render the gap rather than silently miss a column.
    pub unavailable: Vec<UnavailableMetric>,
}

/// Tickets by workflow category. The status NAME is per-board and repeats
/// across boards, so a name-keyed map would merge different statuses;
/// `category` is the closed vocabulary the workflow layer owns.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
pub struct TicketsByCategory {
    pub open: i64,
    pub in_progress: i64,
    pub waiting: i64,
    pub closed: i64,
}

/// A metric the core does not report, and why. The reason is prose for a
/// HUMAN operator reading the surface; `metric` is the stable token a
/// dashboard can group on.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UnavailableMetric {
    pub metric: String,
    pub reason: String,
}

/// The two metrics this build deliberately does not compute, with the
/// reason each is impossible from recorded rows. A gap that cannot be
/// stated is how a fake number gets shipped, so it is stated once here
/// and rendered everywhere.
fn unavailable_metrics() -> Vec<UnavailableMetric> {
    vec![
        UnavailableMetric {
            metric: "api_calls_last_24h".into(),
            reason: "this endpoint reports RECORDED rows, and per-org request volume \
                     is not recorded in one: requests are counted as the OTLP metric \
                     `marula.org.requests` (org_id × status class) at the auth \
                     boundary, and metric series cannot be read back out of the \
                     export pipeline into a query. Chart it from the metrics store \
                     with increase(...[24h]); it is deliberately not estimated here"
                .into(),
        },
        UnavailableMetric {
            metric: "database_bytes".into(),
            reason: "per-org database size is not attributable in a shared database: \
                     Postgres sizes relations and pages, and a shared table's pages \
                     belong to every org that shares them"
                .into(),
        },
    ]
}

impl OrgUsageView {
    /// The view for an org whose row has not been refreshed yet. The
    /// counts are zeroed and `refreshed_at` is `None`, so the payload says
    /// "not measured" through the timestamp rather than through a number
    /// that happens to be zero.
    fn unrefreshed(org_id: Uuid) -> Self {
        Self {
            org_id,
            refreshed_at: None,
            active_users: 0,
            tickets_by_category: TicketsByCategory {
                open: 0,
                in_progress: 0,
                waiting: 0,
                closed: 0,
            },
            tickets_total: 0,
            managed_devices: 0,
            object_store_bytes: 0,
            automation_runs: 0,
            automation_runs_24h: 0,
            webhook_deliveries: 0,
            webhook_deliveries_24h: 0,
            audit_events_24h: 0,
            unavailable: unavailable_metrics(),
        }
    }
}

/// GET /api/v1/orgs/{org_id}/quotas — usage beside configured limits.
///
/// Informational by construction: a percentage here is a measurement, and
/// nothing in the core reads this endpoint to limit anything. `limits`
/// is a real configuration read — an org that has configured none reports
/// `false`, because a default limit is a fake gate.
#[utoipa::path(operation_id = "org_quotas_get", get,
    path = "/api/orgs/{org_id}/quotas", tag = "org", security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "the org to report on; must be the caller's own org")),
    responses((status = 200, body = OrgQuotaView),
              (status = 401, description = "not authenticated"),
              (status = 403, description = "service account without org_usage:read"),
              (status = 404, description = "no such org for this caller")))]
pub async fn get_org_quotas(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(org_id): Path<Uuid>,
) -> ApiResult<Json<OrgQuotaView>> {
    let authed = authed::<AuthedUser>(&extensions)?;
    authorize_usage_read(&authed, org_id)?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let usage = core_db::org_usage::get_usage(&mut *tx, org_id).await?;
    let limits = core_db::org_usage::list_configured_limits(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let usage = usage.as_ref();
    // A metric's usage value comes from the materialized row; with no
    // row yet there is nothing measured, which is not the same as zero.
    let usage_for = |metric: UsageMetric| usage.map(|row| row.metric_value(metric));

    let metrics: Vec<QuotaMetricView> = UsageMetric::ALL
        .into_iter()
        .map(|metric| {
            let (usage_value, limit) = match limits.iter().find(|(m, _)| *m == metric) {
                Some((_, limit)) => (usage_for(metric), Some(*limit)),
                None => (usage_for(metric), None),
            };
            QuotaMetricView {
                metric: metric.as_str().to_string(),
                setting_key: metric.limit_setting_key().to_string(),
                usage: usage_value,
                limit,
                // `None` for "no limit", which is the honest answer for an
                // org that configured nothing — not 0% and not an error.
                percent: usage_value.and_then(|used| percent_of(used, limit)),
            }
        })
        .collect();
    let any_limits = metrics.iter().any(|m| m.limit.is_some());

    Ok(Json(OrgQuotaView {
        org_id,
        refreshed_at: usage.map(|row| row.refreshed_at),
        limits_configured: any_limits,
        metrics,
    }))
}

/// One metric beside its configured limit, if any, and the percentage of
/// it in use.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OrgQuotaView {
    pub org_id: Uuid,
    /// When the usage figures were computed. `None` while the org's row has
    /// never been refreshed. Note that `UsageMetric::AutomationRuns` and
    /// `WebhookDeliveries` read their all-time totals from this row, so the
    /// totals are as of this instant rather than as of the request.
    #[schema(value_type = Option<String>, format = "date-time")]
    pub refreshed_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether the org configured ANY limit. `false` means every
    /// `percent` below is `None` because there is nothing to measure
    /// against — the answer for an unconfigured org, not a default.
    pub limits_configured: bool,
    /// Every reportable metric in a stable order, whether or not a limit
    /// is configured for it.
    pub metrics: Vec<QuotaMetricView>,
}

/// A single metric's reading: the measured usage, the configured limit
/// (when one exists), and the percentage of the limit in use.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct QuotaMetricView {
    /// Stable metric token (SDK-visible).
    pub metric: String,
    /// The org-settings key the limit is read from.
    pub setting_key: String,
    /// The measured value, or `None` when the org's usage row has never
    /// been refreshed — an unmeasured org is not a zero org.
    #[schema(value_type = Option<i64>)]
    pub usage: Option<i64>,
    /// The configured limit, or `None` when the org configured none.
    #[schema(value_type = Option<i64>)]
    pub limit: Option<i64>,
    /// `usage` as a percentage of `limit`, to one decimal. `None` whenever
    /// there is no limit to measure against (or the limit is zero), never
    /// an error and never clamped: 300.0 is a valid, reportable reading.
    #[schema(value_type = Option<f64>)]
    pub percent: Option<f64>,
}
