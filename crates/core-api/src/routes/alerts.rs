//! Alert ingestor (TASKLIST Phase 3): HMAC-signed webhook ingest →
//! dedup/correlation → incident tickets (a storm collapses to ONE ticket).
//!
//! Surface layout (one module, two mounts — kept in a single file to
//! minimize merge surface on mod.rs):
//!
//! * `POST /api/alerts/ingest/{source_ref}` — the machine-to-machine
//!   ingest endpoint. Mounted OUTSIDE the JWT middleware (like the email
//!   inbound webhook): the `source_ref` path segment resolves org + sealed
//!   secret through a narrow SECURITY DEFINER lookup (0014 seam pattern),
//!   then `X-Marula-Signature` (hex HMAC-SHA256 over the raw body) is
//!   verified constant-time. Dedup/correlation runs synchronously here —
//!   the worker queue is deliberately not involved.
//! * `/api/alert-sources` + `GET /api/alerts` — the staff surface on the
//!   Wave-8a envelope: idempotency-keyed writes, read-your-write mutation
//!   metadata, typed error codes.
//!
//! Envelope mapping for the machine endpoint: the provider's `dedup_key`
//! IS the idempotency key — UNIQUE (org, source, dedup_key) makes a
//! replayed delivery return the ORIGINAL stored alert with zero side
//! effects (no second ticket, no duplicate events or audit rows), which
//! is a stronger replay contract than a client-chosen idempotency key.
//! The response body IS the stored alert (read-your-write); every failure
//! carries the central typed `reason` envelope.
//!
//! Storm rule: alerts sharing a `correlation_key` (device/monitor/check
//! identity; defaults to the dedup key when absent) within the source's
//! correlation window fold into the ONE incident ticket anchored by the
//! first alert. A partial unique index enforces at most one OPEN group
//! per key, and the group row is locked FOR UPDATE inside the ingest
//! transaction, so a concurrent burst cannot double-anchor a second
//! ticket.

use crate::auth::middleware::{authed, require_permission};
use crate::crypto;
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{Extensions, HeaderMap};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use core_db::alerting::{
    self, AlertFilter, AlertIncidentRow, AlertRow, AlertSourceAuth, AlertSourceRow,
};
use core_domain::entities::{Actor, AuditEvent};
use core_domain::events::types;
use core_domain::policy::Permission;
use core_domain::status::Priority;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

/// Signature header for the ingest endpoint (same scheme as the email
/// inbound webhook: hex HMAC-SHA256 over the exact request body).
pub const SIGNATURE_HEADER: &str = "X-Marula-Signature";

// --- wire types ---------------------------------------------------------------

/// Alert severity vocabulary. Maps onto ticket priorities when an alert
/// anchors an incident ticket: critical→P1, warning→P2, info→P3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

impl AlertSeverity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "info" => Some(Self::Info),
            "warning" => Some(Self::Warning),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }

    pub const fn priority(self) -> Priority {
        match self {
            Self::Critical => Priority::P1,
            Self::Warning => Priority::P2,
            Self::Info => Priority::P3,
        }
    }
}

/// One normalized alert (canonical ingest document + ledger row view).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AlertView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub source_id: Uuid,
    /// Provider's per-event identity; the replay gate per source.
    pub dedup_key: String,
    /// Storm grouping key (defaults to the dedup key server-side).
    pub correlation_key: String,
    pub title: String,
    pub description: Option<String>,
    pub severity: String,
    #[schema(value_type = String)]
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = Object)]
    pub attributes: Value,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub incident_ticket_id: Option<Uuid>,
    #[schema(value_type = String)]
    pub received_at: chrono::DateTime<chrono::Utc>,
    /// Triage state (0089): "open" | "acknowledged" | "resolved".
    pub status: String,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub acked_by: Option<Uuid>,
    #[schema(value_type = Option<String>)]
    pub acked_at: Option<chrono::DateTime<chrono::Utc>>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub resolved_by: Option<Uuid>,
    #[schema(value_type = Option<String>)]
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<AlertRow> for AlertView {
    fn from(r: AlertRow) -> Self {
        Self {
            id: r.id,
            org_id: r.org_id,
            source_id: r.source_id,
            dedup_key: r.dedup_key,
            correlation_key: r.correlation_key,
            title: r.title,
            description: r.description,
            severity: r.severity,
            occurred_at: r.occurred_at,
            attributes: r.attributes,
            incident_ticket_id: r.incident_ticket_id,
            received_at: r.received_at,
            status: r.status,
            acked_by: r.acked_by,
            acked_at: r.acked_at,
            resolved_by: r.resolved_by,
            resolved_at: r.resolved_at,
        }
    }
}

/// The storm group behind an incident ticket.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IncidentView {
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: Uuid,
    pub ticket_number: String,
    /// How many alerts have collapsed into this ticket so far.
    pub alert_count: i32,
    #[schema(value_type = String)]
    pub first_alert_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = String)]
    pub last_alert_at: chrono::DateTime<chrono::Utc>,
}

