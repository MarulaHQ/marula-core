//! Currency & FX (ADR 0002, Wave 12 lane 10): the rate book surface, the
//! manual-entry built-in provider, and the shared snapshot lock helper
//! the five lock-moment write paths call.
//!
//! Doctrine: conversion is EXPLICIT and fail-closed. A cross-currency
//! fact whose rate is missing fails the write with a typed validation
//! error naming the missing pair — the core never implicitly converts,
//! never defaults to 1:1, and never calls a rate service (manual entry is
//! the standalone "none" mode; managed feeds are bundles on the
//! [`core_domain::fx::FxRateProvider`] trait).

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::routes::ApiGeneration;
use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::Extensions,
    routing::get,
    Json, Router,
};
use chrono::NaiveDate;
use core_domain::entities::{FxRateEntry, FxReconciliationRow};
use core_domain::fx::{FxError, FxRate, FxRateProvider, FxRateRequest, FxSnapshot};
use core_domain::money::{CurrencyCode, Money};
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

fn commercial(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Commercial,
        action,
    }
}

const LIST_LIMIT: i64 = 500;

pub fn routes(gen: ApiGeneration) -> Router<AppState> {
    let base = gen.base();
    Router::new()
        .route(
            &format!("{base}/fx/rates"),
            get(list_rates).post(record_rate),
        )
        .route(&format!("{base}/fx/rates/resolve"), get(resolve_rate))
        .route(&format!("{base}/fx/reconciliation"), get(reconciliation))
}

// --- the built-in provider (FX6) ---------------------------------------------

/// The manual-entry built-in provider: resolves the effective rate from
/// the org's append-only `fx_rates` book (core-db), the standalone "none"
/// mode. Zero network calls. Managed daily feeds (ECB/OFX-class) are
/// Relay/community bundles implementing the same trait.
#[derive(Debug, Clone)]
pub struct ManualFxRateProvider {
    pool: PgPool,
}

impl ManualFxRateProvider {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl FxRateProvider for ManualFxRateProvider {
    fn name(&self) -> &str {
        "manual"
    }

    async fn resolve(&self, req: &FxRateRequest) -> Result<FxRate, FxError> {
        let mut tx = self.pool.begin().await.map_err(|e| FxError::Backend {
            reason: e.to_string(),
        })?;
        core_db::set_org(&mut tx, req.org_id)
            .await
            .map_err(|e| FxError::Backend {
                reason: e.to_string(),
            })?;
        let entry = core_db::fx::resolve_rate(
            &mut tx,
            req.org_id,
            req.from.as_str(),
            req.to.as_str(),
            req.as_of,
        )
        .await
        .map_err(|e| FxError::Backend {
            reason: e.to_string(),
        })?;
        tx.commit().await.map_err(|e| FxError::Backend {
            reason: e.to_string(),
        })?;
        let entry = entry.ok_or_else(|| FxError::NoRate {
            from: req.from.to_string(),
            to: req.to.to_string(),
            as_of: req.as_of,
        })?;
        // Stored pair (base, quote, rate) = "1 base buys rate quote".
        // Directed coverage matches (from, to) directly; inverse coverage
        // (stored to→from) inverts through the typed operation.
        let rate = if entry.base_currency == req.from.as_str() {
            FxRate::new(req.from.clone(), req.to.clone(), &entry.rate)
        } else {
            FxRate::new(req.to.clone(), req.from.clone(), &entry.rate).map(|r| r.inverse())
        }
        .map_err(|e| FxError::Backend {
            reason: e.to_string(),
        })?;
        Ok(rate)
    }
}

// --- shared lock helper (the five lock moments call this) --------------------

/// A locked conversion snapshot ready to persist into the uniform column
/// group (`fx_rate_used numeric(24,12)`, `fx_rate_date`, `fx_source`,
/// base amount).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LockedSnapshot {
    pub rate_used: String,
    pub rate_date: NaiveDate,
    pub source: String,
    pub base_amount_minor: i64,
}

