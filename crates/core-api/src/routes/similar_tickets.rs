//! The `decision_search_rerank` consumer (Wave 23 lane 4, ADR 0010 §3 #5):
//! GET /api/tickets/{id}/similar-tickets.
//!
//! Shape: the EXISTING Postgres index produces the top-k retrieval
//! candidates (the decision surface rides retrieval, never replaces it);
//! each candidate then gets its "does this actually relate?" Noul through
//! the decision edge — one ask per candidate, state composed per the
//! surface's starter template (query.text / candidate.title /
//! candidate.excerpt, clamped). Answers are recorded into the append-only
//! shadow store like every decision run.
//!
//! Ordering posture: the rerank applies ONLY to live-mode answers (shadow
//! rows drive nothing — ADR 0010 §5); quiet candidates keep their
//! retrieval rank. Below the act threshold a live candidate is labeled
//! `pending`/`ignore` but stays in the list with its answer visible —
//! this is a suggestion-grade read model, nothing is filtered away
//! silently.
//!
//! Old-school posture: an org with the decision surface unconfigured gets
//! the plain retrieval list (every candidate `relates: null`, `mode:
//! null`) and ZERO decision calls — the surface is silently quiet
//! (ADR 0011 §3).

use crate::auth::middleware::{authed, require_permission};
use crate::error::ApiResult;
use crate::routes::decision_edge::{self, DecisionAsk};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::Extensions;
use axum::Json;
use core_domain::policy::Permission;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Retrieval window: the lexical top-k slice fed to the reranker.
const RETRIEVAL_K: i64 = 8;
/// The reranked list size actually returned.
const MAX_CANDIDATES: usize = 5;