impl IncidentView {
    fn from_row(row: AlertIncidentRow, ticket_number: String) -> Self {
        Self {
            ticket_id: row.ticket_id,
            ticket_number,
            alert_count: row.alert_count,
            first_alert_at: row.first_alert_at,
            last_alert_at: row.last_alert_at,
        }
    }
}

/// POST /api/alerts/ingest — outcome of one signed delivery.
#[derive(Debug, Serialize, ToSchema)]
pub struct IngestResponse {
    /// `incident_created` (this alert anchored a NEW incident ticket),
    /// `collapsed` (folded into the still-open storm incident), or
    /// `duplicate` (replayed dedup_key — the stored alert is returned,
    /// nothing was written).
    pub status: String,
    pub alert: AlertView,
    /// Present for `incident_created` and `collapsed`; `None` when the
    /// anchor ticket vanished before a replay answered.
    pub incident: Option<IncidentView>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct IngestAlertRequest {
    /// Provider's per-event identity. REQUIRED: this is the replay gate.
    pub dedup_key: String,
    pub title: String,
    pub description: Option<String>,
    pub severity: AlertSeverity,
    /// Storm grouping key; defaults to `dedup_key` when absent so
    /// uncorrelated alerts each anchor their own incident.
    #[serde(default)]
    pub correlation_key: Option<String>,
    /// When the condition started (defaults to receipt time).
    #[serde(default)]
    pub occurred_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Provider-specific structured context (must be a JSON object).
    #[serde(default)]
    pub attributes: Value,
}

// --- ingest endpoint ----------------------------------------------------------

/// POST /api/alerts/ingest/{source_ref} — HMAC-verified alert ingest.
///
/// Responses: 200 with a terminal `status` (incident_created / collapsed /
/// duplicate); 401 missing/bad signature; 403 disabled source; 404 unknown
/// source_ref; 422 malformed payload.
#[utoipa::path(
    operation_id = "alerts_ingest",
    post,
    path = "/api/alerts/ingest/{source_ref}",
    tag = "alerts",
    params(("source_ref" = String, Path, description = "Public ingest routing id of an alert source")),
    request_body = IngestAlertRequest,
    responses(
        (status = 200, body = IngestResponse),
        (status = 401, description = "missing or invalid signature"),
        (status = 403, description = "alert source disabled"),
        (status = 404, description = "unknown source_ref"),
        (status = 422, description = "validation failed")
    )
)]
pub async fn ingest(
    State(state): State<AppState>,
    Path(source_ref): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<IngestResponse>> {
    // Resolve org + sealed secret BEFORE anything else (pre-auth lookup;
    // the ref is public routing info, not a credential).
    let source = alerting::auth_lookup_source(&state.pool, source_ref.trim())
        .await?
        .ok_or(ApiError::NotFound)?;
    if !source.r_enabled {
        return Err(ApiError::Forbidden("alert source is disabled".into()));
    }

    // HMAC over the RAW body — tampering invalidates even a stale valid
    // signature. Constant-time via core_mail::hmac.
    let presented = headers
        .get(SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if presented.is_empty() {
        return Err(ApiError::Unauthorized("missing webhook signature".into()));
    }
    let secret = crypto::open(&state.config.secret_key, &source.r_secret_encrypted)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("alert source secret: {e}")))?;
    if !core_mail::hmac::verify(&secret, &body, presented) {
        return Err(ApiError::Unauthorized("invalid webhook signature".into()));
    }

    let req: IngestAlertRequest = serde_json::from_slice(&body).map_err(|e| {
        // Raw serde text stays server-side (Wave 13 lane 1).
        tracing::debug!(error = %e, "alert payload failed to parse");
        ApiError::Validation("alert payload is malformed or has the wrong shape".into())
    })?;
    validate_ingest(&req)?;

    let correlation_key = req
        .correlation_key
        .clone()
        .filter(|k| !k.trim().is_empty())
        .unwrap_or_else(|| req.dedup_key.clone());
    let occurred_at = req.occurred_at.unwrap_or_else(chrono::Utc::now);
    let now = chrono::Utc::now();

    let response =
        ingest_correlated(&state, &source, &req, &correlation_key, occurred_at, now).await?;
    Ok(Json(response))
}

fn validate_ingest(req: &IngestAlertRequest) -> ApiResult<()> {
    if req.dedup_key.trim().is_empty() {
        return Err(ApiError::Validation("dedup_key is required".into()));
    }
    if req.dedup_key.len() > 200 {
        return Err(ApiError::Validation(
            "dedup_key must be at most 200 characters".into(),
        ));
    }
    if req.title.trim().is_empty() {
        return Err(ApiError::Validation("title is required".into()));
    }
    if req.title.len() > 200 {
        return Err(ApiError::Validation(
            "title must be at most 200 characters".into(),
        ));
    }
    if let Some(key) = req.correlation_key.as_deref() {
        if key.len() > 200 {
            return Err(ApiError::Validation(
                "correlation_key must be at most 200 characters".into(),
            ));
        }
    }
    if let Some(desc) = req.description.as_deref() {
        if desc.len() > 10_000 {
            return Err(ApiError::Validation(
                "description must be at most 10000 characters".into(),
            ));
        }
    }
    if !req.attributes.is_null() && !req.attributes.is_object() {
        return Err(ApiError::Validation(
            "attributes must be a JSON object".into(),
        ));
    }
    Ok(())
}