/// Resolves the effective rate for `currency → org base` on `as_of` and
/// locks the immutable snapshot. `Ok(None)` when the fact is already in
/// the org base currency (no snapshot needed). Missing rate coverage ⇒
/// typed 422 naming the pair — fail-closed manual mode.
pub(crate) async fn lock_snapshot(
    pool: &PgPool,
    org_id: Uuid,
    amount_minor: i64,
    currency: &str,
    as_of: NaiveDate,
) -> ApiResult<Option<LockedSnapshot>> {
    let mut tx = pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let base = core_db::fx::base_currency(&mut *tx, org_id)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    if currency == base {
        return Ok(None);
    }
    let provider = ManualFxRateProvider::new(pool.clone());
    let req = FxRateRequest {
        org_id,
        from: CurrencyCode::new(currency)
            .map_err(|_| ApiError::Validation(format!("invalid currency code: {currency}")))?,
        to: CurrencyCode::new(&base)
            .map_err(|_| ApiError::Validation(format!("invalid base currency code: {base}")))?,
        as_of,
    };
    let rate = provider.resolve(&req).await.map_err(|e| match e {
        FxError::NoRate { from, to, as_of } => ApiError::Validation(format!(
            "no fx rate recorded for {from}→{to} effective {as_of}; \
             record one before entering this cross-currency fact"
        )),
        other => ApiError::Internal(anyhow::anyhow!("fx provider error: {other}")),
    })?;
    let original = Money::new(amount_minor, req.from.clone());
    let snapshot = FxSnapshot::lock(&original, &req.to, rate, as_of, provider.name())
        .map_err(|e| ApiError::Validation(e.to_string()))?;
    let (rate_used, rate_date, source, base_amount_minor) = snapshot.columns();
    Ok(Some(LockedSnapshot {
        rate_used,
        rate_date,
        source: source.to_string(),
        base_amount_minor,
    }))
}

// --- wire structs ------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RecordRateRequest {
    /// 1 base major unit buys `rate` quote major units.
    pub base_currency: String,
    pub quote_currency: String,
    #[schema(value_type = String)]
    pub as_of_date: NaiveDate,
    /// Positive decimal, max 12 fractional digits ("17.85").
    pub rate: String,
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RateListQuery {
    pub base_currency: Option<String>,
    pub quote_currency: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ResolveRateQuery {
    pub from: String,
    pub to: String,
    #[schema(value_type = String)]
    pub as_of: Option<NaiveDate>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RateMutationResponse {
    #[serde(flatten)]
    pub rate: FxRateEntry,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ReconciliationQuery {
    pub limit: Option<i64>,
}

fn rate_json(entry: &FxRateEntry) -> serde_json::Value {
    serde_json::to_value(entry).unwrap_or(serde_json::json!({}))
}

// --- handlers ----------------------------------------------------------------

/// GET /api/fx/rates — the org's append-only rate book, newest first.
#[utoipa::path(operation_id = "fx_rates_list", get, path = "/api/fx/rates", tag = "fx",
    security(("bearer" = [])), params(("base_currency" = Option<String>, Query), ("quote_currency" = Option<String>, Query)),
    responses((status = 200, body = [FxRateEntry])))]
pub async fn list_rates(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<RateListQuery>,
) -> ApiResult<Json<Vec<FxRateEntry>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let rates = core_db::fx::list_rates(
        &mut *tx,
        org_id,
        q.base_currency.as_deref(),
        q.quote_currency.as_deref(),
        LIST_LIMIT,
    )
    .await
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rates))
}

/// POST /api/fx/rates — manual entry (the built-in provider's write path).
/// Append-only: corrections are new rows; the effective-rate resolver
/// always picks the newest capture on-or-before the requested date.
#[utoipa::path(operation_id = "fx_rates_record", post, path = "/api/fx/rates", tag = "fx",
    security(("bearer" = [])), request_body = RecordRateRequest,
    responses((status = 200, body = RateMutationResponse), (status = 422)))]
