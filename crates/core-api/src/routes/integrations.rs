//! The ecosystem integrations surface (Wave 22): Slack / Teams / Rewst /
//! VoIP over the EXISTING `connector_installations` consent seam + the
//! distributor pricing-feeds ingest.
//!
//! Posture:
//! * installation = owner/admin CONSENT (the table is the consent
//!   store): re-configuring replaces config + scopes atomically,
//!   removal is typed 404 when absent;
//! * config is SEALED at rest (same crypto posture as webhook secrets)
//!   and NEVER returned once installed — responses carry presence only;
//! * the ADDRESS RULE runs at both ends: a `webhook_url` is screened with
//!   `core_domain::egress` before it is sealed and again before a send, so a
//!   consent can never become a request to the host this API runs on (the
//!   dev-gated loopback allowance is the only exception, and only a debug
//!   build with `MARULA_DEV_TOOLS=1` holds it);
//! * test pings go through the app-bridges adapters with the UNSEALED
//!   secret held only in the request scope (never logged, never persisted);
//! * the VoIP surface is machine ingest THROUGH THE SAME consents —
//!   call rows are idempotent per call id;
//! * pricing feeds land as immutable batches, apply = typed overlay of
//!   `catalog_items` by lowercase SKU; unmatched SKUs leave a typed
//!   outcome, never a silent partial.
use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::Value;
use uuid::Uuid;

/// Slack/Teams/Rewst/VoIP — typed kinds; anything else is a validation
/// refusal, never an install.
fn parse_kind(kind: &str) -> ApiResult<&'static str> {
    match kind.trim().to_lowercase().as_str() {
        "slack" => Ok("slack"),
        "teams" => Ok("teams"),
        "rewst" => Ok("rewst"),
        "voip" => Ok("voip"),
        _ => Err(ApiError::Validation(
            "unknown connector kind (typed kinds: slack, teams, rewst, voip)".into(),
        )),
    }
}