/// The synchronous dedup/correlation pipeline inside ONE org-bound
/// transaction: replay gate → storm group lock → collapse-or-anchor →
/// events + audit, all-or-nothing.
async fn ingest_correlated(
    state: &AppState,
    source: &AlertSourceAuth,
    req: &IngestAlertRequest,
    correlation_key: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> ApiResult<IngestResponse> {
    let org_id = source.r_org_id;
    let attributes = if req.attributes.is_null() {
        Value::Object(serde_json::Map::new())
    } else {
        req.attributes.clone()
    };

    org_tx::with_org_tx(state, org_id, org_id, |mut ctx| async move {
        // 1. Replay gate: a duplicate dedup_key writes NOTHING — the
        //    stored alert answers. This is the ingest surface's
        //    idempotency-key contract (wave-8a envelope, machine flavor).
        let inserted = alerting::insert_alert(
            &mut *ctx.tx,
            alerting::NewAlert {
                id: Uuid::now_v7(),
                org_id,
                source_id: source.r_id,
                dedup_key: req.dedup_key.trim(),
                correlation_key,
                title: req.title.trim(),
                description: req.description.as_deref().map(str::trim),
                severity: req.severity.as_str(),
                occurred_at,
                attributes: &attributes,
            },
        )
        .await?;
        let Some(alert) = inserted else {
            let stored =
                alerting::get_alert_by_dedup_key(&mut *ctx.tx, org_id, source.r_id, &req.dedup_key)
                    .await?;
            let incident = match stored.incident_ticket_id {
                Some(ticket_id) => {
                    let number = ticket_number_of(&mut *ctx.tx, org_id, ticket_id).await;
                    alerting::get_incident_by_ticket(&mut *ctx.tx, org_id, ticket_id)
                        .await?
                        .map(|g| IncidentView::from_row(g, number))
                }
                None => None,
            };
            return Ok((
                ctx,
                IngestResponse {
                    status: "duplicate".into(),
                    alert: stored.into(),
                    incident,
                },
            ));
        };

        // 2. Correlation: lock the OPEN storm group for this key (the
        //    partial unique index guarantees at most one row).
        let group =
            alerting::lock_open_incident(&mut *ctx.tx, org_id, source.r_id, correlation_key)
                .await?;
        let window = chrono::Duration::seconds(i64::from(source.r_correlation_window_secs));
        let live = matches!(&group, Some(g) if now - g.last_alert_at <= window);

        let (status, incident) = match group {
            // 3a. Storm collapse: fold into the ONE open incident ticket.
            Some(g) if live => {
                let g = alerting::attach_alert_to_incident(&mut *ctx.tx, g.id, now).await?;
                alerting::link_alert_to_ticket(&mut *ctx.tx, alert.id, g.ticket_id).await?;
                emit_ingested(
                    &mut ctx,
                    source,
                    &alert,
                    req,
                    correlation_key,
                    "collapsed",
                    i64::from(g.alert_count),
                    occurred_at,
                )
                .await?;
                audit_ingest(&mut ctx, source, &alert, "collapsed", g.alert_count).await?;
                audit_collapse(&mut ctx, source, &alert, &g).await?;
                let number = ticket_number_of(&mut *ctx.tx, org_id, g.ticket_id).await;
                ("collapsed", Some(IncidentView::from_row(g, number)))
            }
            // 3b. Expired window or first alert for the key: close any
            //     stale group, then anchor a fresh incident ticket through
            //     the standard creation path.
            maybe_stale => {
                if let Some(g) = maybe_stale {
                    alerting::close_incident(&mut *ctx.tx, g.id).await?;
                }
                let incident =
                    anchor_storm(&mut ctx, source, req, &alert, correlation_key, occurred_at)
                        .await?;
                ("incident_created", Some(incident))
            }
        };

        Ok((
            ctx,
            IngestResponse {
                status: status.into(),
                alert: alert.into(),
                incident,
            },
        ))
    })
    .await
}

/// Anchors a fresh storm: incident ticket via the standard creation path
/// (entry status → number allocation → insert → SLA init →
/// `ticket.created`), then opens the group and links the anchor alert.
async fn anchor_storm(
    ctx: &mut org_tx::OrgTx<'_>,
    source: &AlertSourceAuth,
    req: &IngestAlertRequest,
    alert: &AlertRow,
    correlation_key: &str,
    occurred_at: chrono::DateTime<chrono::Utc>,
) -> ApiResult<IncidentView> {
    let org_id = source.r_org_id;
    let status = core_db::boards::default_status_for_board(&mut *ctx.tx, source.r_board_id)
        .await
        .map_err(|_| ApiError::Conflict("alert source board has no entry status".into()))?;
    let (prefix, seq) = core_db::orgs::next_ticket_number(&mut *ctx.tx, org_id).await?;
    let number = format!("{prefix}-{seq}");

    let mut body = String::new();
    if let Some(desc) = req.description.as_deref().map(str::trim) {
        if !desc.is_empty() {
            body.push_str(desc);
            body.push_str("\n\n");
        }
    }
    body.push_str(&format!(
        "Ingested via alert source `{}`.\nCorrelation key: {}\nSeverity: {}\nDedup key: {}",
        source.r_source_ref, alert.correlation_key, alert.severity, alert.dedup_key,
    ));

    let ticket = core_db::tickets::insert_ticket(
        &mut ctx.tx,
        core_db::tickets::NewTicket {
            id: Uuid::now_v7(),
            org_id,
            number: &number,
            board_id: source.r_board_id,
            status_id: status.id,
            title: alert.title.trim(),
            body: Some(body.as_str()),
            priority: req.severity.priority(),
            client_id: source.r_client_id,
            site_id: None,
            contact_id: None,
            assignee_id: None,
            requester_id: None,
            due_at: None,
        },
    )
    .await?;

    if let Ok(policy) = core_db::sla::get_default_policy(&mut *ctx.tx, org_id).await {
        let sla = core_domain::entities::TicketSlaState::init(
            org_id,
            ticket.id,
            &policy,
            ticket.priority,
            ticket.created_at,
        );
        core_db::sla::upsert_ticket_state(&mut *ctx.tx, &sla).await?;
    }

    ctx.emit(
        types::TICKET_CREATED,
        "ticket",
        Some(ticket.id.to_string().as_str()),
        &serde_json::to_value(core_domain::events::TicketCreated {
            ticket_id: ticket.id,
            number: ticket.number.0.clone(),
            board_id: ticket.board_id,
            title: ticket.title.clone(),
            priority: ticket.priority,
            client_id: ticket.client_id,
            assignee_id: ticket.assignee_id,
            created_at: ticket.created_at,
        })
        .unwrap_or_default(),
    )
    .await?;

    let group = alerting::open_incident(
        &mut *ctx.tx,
        Uuid::now_v7(),
        org_id,
        source.r_id,
        correlation_key,
        ticket.id,
        occurred_at,
    )
    .await?;
    alerting::link_alert_to_ticket(&mut *ctx.tx, alert.id, ticket.id).await?;
    emit_ingested(
        ctx,
        source,
        alert,
        req,
        correlation_key,
        "incident_created",
        1,
        occurred_at,
    )
    .await?;
    audit_ingest(ctx, source, alert, "incident_created", 1).await?;

    Ok(IncidentView::from_row(group, ticket.number.0.clone()))
}

#[allow(clippy::too_many_arguments)]
async fn emit_ingested(
    ctx: &mut org_tx::OrgTx<'_>,
    source: &AlertSourceAuth,
    alert: &AlertRow,
    req: &IngestAlertRequest,
    correlation_key: &str,
    outcome: &str,
    alert_count: i64,
    occurred_at: chrono::DateTime<chrono::Utc>,
) -> ApiResult<()> {
    ctx.emit(
        types::ALERT_INGESTED,
        "alert",
        Some(alert.id.to_string().as_str()),
        &serde_json::to_value(core_domain::events::AlertIngested {
            alert_id: alert.id,
            org_id: source.r_org_id,
            source_id: source.r_id,
            source_ref: source.r_source_ref.clone(),
            dedup_key: req.dedup_key.trim().to_string(),
            correlation_key: correlation_key.to_string(),
            title: alert.title.clone(),
            severity: alert.severity.clone(),
            outcome: outcome.to_string(),
            ticket_id: alert.incident_ticket_id,
            alert_count,
            occurred_at,
        })
        .unwrap_or_default(),
    )
    .await
}

fn ingest_actor(source: &AlertSourceAuth) -> Actor {
    Actor::Connector(format!("alert-source:{}", source.r_source_ref))
}

async fn audit_ingest(
    ctx: &mut org_tx::OrgTx<'_>,
    source: &AlertSourceAuth,
    alert: &AlertRow,
    outcome: &str,
    alert_count: i32,
) -> ApiResult<()> {
    core_db::audit::insert(
        &mut *ctx.tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: source.r_org_id,
            actor: ingest_actor(source),
            action: "alert.ingested".into(),
            entity_type: Some("alert".into()),
            entity_id: Some(alert.id.to_string()),
            detail: serde_json::json!({
                "dedup_key": alert.dedup_key,
                "correlation_key": alert.correlation_key,
                "severity": alert.severity,
                "outcome": outcome,
                "alert_count": alert_count,
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    Ok(())
}

async fn audit_collapse(
    ctx: &mut org_tx::OrgTx<'_>,
    source: &AlertSourceAuth,
    alert: &AlertRow,
    group: &AlertIncidentRow,
) -> ApiResult<()> {
    core_db::audit::insert(
        &mut *ctx.tx,
        &AuditEvent {
            id: Uuid::now_v7(),
            org_id: source.r_org_id,
            actor: ingest_actor(source),
            action: "alert.storm_collapsed".into(),
            entity_type: Some("ticket".into()),
            entity_id: Some(group.ticket_id.to_string()),
            detail: serde_json::json!({
                "alert_id": alert.id,
                "correlation_key": group.correlation_key,
                "alert_count": group.alert_count,
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    Ok(())
}

async fn ticket_number_of<'e, E>(executor: E, org_id: Uuid, ticket_id: Uuid) -> String
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    core_db::tickets::get_ticket(executor, org_id, ticket_id)
        .await
        .map(|t| t.number.0)
        .unwrap_or_default()
}

// --- alert source admin (staff) -----------------------------------------------

/// An ingest source as the admin surface sees it. The HMAC secret is
/// NEVER here — it is returned exactly once, in the create response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AlertSourceView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    #[schema(value_type = uuid::Uuid)]
    pub org_id: Uuid,
    pub name: String,
    /// Path segment of the ingest URL (public routing id).
    pub source_ref: String,
    #[schema(value_type = uuid::Uuid)]
    pub board_id: Uuid,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    pub correlation_window_secs: i32,
    pub enabled: bool,
    #[schema(value_type = String)]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[schema(value_type = String)]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<AlertSourceRow> for AlertSourceView {
    fn from(r: AlertSourceRow) -> Self {
        Self {
            id: r.id,
            org_id: r.org_id,
            name: r.name,
            source_ref: r.source_ref,
            board_id: r.board_id,
            client_id: r.client_id,
            correlation_window_secs: r.correlation_window_secs,
            enabled: r.enabled,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// Instant-first mutation response for a source (IF2). `secret` is the
/// raw HMAC signing secret — populated ONLY by the create response and
/// never again (same show-once contract as webhooks/api-keys).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AlertSourceMutationResponse {
    #[serde(flatten)]
    pub source: AlertSourceView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    pub mutation: instant::MutationMeta,
}

impl AlertSourceMutationResponse {
    /// The response body stored under an idempotency key. The placeholder
    /// mutation metadata here is overwritten by `instant::record` before
    /// storage, so replays carry the real applied metadata.
    fn wire(
        source: AlertSourceView,
        secret: Option<String>,
        seq: i64,
        updated_at: chrono::DateTime<chrono::Utc>,
        key: Option<&str>,
    ) -> Self {
        Self {
            source,
            secret,
            mutation: instant::MutationMeta {
                seq,
                version: seq,
                updated_at,
                before: None,
                after: Value::Null,
                idempotency_key: key.map(str::to_string),
            },
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AlertSourceList {
    pub sources: Vec<AlertSourceView>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateAlertSourceRequest {
    pub name: String,
    /// Optional custom routing id (3–64 chars of `[a-z0-9-]`); generated
    /// when absent. Globally unique — collisions are a typed 409.
    pub source_ref: Option<String>,
    /// Board incident tickets land on.
    #[schema(value_type = uuid::Uuid)]
    pub board_id: Uuid,
    /// Optional client context stamped onto created incident tickets.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
    /// Storm window in seconds (10..=86400; default 900).
    pub correlation_window_secs: Option<i32>,
    /// Instant-first autosave parameters (IF1) — idempotency key.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatchAlertSourceRequest {
    pub name: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub board_id: Option<Uuid>,
    /// `Some(None)` clears; `Some(Some(v))` sets; `None` = no change.
    #[schema(value_type = Option<Option<uuid::Uuid>>)]
    pub client_id: Option<Option<Uuid>>,
    pub correlation_window_secs: Option<i32>,
    pub enabled: Option<bool>,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// GET /api/alert-sources — newest-first config list (no secrets).
#[utoipa::path(operation_id = "alert_sources_list", get, path = "/api/alert-sources", tag = "alerts", security(("bearer" = [])),
    responses((status = 200, body = AlertSourceList)))]
pub async fn sources_list(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<AlertSourceList>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManagePlugins)?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows =
        core_db::alerting::list_alert_sources(&mut *tx, authed.principal.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(AlertSourceList {
        sources: rows.into_iter().map(Into::into).collect(),
    }))
}

/// POST /api/alert-sources — creates an ingest source, generates + seals
/// its HMAC secret (returned ONCE), and writes the audit trail.
/// Instant-first (IF1/IF2): idempotency-keyed, read-your-write response.
#[utoipa::path(operation_id = "alert_sources_create", post, path = "/api/alert-sources", tag = "alerts", security(("bearer" = [])),
    request_body = CreateAlertSourceRequest,
    responses((status = 200, body = AlertSourceMutationResponse),
              (status = 409, description = "source_ref collision or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn sources_create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateAlertSourceRequest>,
) -> ApiResult<Json<AlertSourceMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManagePlugins)?;
    if req.name.trim().is_empty() {
        return Err(ApiError::Validation("name is required".into()));
    }
    let source_ref = match req.source_ref.as_deref() {
        Some(custom) => {
            let custom = custom.trim();
            let ok_len = (3..=64).contains(&custom.len());
            let ok_chars = custom
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if !ok_len || !ok_chars {
                return Err(ApiError::Validation(
                    "source_ref must be 3-64 chars of [a-z0-9-]".into(),
                ));
            }
            custom.to_string()
        }
        None => format!("alsrc-{}", Uuid::now_v7().simple()),
    };
    let window = req.correlation_window_secs.unwrap_or(900);
    if !(10..=86_400).contains(&window) {
        return Err(ApiError::Validation(
            "correlation_window_secs must be between 10 and 86400".into(),
        ));
    }

    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    // The board must exist in THIS org (RLS-visible) before we mint a
    // secret against it. RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    core_db::boards::get_board(&mut *tx, org_id, req.board_id)
        .await
        .map_err(|_| ApiError::Validation("board_id does not resolve in your org".into()))?;
    tx.commit().await.map_err(ApiError::from)?;

    // Fresh 256-bit secret; sealed immediately, returned once.
    let raw_secret = format!("alsrc_{}", Uuid::now_v7().simple());
    let sealed = crypto::seal(&state.config.secret_key, raw_secret.as_bytes())
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))?;

    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "alert_source",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let id = Uuid::now_v7();
        let row = core_db::alerting::insert_alert_source(
            &mut *ctx.tx,
            core_db::alerting::NewAlertSource {
                id,
                org_id,
                name: req.name.trim(),
                source_ref: &source_ref,
                secret_encrypted: &sealed,
                board_id: req.board_id,
                client_id: req.client_id,
                correlation_window_secs: window,
                created_by: user_id,
            },
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "alert_source",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "alert_source.created".into(),
                entity_type: Some("alert_source".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "source_ref": row.source_ref,
                    "board_id": row.board_id,
                    "correlation_window_secs": row.correlation_window_secs,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "alert_source",
                entity_id: id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(AlertSourceView::from(row.clone()))
                    .unwrap_or(Value::Null),
                updated_at: row.updated_at,
                key: key.as_deref(),
                payload: &AlertSourceMutationResponse::wire(
                    row.clone().into(),
                    Some(raw_secret.clone()),
                    write_seq,
                    row.updated_at,
                    key.as_deref(),
                ),
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(AlertSourceMutationResponse {
                source: row.into(),
                secret: Some(raw_secret),
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/alert-sources/{id} — re-point the board, rename, resize the
/// storm window, or disable. Instant-first (IF1/IF2).
#[utoipa::path(operation_id = "alert_sources_update", patch, path = "/api/alert-sources/{id}", tag = "alerts", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = PatchAlertSourceRequest,
    responses((status = 200, body = AlertSourceMutationResponse),
              (status = 404, description = "source not visible in scope (rls_sparse)"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "validation failed")))]
pub async fn sources_patch(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchAlertSourceRequest>,
) -> ApiResult<Json<AlertSourceMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, Permission::ManagePlugins)?;
    if let Some(name) = req.name.as_deref() {
        if name.trim().is_empty() {
            return Err(ApiError::Validation("name must not be empty".into()));
        }
    }
    if let Some(window) = req.correlation_window_secs {
        if !(10..=86_400).contains(&window) {
            return Err(ApiError::Validation(
                "correlation_window_secs must be between 10 and 86400".into(),
            ));
        }
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    // Validate a re-pointed board against THIS org before opening the
    // transaction (keeps the closure free of pool borrows).
    // RLS-scoped read (pool carries no app.org_id GUC).
    if let Some(board_id) = req.board_id {
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org_and_user(&mut tx, org_id, user_id)
            .await
            .map_err(ApiError::from)?;
        core_db::boards::get_board(&mut *tx, org_id, board_id)
            .await
            .map_err(|_| ApiError::Validation("board_id does not resolve in your org".into()))?;
        tx.commit().await.map_err(ApiError::from)?;
    }
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "alert_source",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::alerting::get_alert_source(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| crate::instant::mutation_scope_err(ApiError::from(e)))?;
        let row = core_db::alerting::update_alert_source(
            &mut *ctx.tx,
            org_id,
            id,
            core_db::alerting::AlertSourcePatch {
                name: req.name.as_deref().map(str::trim),
                board_id: req.board_id,
                client_id: req.client_id,
                correlation_window_secs: req.correlation_window_secs,
                enabled: req.enabled,
            },
        )
        .await
        .map_err(|e| crate::instant::mutation_scope_err(ApiError::from(e)))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "alert_source",
                entity_id: id,
                version: req.instant.version,
            },
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: "alert_source.updated".into(),
                entity_type: Some("alert_source".into()),
                entity_id: Some(id.to_string()),
                detail: serde_json::json!({
                    "source_ref": row.source_ref,
                    "enabled": row.enabled,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "alert_source",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(
                    serde_json::to_value(AlertSourceView::from(before)).unwrap_or(Value::Null),
                ),
                after: serde_json::to_value(AlertSourceView::from(row.clone()))
                    .unwrap_or(Value::Null),
                updated_at: row.updated_at,
                key: key.as_deref(),
                payload: &AlertSourceMutationResponse::wire(
                    row.clone().into(),
                    None,
                    write_seq,
                    row.updated_at,
                    key.as_deref(),
                ),
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(AlertSourceMutationResponse {
                source: row.into(),
                secret: None,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

// --- alerts read surface --------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams, ToSchema)]
pub struct AlertListQuery {
    #[schema(value_type = Option<uuid::Uuid>)]
    pub source_id: Option<Uuid>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    /// Triage filter (0089): "open" | "acknowledged" | "resolved" —
    /// absent returns all states.
    pub status: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AlertListResponse {
    pub alerts: Vec<AlertView>,
}

/// Instant-first mutation response for a triage action (0089). The alert
/// IS the read-your-write result; `mutation` carries the envelope metadata
/// (idempotency replay, entity seq).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AlertMutationResponse {
    pub alert: AlertView,
    pub mutation: instant::MutationMeta,
}

/// RLS-verified triage vocabulary — exactly the CHECK's three values.
fn parse_alert_status(s: &str) -> ApiResult<String> {
    match s.trim() {
        "open" | "acknowledged" | "resolved" => Ok(s.trim().to_string()),
        other => Err(ApiError::Validation(format!(
            "status must be one of open|acknowledged|resolved, got `{other}`"
        ))),
    }
}

/// POST /api/alerts/{id}/acknowledge — triage: open → acknowledged.
/// Instant-first (IF1/IF2): idempotency-keyed, read-your-write. Re-ack
/// (or ack of an already-resolved alert) is a typed conflict — the state
/// is honest, never silently rewritten.
#[utoipa::path(operation_id = "alerts_acknowledge", post, path = "/api/alerts/{id}/acknowledge", tag = "alerts", security(("bearer" = [])),
    params(("id" = Uuid, Path)), responses(
        (status = 200, body = AlertMutationResponse),
        (status = 404, description = "alert not visible in scope"),
        (status = 409, description = "alert already acknowledged or resolved"),
        (status = 422, description = "validation failed")))]
pub async fn acknowledge(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<instant::MutationParams>,
) -> ApiResult<Json<AlertMutationResponse>> {
    triage(state, extensions, id, req, Triage::Acknowledge).await
}

/// POST /api/alerts/{id}/resolve — triage: open/acknowledged → resolved.
/// Resolving straight from `open` leaves the ack fields NULL (no
/// acknowledgement ever happened — the row renders that honestly).
#[utoipa::path(operation_id = "alerts_resolve", post, path = "/api/alerts/{id}/resolve", tag = "alerts", security(("bearer" = [])),
    params(("id" = Uuid, Path)), responses(
        (status = 200, body = AlertMutationResponse),
        (status = 404, description = "alert not visible in scope"),
        (status = 409, description = "alert already resolved"),
        (status = 422, description = "validation failed")))]
pub async fn resolve(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<instant::MutationParams>,
) -> ApiResult<Json<AlertMutationResponse>> {
    triage(state, extensions, id, req, Triage::Resolve).await
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Triage {
    Acknowledge,
    Resolve,
}

impl Triage {
    fn entity_op(self) -> &'static str {
        match self {
            Self::Acknowledge => "acknowledge",
            Self::Resolve => "resolve",
        }
    }
}

async fn triage(
    state: AppState,
    extensions: Extensions,
    alert_id: Uuid,
    params: instant::MutationParams,
    action: Triage,
) -> ApiResult<Json<AlertMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: core_domain::policy::Module::Tickets,
            action: core_domain::policy::Action::Edit,
        },
    )?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::json!({ "action": action.entity_op() });
    let key = params.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "alert",
                entity_id: Some(alert_id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Load first so "gone" is a 404 and "already handled" is a typed
        // conflict — never a silent no-op.
        let before = core_db::alerting::get_alert(&mut *ctx.tx, org_id, alert_id)
            .await
            .map_err(ApiError::from)?;
        let already_err = |message: &'static str| ApiError::Conflict(message.to_string());
        let row = match action {
            Triage::Acknowledge => match before.status.as_str() {
                "open" => {
                    core_db::alerting::acknowledge_alert(&mut *ctx.tx, org_id, alert_id, user_id)
                        .await?
                        .expect("guard above proved status='open'")
                }
                "acknowledged" => return Err(already_err("alert is already acknowledged")),
                "resolved" => return Err(already_err("alert is already resolved")),
                _ => return Err(already_err("alert is not open")),
            },
            Triage::Resolve => match before.status.as_str() {
                "open" | "acknowledged" => {
                    core_db::alerting::resolve_alert(&mut *ctx.tx, org_id, alert_id, user_id)
                        .await?
                        .expect("the pre-read proved a live state")
                }
                "resolved" => return Err(already_err("alert is already resolved")),
                _ => return Err(already_err("alert is not live")),
            },
        };
        let view = AlertView::from(row.clone());
        match action {
            Triage::Acknowledge => {
                ctx.emit(
                    types::ALERT_ACKNOWLEDGED,
                    "alert",
                    Some(alert_id.to_string().as_str()),
                    &serde_json::to_value(core_domain::events::AlertAcknowledged {
                        alert_id,
                        org_id,
                        acknowledged_by: user_id,
                        acknowledged_at: chrono::Utc::now(),
                    })
                    .unwrap_or_default(),
                )
                .await?;
            }
            Triage::Resolve => {
                ctx.emit(
                    types::ALERT_RESOLVED,
                    "alert",
                    Some(alert_id.to_string().as_str()),
                    &serde_json::to_value(core_domain::events::AlertResolved {
                        alert_id,
                        org_id,
                        resolved_by: user_id,
                        resolved_at: chrono::Utc::now(),
                    })
                    .unwrap_or_default(),
                )
                .await?;
            }
        }
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id,
                actor: Actor::Human(user_id),
                action: format!("alert.{}", action.entity_op()),
                entity_type: Some("alert".into()),
                entity_id: Some(alert_id.to_string()),
                detail: serde_json::json!({
                    "from_status": before.status,
                    "to_status": row.status,
                    "dedup_key": row.dedup_key,
                    "severity": row.severity,
                }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "alert",
                entity_id: alert_id,
                version: params.version,
            },
        )
        .await?;
        let after = serde_json::to_value(view.clone()).unwrap_or(Value::Null);
        let now = chrono::Utc::now();
        let placeholder = AlertMutationResponse {
            alert: view.clone(),
            mutation: instant::MutationMeta {
                seq: write_seq,
                version: write_seq,
                updated_at: now,
                before: None,
                after: Value::Null,
                idempotency_key: key.clone(),
            },
        };
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "alert",
                entity_id: alert_id,
                op: action.entity_op(),
                seq: write_seq,
                before: Some(serde_json::json!({ "status": before.status })),
                after,
                updated_at: now,
                key: key.as_deref(),
                payload: &placeholder,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(AlertMutationResponse {
                alert: view,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// GET /api/alerts — newest-first ledger browse, filterable by source and
/// incident ticket (staff, ticket-view scoped).
#[utoipa::path(operation_id = "alerts_list", get, path = "/api/alerts", tag = "alerts", security(("bearer" = [])),
    params(AlertListQuery),
    responses((status = 200, body = AlertListResponse)))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<AlertListQuery>,
) -> ApiResult<Json<AlertListResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: core_domain::policy::Module::Tickets,
            action: core_domain::policy::Action::View,
        },
    )?;
    let status = match q.status.as_deref() {
        None => None,
        Some(raw) => Some(parse_alert_status(raw)?),
    };
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows = core_db::alerting::list_alerts(
        &mut *tx,
        authed.principal.user.org_id,
        AlertFilter {
            source_id: q.source_id,
            ticket_id: q.ticket_id,
            status,
            limit: q.limit,
            offset: q.offset,
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(AlertListResponse {
        alerts: rows.into_iter().map(Into::into).collect(),
    }))
}

// --- router builders -------------------------------------------------------------

/// The machine ingest route: mounted OUTSIDE the JWT middleware on both
/// generations (`/api/v1` canonical + `/api` legacy alias).
pub fn public_routes(gen: crate::routes::ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new().route(
        &format!("{base}/alerts/ingest/{{source_ref}}"),
        post(ingest),
    )
}

/// The staff surface: merged inside the protected router so the standard
/// `require_auth` middleware covers it.
pub fn admin_routes(gen: crate::routes::ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/alert-sources"),
            get(sources_list).post(sources_create),
        )
        .route(
            &format!("{base}/alert-sources/{{id}}"),
            patch(sources_patch),
        )
        .route(&format!("{base}/alerts"), get(list))
        .route(
            &format!("{base}/alerts/{{id}}/acknowledge"),
            post(acknowledge),
        )
        .route(&format!("{base}/alerts/{{id}}/resolve"), post(resolve))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_maps_onto_ticket_priorities() {
        assert_eq!(AlertSeverity::Critical.priority(), Priority::P1);
        assert_eq!(AlertSeverity::Warning.priority(), Priority::P2);
        assert_eq!(AlertSeverity::Info.priority(), Priority::P3);
        assert_eq!(
            AlertSeverity::parse("critical"),
            Some(AlertSeverity::Critical)
        );
        assert_eq!(
            AlertSeverity::parse("INFO"),
            None,
            "vocabulary is lowercase"
        );
        assert_eq!(AlertSeverity::Critical.as_str(), "critical");
    }

    #[test]
    fn absent_correlation_key_defaults_in_doc_roundtrip() {
        // The canonical ingest document survives a serde roundtrip; the
        // correlation defaulting happens server-side (handler), never in
        // the wire type.
        let doc: IngestAlertRequest = serde_json::from_value(serde_json::json!({
            "dedup_key": "evt-1",
            "title": "Disk critical",
            "severity": "critical",
            "attributes": { "disk": "C:" },
        }))
        .expect("canonical document parses");
        assert_eq!(doc.correlation_key, None);
        assert_eq!(doc.severity, AlertSeverity::Critical);
        assert!(doc.attributes.is_object());
    }

    #[test]
    fn validation_rejects_missing_identity_or_title() {
        let bad = serde_json::from_value::<IngestAlertRequest>(serde_json::json!({
            "title": "no dedup key",
            "severity": "info",
        }));
        assert!(bad.is_err(), "dedup_key is required at the serde layer");
        let req = IngestAlertRequest {
            dedup_key: "evt-1".into(),
            title: "   ".into(),
            description: None,
            severity: AlertSeverity::Info,
            correlation_key: None,
            occurred_at: None,
            attributes: Value::Null,
        };
        let err = validate_ingest(&req).unwrap_err();
        assert!(err.to_string().contains("title is required"));
    }
}