fn clamp(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SimilarTicketCandidate {
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: Uuid,
    pub title: String,
    /// Raw retrieval excerpt (client-side snippet rendering).
    pub snippet: String,
    /// Blended lexical relevance from the Postgres index (higher is
    /// better) — the retrieval rank that fed the reranker.
    pub retrieval_score: f64,
    /// The `relates` Noul probability, when the decision edge answered.
    /// `None` = the edge went quiet (unconfigured/downed) for that
    /// candidate.
    pub relates: Option<f64>,
    /// Threshold classification of the live answer: "act" | "pending" |
    /// "ignore". `None` when no answer, shadow mode, or no thresholds.
    pub state: Option<String>,
    /// `shadow` rows are recorded but drive nothing (no rerank applied).
    pub mode: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SimilarTicketsResponse {
    #[schema(value_type = uuid::Uuid)]
    pub ticket_id: Uuid,
    /// The retrieval query executed (the ticket's own title/body text).
    pub query: String,
    pub candidates: Vec<SimilarTicketCandidate>,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct SimilarTicketsQuery {
    /// Override the retrieval text (defaults to the ticket's own title
    /// plus a body excerpt). Clamped like the search surface.
    pub q: Option<String>,
}

/// GET /api/tickets/{id}/similar-tickets — retrieval top-k + per-candidate
/// relatedness Noul. Degrade-never-5xx: a quiet decision edge returns the
/// plain retrieval list, never an error.
#[utoipa::path(operation_id = "tickets_similar_tickets", get, path = "/api/tickets/{id}/similar-tickets", tag = "tickets", security(("bearer" = [])),
    params(("id" = Uuid, Path), SimilarTicketsQuery),
    responses(
        (status = 200, body = SimilarTicketsResponse),
        (status = 404, description = "ticket not visible in scope (rls_sparse)")))]
pub async fn similar_tickets(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Query(q): Query<SimilarTicketsQuery>,
) -> ApiResult<Json<SimilarTicketsResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    require_permission(
        &authed.principal,
        Permission::Matrix {
            module: core_domain::policy::Module::Tickets,
            action: core_domain::policy::Action::View,
        },
    )?;

    // 1. The anchor ticket + the retrieval page share one RLS'd tx.
    let mut tx = crate::routes::ai_policy::org_bound_tx(&state, org_id, user_id).await?;
    let ticket = core_db::tickets::get_ticket(&mut *tx, org_id, id).await?;
    let query_text = match q.q.as_deref().map(str::trim) {
        Some(q) if !q.is_empty() => clamp(q, 200),
        _ => {
            let body = ticket.body.clone().unwrap_or_default();
            clamp(&format!("{} {}", ticket.title, body), 200)
        }
    };
    if query_text.chars().count() < 2 {
        // Nothing meaningful to retrieve on — an honest empty list (the
        // search surface's short-query empty state, mirrored).
        tx.commit().await.map_err(crate::error::ApiError::from)?;
        return Ok(Json(SimilarTicketsResponse {
            ticket_id: id,
            query: query_text,
            candidates: Vec::new(),
        }));
    }
    let trigram = query_text.chars().count() >= 3;
    let kinds = vec!["ticket".to_string()];
    let hits = core_db::search::search_page(
        &mut *tx,
        org_id,
        &kinds,
        &query_text,
        trigram,
        None,
        RETRIEVAL_K,
        0,
    )
    .await?;
    tx.commit().await.map_err(crate::error::ApiError::from)?;

    // 2. Per-candidate relatedness Noul. One ask per candidate (the
    //    state carries THAT candidate), capped by the retrieval slice.
    let mut candidates = Vec::new();
    for hit in hits
        .iter()
        .filter(|h| h.entity_id != id)
        .take(MAX_CANDIDATES)
    {
        let mut relates = None;
        let mut state_label = None;
        let mut mode = None;
        let ask = DecisionAsk {
            org_id,
            user_id,
            surface_key: "decision_search_rerank",
            entity_type: "ticket",
            entity_id: Some(hit.entity_id),
            state: serde_json::json!({
                "query": { "text": query_text },
                "candidate": {
                    "title": clamp(&hit.title, 200),
                    "excerpt": clamp(&hit.body_excerpt, 1000),
                },
            }),
            request_id: format!("decision-rerank-{id}-{}", hit.entity_id),
            inventories: Default::default(),
        };
        if let Ok(Ok(run)) = decision_edge::decide(&state, ask).await {
            mode = Some(run.mode.clone());
            for answer in &run.response.answers {
                if answer.id() != "relates" {
                    continue;
                }
                if let core_decision::provider::Answer::Noul { probability, .. } = answer {
                    relates = Some(*probability);
                    if run.mode == "live" {
                        if let Some(question) =
                            run.config.questions.iter().find(|q| q.id == "relates")
                        {
                            state_label = Some(
                                if *probability >= question.thresholds.act {
                                    "act"
                                } else if *probability >= question.thresholds.review {
                                    "pending"
                                } else {
                                    "ignore"
                                }
                                .to_string(),
                            );
                        }
                    }
                }
            }
        }
        candidates.push(SimilarTicketCandidate {
            ticket_id: hit.entity_id,
            title: hit.title.clone(),
            snippet: hit.body_excerpt.clone(),
            retrieval_score: hit.score,
            relates,
            state: state_label,
            mode,
        });
    }

    // 3. Rerank ONLY live answers (shadow rows drive nothing): live
    //    candidates sort by relates desc ahead of everything else; quiet
    //    and shadow-answered candidates keep their retrieval rank.
    candidates.sort_by(|a, b| match (a.mode.as_deref(), b.mode.as_deref()) {
        (Some("live"), Some("live")) => b
            .relates
            .partial_cmp(&a.relates)
            .unwrap_or(std::cmp::Ordering::Equal),
        (Some("live"), _) => std::cmp::Ordering::Less,
        (_, Some("live")) => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Equal,
    });

    Ok(Json(SimilarTicketsResponse {
        ticket_id: id,
        query: query_text,
        candidates,
    }))
}