pub async fn record_rate(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<RecordRateRequest>,
) -> ApiResult<Json<RateMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::Create))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let base = CurrencyCode::new(&req.base_currency)
        .map_err(|_| ApiError::Validation("base_currency must be an ISO 4217 code".into()))?;
    let quote = CurrencyCode::new(&req.quote_currency)
        .map_err(|_| ApiError::Validation("quote_currency must be an ISO 4217 code".into()))?;
    if base == quote {
        return Err(ApiError::Validation(
            "base_currency and quote_currency must differ".into(),
        ));
    }
    // Shape-validate the rate through the domain type (≤12 fractional
    // digits, strictly positive) — the same parser that reads it back.
    let rate = FxRate::new(base.clone(), quote.clone(), req.rate.trim())
        .map_err(|e| ApiError::Validation(e.to_string()))?;

    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "fx_rate",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let entry = core_db::fx::insert_rate(
            &mut *ctx.tx,
            org_id,
            base.as_str(),
            quote.as_str(),
            req.as_of_date,
            &rate.db_text(),
            "manual",
            Some(user_id),
        )
        .await
        .map_err(ApiError::from)?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "fx_rate",
                entity_id: entry.id,
                version: req.instant.version,
            },
        )
        .await?;
        ctx.emit(
            core_domain::events::types::FX_RATE_RECORDED,
            "fx_rate",
            Some(entry.id.to_string().as_str()),
            &serde_json::to_value(core_domain::events::FxRateRecorded {
                rate_id: entry.id,
                org_id,
                base_currency: entry.base_currency.clone(),
                quote_currency: entry.quote_currency.clone(),
                as_of_date: entry.as_of_date,
                rate: entry.rate.clone(),
                source: entry.source.clone(),
                captured_at: entry.captured_at,
            })
            .unwrap_or_default(),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &core_domain::entities::AuditEvent {
                id: Uuid::now_v7(),
                org_id: ctx.org_id,
                actor: core_domain::entities::Actor::Human(user_id),
                action: "fx.rate_recorded".to_string(),
                entity_type: Some("fx_rate".to_string()),
                entity_id: Some(entry.id.to_string()),
                detail: rate_json(&entry),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await
        .map_err(ApiError::from)?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "fx_rate",
                entity_id: entry.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: rate_json(&entry),
                updated_at: entry.captured_at,
                key: key.as_deref(),
                payload: &entry,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(RateMutationResponse {
                rate: entry,
                mutation,
            }),
        ))
    })
    .await?;
    Ok(Json(outcome.into_response()?))
}

/// GET /api/fx/rates/resolve — the effective rate for one direction on
/// one date (defaults to today). Read-only; mirrors the resolver the
/// lock-moment write paths use.
#[utoipa::path(operation_id = "fx_rates_resolve", get, path = "/api/fx/rates/resolve", tag = "fx",
    security(("bearer" = [])), params(("from" = String, Query), ("to" = String, Query), ("as_of" = Option<String>, Query)),
    responses((status = 200, body = FxRateEntry), (status = 404)))]
pub async fn resolve_rate(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ResolveRateQuery>,
) -> ApiResult<Json<FxRateEntry>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let from = CurrencyCode::new(&q.from)
        .map_err(|_| ApiError::Validation("from must be an ISO 4217 code".into()))?;
    let to = CurrencyCode::new(&q.to)
        .map_err(|_| ApiError::Validation("to must be an ISO 4217 code".into()))?;
    if from == to {
        return Err(ApiError::Validation("from and to must differ".into()));
    }
    let as_of = q.as_of.unwrap_or_else(|| chrono::Utc::now().date_naive());
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let entry = core_db::fx::resolve_rate(&mut tx, org_id, from.as_str(), to.as_str(), as_of)
        .await
        .map_err(ApiError::from)?
        .ok_or(ApiError::NotFound)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(entry))
}

/// GET /api/fx/reconciliation — the FX deliverable to the books (FX7):
/// original amount, rate, source/date, base equivalent, sync target. The
/// core records conversions that happened; realized/unrealized gain-loss
/// stays in the accounting system.
#[utoipa::path(operation_id = "fx_reconciliation", get, path = "/api/fx/reconciliation", tag = "fx",
    security(("bearer" = [])), params(("limit" = Option<i64>, Query)),
    responses((status = 200, body = [FxReconciliationRow])))]
pub async fn reconciliation(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ReconciliationQuery>,
) -> ApiResult<Json<Vec<FxReconciliationRow>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, commercial(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let limit = q.limit.unwrap_or(LIST_LIMIT).clamp(1, LIST_LIMIT);
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let rows = core_db::fx::reconciliation(&mut *tx, org_id, limit)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(rows))
}
