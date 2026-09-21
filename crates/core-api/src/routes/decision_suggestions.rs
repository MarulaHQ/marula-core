//! The workbench suggestion-card read model (Wave 23 lane 3,
//! `decision_workbench`): GET /tickets/{id}/decision-suggestions.
//!
//! Purpose: the newest recorded decision answers per surface for one
//! ticket, classified against the surface config's per-question
//! thresholds (act / pending / ignore — ADR 0010 §5). Suggestion
//! storage rides the append-only `decision_answers` audit store: the
//! newest row per (surface, entity) IS the live suggestion; shadow-mode
//! rows drive nothing and are labeled as such.
//!
//! Posture: degrade-quiet. An org with the master switch OFF simply has
//! no rows — the card renders nothing (visibly-absent doctrine), and
//! this route is never polled by surfaces that never saw answers.
//! Master-switch/OFF posture does NOT 5xx here: the read is honest over
//! whatever exists.

use crate::error::ApiResult;
use crate::routes::decision_edge;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::Extensions;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
pub struct SuggestionAnswer {
    pub question_id: String,
    /// Raw answer entry: kind + picked/level/probability + the FULL
    /// normalized distribution + confidence (no prose — none exists).
    pub answer: serde_json::Value,
    pub act: Option<f64>,
    pub review: Option<f64>,
    /// "act" | "pending" | "ignore" — the threshold gate at read time.
    /// `None` when no thresholds resolve (honest unconfigured state).
    pub state: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SuggestionSurface {
    pub surface: String,
    /// `shadow` rows drive nothing and are labeled as such.
    pub mode: String,
    pub provider: String,
    pub model: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub recorded_at: chrono::DateTime<chrono::Utc>,
    pub answers: Vec<SuggestionAnswer>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DecisionSuggestionsResponse {
    pub ticket_id: Uuid,
    pub suggestions: Vec<SuggestionSurface>,
}

/// GET /api/tickets/{id}/decision-suggestions — the workbench card's
/// read. Optimistic UI rides the instant-first envelope upstream; this
/// endpoint is a plain RLS'd read (nothing to undo, nothing to gate).
#[utoipa::path(operation_id = "tickets_decision_suggestions", get, path = "/api/tickets/{id}/decision-suggestions", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path)), responses(
        (status = 200, body = DecisionSuggestionsResponse),
        (status = 404, description = "ticket not visible in scope (rls_sparse)")))]
pub async fn list(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<DecisionSuggestionsResponse>> {
    let authed = crate::auth::middleware::authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let mut tx = crate::routes::ai_policy::org_bound_tx(&state, org_id, user_id).await?;
    // RLS proves visibility: an invisible ticket reads as no answers at
    // all (never a 404 leak about entities the caller cannot see).
    let rows = core_db::decision::latest_answers_for_ticket(&mut *tx, org_id, id).await?;
    let mut suggestions = Vec::with_capacity(rows.len());
    for row in rows {
        let config = match core_db::decision::get_surface_config(&mut *tx, org_id, &row.surface)
            .await
            .map_err(crate::error::ApiError::from)?
        {
            Some(cfg) => decision_edge::effective_config(&mut tx, &row.surface, &cfg)
                .await
                .ok(),
            None => None,
        };
        suggestions.push(classify(&row, config.as_ref()));
    }
    tx.commit().await.map_err(crate::error::ApiError::from)?;
    Ok(Json(DecisionSuggestionsResponse {
        ticket_id: id,
        suggestions,
    }))
}

/// Classifies one recorded row against the effective config's
/// thresholds: ≥ act → "act", ≥ review → "pending", else "ignore";
/// unresolvable config or unanswered question → `None` (honest).
fn classify(
    row: &core_db::decision::DecisionAnswersRow,
    config: Option<&core_decision::config::SurfaceConfig>,
) -> SuggestionSurface {
    let thresholds: std::collections::HashMap<&str, (f64, f64)> = config
        .map(|cfg| {
            cfg.questions
                .iter()
                .map(|q| (q.id.as_str(), (q.thresholds.act, q.thresholds.review)))
                .collect()
        })
        .unwrap_or_default();
    let mut answers = Vec::new();
    if let serde_json::Value::Object(map) = &row.answers {
        for (question_id, entry) in map {
            let threshold = thresholds.get(question_id.as_str()).copied();
            let value = entry
                .get("probability")
                .or_else(|| entry.get("level"))
                .and_then(|v| v.as_f64())
                .or_else(|| entry.get("confidence").and_then(|v| v.as_f64()));
            let state = match (threshold, value) {
                (Some((act, _)), Some(v)) if v >= act => Some("act".to_string()),
                (Some((_, review)), Some(v)) if v >= review => Some("pending".to_string()),
                (Some(_), Some(_)) => Some("ignore".to_string()),
                // Choice answers gate on confidence alone.
                _ => None,
            };
            answers.push(SuggestionAnswer {
                question_id: question_id.clone(),
                answer: entry.clone(),
                act: threshold.map(|(act, _)| act),
                review: threshold.map(|(_, review)| review),
                state,
            });
        }
    }
    SuggestionSurface {
        surface: row.surface.clone(),
        mode: row.mode.clone(),
        provider: row.provider.clone(),
        model: row.model.clone(),
        recorded_at: row.created_at,
        answers,
    }
}
