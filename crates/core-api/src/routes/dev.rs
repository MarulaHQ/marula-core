//! Dev-tools surface (the private build tracker DX2, QA reset loops).
//!
//! Purpose: runtime-gated machine endpoints that make QA loops fast —
//! seed/reseed the demo org (DX1), wipe+reseed, shift SLA clocks for breach
//! testing, and dump recent outbox envelopes.
//!
//! Gating contract (hard rule): the router is mounted ONLY when
//! `MARULA_DEV_TOOLS=1` (see `build_router`). Unset, these routes DO NOT
//! EXIST — no registration at all, not a 403 — and they never appear in
//! OpenAPI/the SDK (same undocumented-surface posture as /internal). No
//! bearer auth: the gate is the environment; dev stacks bind the API to a
//! trusted loopback/compose network.
//!
//! Provenance: every mutating handler appends an `audit_events` row as
//! `Actor::System` (action `dev.*`), and the seeder itself trails each
//! domain-entity write as `actor_kind = 'system'` through the IF9
//! write-trail seam (core-db::demo::trail_system_write).
//!
//! Non-purpose: not a production ops surface (no auth hardening, no rate
//! limiting beyond scope), not a data-export path, and never a default-on
//! capability.

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Query, State},
    Json,
};
use core_domain::entities::AuditEvent;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

/// Mounts the /api/dev routes. `build_router` calls this ONLY when
/// `config.dev_tools` is true — the function exists so the gate reads as
/// one visible branch in routes/mod.rs.
pub fn router(state: AppState) -> axum::Router<AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/api/dev/status", get(status))
        .route("/api/dev/seed", post(seed))
        .route("/api/dev/reset-org", post(reset_org))
        .route("/api/dev/time-travel", post(time_travel))
        .route("/api/dev/events", get(events))
        .with_state(state)
}

/// Appends one system-actor audit row for a dev action. Runs after the
/// action committed: the audit trail records outcomes, never shadows them.
async fn audit_system(state: &AppState, action: &str, detail: serde_json::Value) {
    let event = AuditEvent {
        id: Uuid::now_v7(),
        org_id: core_db::demo::DEMO_ORG_ID,
        actor: core_domain::entities::Actor::System,
        action: action.to_string(),
        entity_type: Some("org".into()),
        entity_id: Some(core_db::demo::DEMO_ORG_ID.to_string()),
        detail,
        ip: None,
        occurred_at: chrono::Utc::now(),
    };
    // Best-effort: a failed audit row must not mask the action's result,
    // but it IS loud — the audit trail is the security authority (§6).
    if let Err(e) = core_db::audit::insert(&state.pool, &event).await {
        tracing::error!("dev audit insert failed ({action}): {e}");
    }
}

/// GET /api/dev/status — build info + demo-org state for the ops page and
/// the in-app gating probe. Cheap by design: no mutation, one tiny read.
pub async fn status(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    let seeded = core_db::demo::is_seeded(&state.pool).await.unwrap_or(false);
    Ok(Json(json!({
        "enabled": true,
        "git_sha": option_env!("MARULA_GIT_SHA").unwrap_or("dev"),
        "migration_head": core_db::migration_head(),
        "demo_seeded": seeded,
        "demo_org_id": core_db::demo::DEMO_ORG_ID,
        "demo_slug": core_db::demo::DEMO_ORG_SLUG,
    })))
}

#[derive(Debug, Deserialize)]
pub struct TimeTravelRequest {
    /// How many hours to shift the SLA clocks back (1–720).
    pub hours: i64,
}

/// The local object-store directory the demo's attachment bytes belong in, or
/// `None` when this instance is not on a local store. On S3 the seeder writes
/// no bytes (it has no credentials): the rows still land, and the frontend's
/// type badges and size labels render; only the byte fetch reports a missing
/// object instead of opening a file.
fn demo_object_store_dir(state: &AppState) -> Option<std::path::PathBuf> {
    match &state.config.object_store {
        core_store::StoreChoice::Local(dir) => Some(std::path::PathBuf::from(dir)),
        core_store::StoreChoice::S3(_) => None,
    }
}

/// POST /api/dev/seed — runs the DX1 seeder. 409 when the demo org already
/// exists unless `?force=1`, which wipes + reseeds instead.
pub async fn seed(
    State(state): State<AppState>,
    Query(params): Query<SeedQuery>,
) -> ApiResult<Json<core_db::demo::DemoSeedReport>> {
    let seal_opts = core_db::demo::DemoSeedOptions {
        seal_key: state.config.secret_key.0,
        object_store_dir: demo_object_store_dir(&state),
    };
    let force = params.force.unwrap_or(false);
    let result = if force {
        core_db::demo::reset(&state.pool, &seal_opts).await
    } else {
        core_db::demo::seed(&state.pool, &seal_opts).await
    };
    match result {
        Ok(report) => {
            audit_system(
                &state,
                "dev.seed",
                json!({ "force": force, "tickets": report.tickets }),
            )
            .await;
            Ok(Json(report))
        }
        Err(core_db::demo::DemoError::AlreadySeeded(_)) => {
            // Typed reason (Wave 13 lane 1): the ops page branches on
            // `already_seeded` to offer the force-reseed second click —
            // never on the prose.
            Err(ApiError::AlreadySeeded)
        }
        Err(e) => Err(ApiError::Internal(anyhow::anyhow!("dev seed failed: {e}"))),
    }
}

/// POST /api/dev/reset-org — wipe the demo org and reseed from scratch.
pub async fn reset_org(
    State(state): State<AppState>,
) -> ApiResult<Json<core_db::demo::DemoSeedReport>> {
    let seal_opts = core_db::demo::DemoSeedOptions {
        seal_key: state.config.secret_key.0,
        object_store_dir: demo_object_store_dir(&state),
    };
    let report = core_db::demo::reset(&state.pool, &seal_opts)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("dev reset-org failed: {e}")))?;
    audit_system(
        &state,
        "dev.reset_org",
        json!({ "tickets": report.tickets }),
    )
    .await;
    Ok(Json(report))
}

/// POST /api/dev/time-travel {hours} — shift the demo org's SLA clock
/// columns back for breach/ladder testing. Direct column arithmetic on
/// `ticket_sla_state` (first_response_due/resolution_due); the worker's
/// next evaluation then sees the shifted budgets.
pub async fn time_travel(
    State(state): State<AppState>,
    Json(body): Json<TimeTravelRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let shifted = core_db::demo::time_travel(&state.pool, body.hours)
        .await
        .map_err(|e| match e {
            core_db::demo::DemoError::InvalidHours => {
                ApiError::Validation("hours must be between 1 and 720".into())
            }
            other => ApiError::Internal(anyhow::anyhow!("dev time-travel failed: {other}")),
        })?;
    audit_system(
        &state,
        "dev.time_travel",
        json!({ "hours": body.hours, "shifted": shifted }),
    )
    .await;
    Ok(Json(json!({ "hours": body.hours, "shifted": shifted })))
}

/// GET /api/dev/events?limit=100 — the last N outbox envelopes (newest
/// first) for the event-stream viewer.
pub async fn events(
    State(state): State<AppState>,
    Query(params): Query<EventsQuery>,
) -> ApiResult<Json<Vec<core_db::demo::DevEvent>>> {
    let limit = params.limit.unwrap_or(100);
    let rows = core_db::demo::recent_events(&state.pool, limit)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("dev events failed: {e}")))?;
    Ok(Json(rows))
}

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SeedQuery {
    pub force: Option<bool>,
}
