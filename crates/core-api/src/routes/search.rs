//! Unified search API (TASKLIST 2.16): ONE read endpoint over the shared
//! per-entity FTS index (tickets, comments, clients, sites, contacts).
//!
//! Purpose: ranked, RLS-scoped, permission-filtered results via
//! `websearch_to_tsquery` with a trigram ILIKE leg for partial tokens.
//! Per-kind visibility derives from the principal's module permissions:
//! `tickets:view` unlocks ticket/comment hits, `clients:view` unlocks
//! client/site/contact hits — a caller missing both gets 403.
//!
//! Non-purpose: no client-portal comment filtering (portal users search in
//! a later stage), no vector ranking (Stage 3 adds pgvector + RRF fusion
//! behind this same response contract), and no writes of any kind.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::{Query, State},
    http::Extensions,
    Json,
};
use core_domain::policy::{check_any, Action, Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Maximum accepted query length; longer input is rejected so pathological
/// requests never reach Postgres.
const MAX_QUERY_CHARS: usize = 256;
/// Queries shorter than this return an empty page instead of an error.
const MIN_QUERY_CHARS: usize = 2;

fn tickets(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Tickets,
        action,
    }
}

fn clients(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Clients,
        action,
    }
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct SearchQuery {
    /// Free-text query using websearch syntax: `"exact phrases"`,
    /// `-excluded`, plain terms AND-ed. Tokens ≥3 chars also match as
    /// substrings via the trigram fallback.
    pub q: String,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    25
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResult {
    /// `ticket` | `comment` | `client` | `site` | `contact`.
    pub entity_kind: String,
    pub entity_id: Uuid,
    /// comment → its ticket id; NULL otherwise.
    pub parent_id: Option<Uuid>,
    pub title: String,
    /// Raw body excerpt for client-side snippet rendering.
    pub snippet: String,
    /// Blended FTS + trigram relevance score (higher is better).
    pub score: f64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResponse {
    /// The trimmed query actually executed.
    pub query: String,
    /// Total matches across all permitted kinds.
    pub total: i64,
    /// Kinds the principal may see, mirrored into the query filter.
    pub kinds: Vec<String>,
    pub limit: i64,
    pub offset: i64,
    pub results: Vec<SearchResult>,
}

/// GET /api/search — unified staff search across tickets, comments and the
/// client hierarchy.
#[utoipa::path(operation_id = "search_query", get, path = "/api/search", tag = "search", security(("bearer" = [])),
    params(SearchQuery),
    responses((status = 200, body = SearchResponse)))]
pub async fn search(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<SearchResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let can_tickets =
        check_any(&authed.principal.permissions, &[tickets(Action::View)]).is_allowed();
    let can_clients =
        check_any(&authed.principal.permissions, &[clients(Action::View)]).is_allowed();
    let can_kb = check_any(
        &authed.principal.permissions,
        &[Permission::Matrix {
            module: Module::Kb,
            action: Action::View,
        }],
    )
    .is_allowed();
    let can_documents = check_any(
        &authed.principal.permissions,
        &[Permission::Matrix {
            module: Module::Documents,
            action: Action::View,
        }],
    )
    .is_allowed();
    if !can_tickets && !can_clients && !can_kb && !can_documents {
        require_permission(&authed.principal, tickets(Action::View))?;
    }

    // Guards first: short queries are a valid empty state, oversized ones
    // are rejected outright.
    let trimmed = q.q.trim().to_string();
    if trimmed.chars().count() > MAX_QUERY_CHARS {
        return Err(ApiError::Validation(format!(
            "query too long (max {MAX_QUERY_CHARS} characters)"
        )));
    }
    if trimmed.chars().count() < MIN_QUERY_CHARS {
        return Ok(Json(SearchResponse {
            query: trimmed,
            total: 0,
            kinds: Vec::new(),
            limit: q.limit.clamp(1, 50),
            offset: q.offset.max(0),
            results: Vec::new(),
        }));
    }

    // Kind allow-list from permissions — never trust the caller to scope.
    let mut kinds: Vec<String> = Vec::with_capacity(7);
    if can_tickets {
        kinds.push("ticket".into());
        kinds.push("comment".into());
    }
    if can_clients {
        kinds.push("client".into());
        kinds.push("site".into());
        kinds.push("contact".into());
    }
    if can_kb {
        kinds.push("kb_article".into());
    }
    if can_documents {
        kinds.push("document".into());
    }

    let trigram = trimmed.chars().count() >= 3;
    let limit = q.limit.clamp(1, 50);
    let offset = q.offset.max(0);

    let hits = core_db::search::search_page(
        &state.pool,
        authed.principal.user.org_id,
        &kinds,
        &trimmed,
        trigram,
        limit,
        offset,
    )
    .await?;
    let total = core_db::search::search_count(
        &state.pool,
        authed.principal.user.org_id,
        &kinds,
        &trimmed,
        trigram,
    )
    .await?;

    Ok(Json(SearchResponse {
        query: trimmed,
        total,
        kinds,
        limit,
        offset,
        results: hits
            .into_iter()
            .map(|h| SearchResult {
                entity_kind: h.entity_kind,
                entity_id: h.entity_id,
                parent_id: h.parent_id,
                title: h.title,
                snippet: h.body_excerpt,
                score: h.score,
            })
            .collect(),
    }))
}