/// The fixed capability scopes per connector kind (what the consent
/// grants — the rest stays deny-by-default).
fn default_scopes(kind: &str) -> Vec<String> {
    match kind {
        // Outbound notifications only.
        "slack" | "teams" => vec!["notifications.forward".into()],
        // Workflow triggering + secret header auth.
        "rewst" => vec!["workflows.trigger".into()],
        // Machine ingestion of call logs.
        "voip" => vec!["calls.ingest".into()],
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Installations (consent store seam).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ConnectorInstallRequest {
    /// Provider configuration document — WHOLESALE sealed (never
    /// logged, never echoed). The adapters expect, per kind:
    /// slack/teams `{ "webhook_url": "https://…" }`,
    /// rewst `{ "webhook_url": "https://…", "secret": "…" }`,
    /// voip `{ "access_token": "…", "provider": "…" }`.
    pub config: Value,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ConnectorIndexResponse {
    /// Installed connectors (typed kinds only, presence summaries).
    pub installed: Vec<ConnectorInstallationView>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ConnectorInstallationView {
    pub kind: String,
    pub capability_scopes: Vec<String>,
    #[schema(value_type = Option<String>, format = "date-time")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// GET /api/v1/connectors — the org's installed bridges (presence only).
#[utoipa::path(operation_id = "connectors_index", get,
    path = "/api/connectors", tag = "integrations",
    security(("bearer" = [])),
    responses((status = 200, body = ConnectorIndexResponse)))]
pub async fn index(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<ConnectorIndexResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let rows = core_db::webhooks::list_connector_installations(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    let installed = rows
        .into_iter()
        .map(|r| ConnectorInstallationView {
            kind: r.connector_type,
            capability_scopes: r.capability_scopes,
            updated_at: r.updated_at,
        })
        .collect();
    Ok(Json(ConnectorIndexResponse { installed }))
}

/// POST /api/v1/connectors/{kind} — the admin installs / re-consents.
#[utoipa::path(operation_id = "connectors_install", post,
    path = "/api/connectors/{kind}", tag = "integrations",
    security(("bearer" = [])), params(("kind" = String, Path)),
    request_body = ConnectorInstallRequest,
    responses((status = 200, body = ConnectorInstallationView), (status = 422)))]
pub async fn install(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(kind): Path<String>,
    Json(req): Json<ConnectorInstallRequest>,
) -> ApiResult<Json<ConnectorInstallationView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Commercial,
            action: Action::Edit,
        },
    )?;
    if !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "connector consent is owner/admin".into(),
        ));
    }
    let kind = parse_kind(&kind)?;
    // Config pre-validation AT THE BOUNDARY: empty bodies are refusals —
    // never a half-configured consent (slack/teams/rewst validate URL
    // + secret at adapter build time below; the seal keeps whatever the
    // admin sent, but an obviously-empty config is refused early).
    if req.config.get("config").is_some() {
        return Err(ApiError::Validation(
            "send the config fields directly (no nested `config` envelope)".into(),
        ));
    }
    if req.config.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        return Err(ApiError::Validation(
            "the connector configuration cannot be empty".into(),
        ));
    }
    // The address rule at the write boundary, the same one `/api/webhooks`
    // asks: a webhook URL that names loopback, a link-local or metadata
    // address, or a private range is refused before it is sealed, so a stored
    // consent can never become a request to the host this API runs on. The
    // dev-gated loopback allowance is the only way a loopback destination gets
    // through, and only a debug build with MARULA_DEV_TOOLS=1 holds it.
    if let Some(url) = req.config.get("webhook_url").and_then(Value::as_str) {
        crate::outbound::guard_under(url, state.config.egress_policy())
            .await
            .map_err(|denial| ApiError::Validation(format!("webhook_url refused: {denial}")))?;
    }
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let scopes = default_scopes(kind);
    let scopes_for_request = scopes.clone();
    let sealed = crate::crypto::seal(
        &state.config.secret_key,
        serde_json::to_vec(&req.config)
            .map_err(|e| ApiError::Validation(format!("unreadable config: {e}")))?
            .as_slice(),
    )
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("sealing failed: {e}")))?;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        core_db::webhooks::upsert_connector_installation(
            &mut *tx.tx,
            Uuid::now_v7(),
            org_id,
            kind,
            &sealed,
            scopes_for_request.as_slice(),
            actor,
        )
        .await?;
        Ok((tx, ()))
    })
    .await?;
    Ok(Json(ConnectorInstallationView {
        kind: kind.into(),
        capability_scopes: scopes.clone(),
        updated_at: chrono::Utc::now(),
    }))
}

/// DELETE /api/v1/connectors/{kind} — withdraws the consent.
#[utoipa::path(operation_id = "connectors_remove", delete,
    path = "/api/connectors/{kind}", tag = "integrations",
    security(("bearer" = [])), params(("kind" = String, Path)),
    responses((status = 200), (status = 404)))]
pub async fn remove(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(kind): Path<String>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Time,
            action: Action::View,
        },
    )?;
    if !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "connector consent is owner/admin".into(),
        ));
    }
    let kind = parse_kind(&kind)?;
    let org_id = authed.principal.user.org_id;
    org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut tx| async move {
            core_db::webhooks::delete_connector_installation(&mut *tx.tx, org_id, kind).await?;
            Ok((tx, ()))
        },
    )
    .await?;
    Ok(Json(json!({ "removed": true })))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TestResult {
    pub kind: String,
    /// `delivered` | `refused` | `bad_config` | `unreachable`.
    pub outcome: String,
    #[schema(value_type = Option<String>)]
    pub detail: Option<String>,
}

/// POST /api/v1/connectors/{kind}/test — a provisioned canary through
/// the adapter (slack/teams notifications; rewst trigger).
#[utoipa::path(operation_id = "connectors_test", post,
    path = "/api/connectors/{kind}/test", tag = "integrations",
    security(("bearer" = [])), params(("kind" = String, Path)),
    responses((status = 200, body = TestResult), (status = 404)))]
pub async fn test(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(kind): Path<String>,
) -> ApiResult<Json<TestResult>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Time,
            action: Action::View,
        },
    )?;
    if !authed.principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "connector tests are owner/admin".into(),
        ));
    }
    let kind = parse_kind(&kind)?;
    let org_id = authed.principal.user.org_id;
    let config = {
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org(&mut tx, org_id).await?;
        let got = core_db::integrations::get_connector_installation(&mut tx, org_id, kind).await;
        match got {
            Ok((_id, sealed)) => {
                tx.commit().await.map_err(ApiError::from)?;
                crate::crypto::open(&state.config.secret_key, &sealed).map_err(|_| {
                    ApiError::Unavailable("the sealed connector config failed to decrypt".into())
                })?
            }
            Err(core_db::DbError::NotFound) => return Err(ApiError::NotFound),
            Err(e) => return Err(e.into()),
        }
    };
    let cfg: Value = serde_json::from_slice(&config)
        .map_err(|_| ApiError::Unavailable("the sealed connector config is unreadable".into()))?;
    // The send-side half of the rule the install boundary applies. A row
    // written before that guard existed — or by another generation — must not
    // become a request to the host this API runs on, and a name's answer today
    // is not the answer it gave when the row was written, so the address is
    // judged again here and the destination is never trusted from storage.
    let policy = state.config.egress_policy();
    // Screen once and KEEP what the guard returned. Its client is pinned to the
    // addresses it judged and follows no redirects, so the adapter dials exactly
    // the destination this check approved; discarding it would let a name answer
    // publicly here and privately a moment later, and would let a 3xx choose a
    // second destination nothing screened.
    let guarded =
        match cfg.get("webhook_url").and_then(Value::as_str) {
            Some(url) => Some(crate::outbound::guard_under(url, policy).await.map_err(
                |denial| ApiError::Validation(format!("webhook_url refused: {denial}")),
            )?),
            None => None,
        };
    // The guard's own serialization, not the row's string: it is what was
    // judged, so a parser differential cannot authorize one URL and dial another.
    let url = guarded
        .as_ref()
        .map(|approved| approved.url().to_string())
        .unwrap_or_default();
    let pinned_client = || -> ApiResult<reqwest::Client> {
        guarded
            .as_ref()
            .ok_or_else(|| ApiError::Validation("webhook_url is required".into()))?
            .client(std::time::Duration::from_secs(20))
            .map_err(|denial| ApiError::Validation(format!("webhook_url refused: {denial}")))
    };
    let payload = app_bridges::NotificationPayload {
        title: "Marula test".into(),
        body: "A provisioned test ping from the Marula integrations surface.".into(),
        link_url: None,
    };
    match kind {
        "slack" | "teams" => {
            let outcome = match kind {
                "slack" => {
                    let connector =
                        app_bridges::slack::SlackConnector::with_http_client(url, pinned_client()?);
                    let connector = if policy.is_dev_loopback() {
                        connector.allow_loopback()
                    } else {
                        connector
                    };
                    connector.send(&payload).await
                }
                _ => {
                    let connector =
                        app_bridges::teams::TeamsConnector::with_http_client(url, pinned_client()?);
                    let connector = if policy.is_dev_loopback() {
                        connector.allow_loopback()
                    } else {
                        connector
                    };
                    connector.send(&payload).await
                }
            };
            Ok(Json(to_test(kind, outcome)))
        }
        "rewst" => {
            let secret = cfg
                .get("secret")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let trigger = app_bridges::rewst::RewstTriggerPayload {
                org: cfg
                    .get("org")
                    .and_then(|v| v.as_str())
                    .unwrap_or("marula")
                    .to_string(),
                event: "connector.test".into(),
                notification: payload,
                entity_id: None,
            };
            let bridge =
                app_bridges::rewst::RewstBridge::with_http_client(url, secret, pinned_client()?);
            let bridge = if policy.is_dev_loopback() {
                bridge.allow_loopback()
            } else {
                bridge
            };
            let outcome = bridge.trigger(&trigger).await;
            Ok(Json(to_test(kind, outcome)))
        }
        _ => Err(ApiError::Validation(
            "the voip connector has no test ping — it is an ingest surface (POST /api/connectors/voip/call-logs)".into(),
        )),
    }
}

pub(crate) fn to_test(kind: &str, outcome: Result<(), app_bridges::BridgeError>) -> TestResult {
    match outcome {
        Ok(()) => TestResult {
            kind: kind.into(),
            outcome: "delivered".into(),
            detail: None,
        },
        Err(e) => {
            let (reason, detail) = match e {
                app_bridges::BridgeError::Refused(_, d) => ("refused", Some(d)),
                app_bridges::BridgeError::BadConfig(kind, m) => {
                    ("bad_config", Some(format!("{kind}: {m}")))
                }
                app_bridges::BridgeError::Unreachable(_, m) => ("unreachable", Some(m)),
            };
            TestResult {
                kind: kind.into(),
                outcome: reason.into(),
                detail,
            }
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TestOutcome {
    pub kind: String,
    /// `delivered` | `refused` | `bad_config` | `unreachable`.
    pub outcome: String,
    #[schema(value_type = Option<String>)]
    pub detail: Option<String>,
}

/// POST /api/v1/connectors/voip/call-logs — machine ingest of one call
/// record (idempotent per provider call id). Requires the consents +
/// the `calls.ingest` scope.
#[utoipa::path(operation_id = "connectors_voip_ingest", post,
    path = "/api/connectors/voip/call-logs", tag = "integrations",
    security(("bearer" = [])),
    request_body = VoipCallLogRequest,
    responses((status = 200, body = VoipCallLogResponse), (status = 404), (status = 409)))]
pub async fn voip_ingest(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<VoipCallLogRequest>,
) -> ApiResult<Json<VoipCallLogResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Tickets,
            action: Action::View,
        },
    )?;
    let org_id = authed.principal.user.org_id;
    if req.direction != "in" && req.direction != "out" {
        return Err(ApiError::Validation(
            "direction must be 'in' or 'out'".into(),
        ));
    }
    if req.external_number.trim().is_empty() {
        return Err(ApiError::Validation("external_number is required".into()));
    }
    if req.duration_seconds < 0 {
        return Err(ApiError::Validation("duration_seconds must be ≥ 0".into()));
    }
    let inserted_id = org_tx::with_org_tx(
        &state,
        org_id,
        authed.principal.user.id,
        |mut tx| async move {
            let (installation_id, _sealed) =
                core_db::integrations::get_connector_installation(&mut tx.tx, org_id, "voip")
                    .await
                    .map_err(ApiError::from)?;
            let model = core_db::integrations::VoipCallLog {
                id: Uuid::now_v7(),
                org_id,
                installation_id,
                call_id: Some(req.call_id.clone()),
                direction: if req.direction == "out" { "out" } else { "in" }.into(),
                external_number: req.external_number.clone(),
                duration_seconds: req.duration_seconds,
                summary: req.summary.clone().unwrap_or_default(),
                linked_ticket_id: req.ticket_id,
                recorded_at: chrono::Utc::now(),
                occurred_at: req.occurred_at,
            };
            core_db::integrations::insert_voip_call_log(&mut tx.tx, &model).await?;
            Ok((tx, model.id))
        },
    )
    .await?;
    Ok(Json(VoipCallLogResponse { id: inserted_id }))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct VoipCallLogRequest {
    /// Provider call id (idempotency — required).
    pub call_id: String,
    /// `in` | `out`.
    pub direction: String,
    pub external_number: String,
    pub duration_seconds: i32,
    #[schema(value_type = Option<String>)]
    pub summary: Option<String>,
    #[schema(value_type = Option<uuid::Uuid>)]
    pub ticket_id: Option<Uuid>,
    #[schema(value_type = String, format = "date-time")]
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct VoipCallLogResponse {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
}

// --- distributor pricing feeds -------------------------------------------------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PricingFeedBatchRequest {
    pub source: String,
    #[schema(value_type = Option<String>)]
    pub note: Option<String>,
    pub entries: Vec<PricingFeedEntryRequest>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct PricingFeedEntryRequest {
    pub sku: String,
    pub currency: String,
    pub unit_price_minor: i64,
    pub cost_minor: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PricingFeedApplyResponse {
    #[schema(value_type = uuid::Uuid)]
    pub batch_id: Uuid,
    pub matched: i64,
    pub unmatched: i64,
    pub unmatched_skus: Vec<String>,
    pub outcomes: Vec<PricingFeedOutcomeView>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PricingFeedOutcomeView {
    pub sku: String,
    pub matched: bool,
    pub reason: String,
}

/// POST /api/v1/catalog/pricing-feeds — upload one immutable distributor
/// feed batch; admin/owner; then `/{batch_id}/apply` overlays pricing by
/// lowercase SKU.
#[utoipa::path(operation_id = "pricing_feed_create", post,
    path = "/api/catalog/pricing-feeds", tag = "integrations",
    security(("bearer" = [])),
    request_body = PricingFeedBatchRequest,
    responses((status = 200, body = PricingFeedBatchResponse), (status = 422)))]
pub async fn create_feed(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PricingFeedBatchRequest>,
) -> ApiResult<Json<PricingFeedBatchResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Commercial,
            action: Action::Edit,
        },
    )?;
    if req.entries.is_empty() || req.entries.len() > 5000 {
        return Err(ApiError::Validation(
            "a batch needs 1..=5000 entries".into(),
        ));
    }
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    let batch_id = Uuid::now_v7();
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        core_db::integrations::create_pricing_feed_batch(
            &mut tx.tx,
            batch_id,
            org_id,
            &req.source,
            req.note.as_deref().unwrap_or_default(),
            Some(actor),
            chrono::Utc::now(),
        )
        .await?;
        core_db::integrations::insert_pricing_feed_entries(
            &mut tx.tx,
            batch_id,
            org_id,
            req.entries
                .iter()
                .map(|e| core_db::integrations::PricingFeedEntry {
                    sku: Some(e.sku.clone()),
                    currency: e.currency.clone(),
                    unit_price_minor: e.unit_price_minor,
                    cost_minor: e.cost_minor,
                })
                .collect::<Vec<_>>()
                .as_slice(),
        )
        .await?;
        Ok((tx, batch_id))
    })
    .await?;
    Ok(Json(PricingFeedBatchResponse { batch_id }))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PricingFeedBatchResponse {
    #[schema(value_type = uuid::Uuid)]
    pub batch_id: Uuid,
}

/// POST /api/v1/catalog/pricing-feeds/{batch_id}/apply
#[utoipa::path(operation_id = "pricing_feed_apply", post,
    path = "/api/catalog/pricing-feeds/{batch_id}/apply", tag = "integrations",
    security(("bearer" = [])), params(("batch_id" = Uuid, Path)),
    responses((status = 200, body = PricingFeedApplyResponse), (status = 404), (status = 409)))]
pub async fn apply_feed(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(batch_id): Path<Uuid>,
) -> ApiResult<Json<PricingFeedApplyResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: Module::Commercial,
            action: Action::Edit,
        },
    )?;
    let actor = authed.principal.user.id;
    let org_id = authed.principal.user.org_id;
    org_tx::with_org_tx(&state, org_id, actor, |mut tx| async move {
        core_db::integrations::apply_pricing_feed_batch(
            &mut tx.tx,
            org_id,
            batch_id,
            Some(actor),
            chrono::Utc::now(),
        )
        .await
        .map_err(ApiError::from)?;
        Ok((tx, ()))
    })
    .await?;
    let outcomes = {
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org(&mut tx, org_id).await?;
        let rows =
            core_db::integrations::list_pricing_feed_outcomes(&mut *tx, org_id, batch_id).await?;
        tx.commit().await.map_err(ApiError::from)?;
        rows
    };
    let unmatched_skus: Vec<String> = outcomes
        .iter()
        .filter(|o| !o.matched)
        .map(|o| o.sku.clone().unwrap_or_default())
        .collect();
    let matched_count = outcomes.iter().filter(|o| o.matched).count() as i64;
    Ok(Json(PricingFeedApplyResponse {
        batch_id,
        matched: matched_count,
        unmatched: outcomes.len() as i64 - matched_count,
        unmatched_skus: unmatched_skus.clone(),
        outcomes: outcomes
            .into_iter()
            .map(|o| PricingFeedOutcomeView {
                sku: o.sku.clone().unwrap_or_default(),
                matched: o.matched,
                reason: o.reason,
            })
            .collect(),
    }))
}
