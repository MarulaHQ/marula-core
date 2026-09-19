//! Unified search API (TASKLIST 2.16): ONE read endpoint over the shared
//! per-entity FTS index (tickets, comments, clients, sites, contacts).
//!
//! Purpose: ranked, RLS-scoped, permission-filtered results via
//! `websearch_to_tsquery` with a trigram ILIKE leg for partial tokens, and
//! — when the org has an embedding-capable provider configured — the
//! pgvector leg fused in by reciprocal rank fusion (§4.5). Per-kind
//! visibility derives from the principal's module permissions:
//! `tickets:view` unlocks ticket/comment hits, `clients:view` unlocks
//! client/site/contact hits — a caller missing both gets 403.
//!
//! Non-purpose: no client-portal comment filtering (portal users search in
//! a later stage), no embedding generation (core-api's ai module resolves
//! the org provider), and no writes of any kind.
//!
//! Settled posture (Wave 23 lane 1, item 10 — a documented DELIBERATE
//! non-change): unified search does NOT narrow the `document` KIND below
//! the module permission (`documents:view`). Site/contact RESTRICTED
//! visibility governs which clients a staff member can SEE in /clients;
//! document hits are already tenant-scoped and per-client-narrowable via
//! the explicit `client_id` scoping param, so no additional document-kind
//! restriction is imposed at the search layer. Escalation trigger: if a
//! per-document sensitivity model (e.g. restricted flag on the document
//! row) or a per-site document ACL lands, THAT feature must add its search
//! leg here — search never invents its own visibility rule.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::{
    extract::Path,
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
    /// Optional client scope for the `document` kind (0089, per-client
    /// document scoping): when set, indexed documents answer only when
    /// org-global (client_scope NULL) or scoped to THIS client. Every
    /// other kind is untouched.
    #[schema(value_type = Option<uuid::Uuid>)]
    pub client_id: Option<Uuid>,
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

    // Vector leg (§4.5): embed the query through the org's BYOK provider.
    // None mode / provider failure ⇒ None ⇒ the legacy lexical path runs
    // byte-identically; search never 5xxs because AI is unconfigured or down.
    let query_embedding =
        crate::ai::embed_search_query(&state, authed.principal.user.org_id, &trimmed).await;

    // RLS-scoped read (pool carries no app.org_id GUC): one tx around
    // both queries.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let (hits, total) = match &query_embedding {
        Some(embedding) => (
            core_db::search::search_hybrid_page(
                &mut tx,
                authed.principal.user.org_id,
                &kinds,
                &trimmed,
                trigram,
                q.client_id,
                embedding,
                limit,
                offset,
            )
            .await?,
            core_db::search::search_hybrid_count(
                &mut *tx,
                authed.principal.user.org_id,
                &kinds,
                &trimmed,
                trigram,
                q.client_id,
            )
            .await?,
        ),
        None => (
            core_db::search::search_page(
                &mut *tx,
                authed.principal.user.org_id,
                &kinds,
                &trimmed,
                trigram,
                q.client_id,
                limit,
                offset,
            )
            .await?,
            core_db::search::search_count(
                &mut *tx,
                authed.principal.user.org_id,
                &kinds,
                &trimmed,
                trigram,
                q.client_id,
            )
            .await?,
        ),
    };
    tx.commit().await.map_err(ApiError::from)?;

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

// --- pins (Wave 17 lane 13, migration 0089) -----------------------------------

/// One pinned result as the /search surface renders it.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchPinView {
    #[schema(value_type = uuid::Uuid)]
    pub id: Uuid,
    /// The pinned query, snapshotted (a pin re-runs THIS query).
    pub query: String,
    pub entity_kind: String,
    pub entity_id: Uuid,
    /// e.g. comment → its ticket id; mirrors the live row.
    pub parent_id: Option<Uuid>,
    /// The title AT PIN TIME (the honest snapshot, never re-faked).
    pub title: String,
    #[schema(value_type = String)]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchPinList {
    pub pins: Vec<SearchPinView>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSearchPinRequest {
    /// The query that produced the result (2..=256 chars — the same bounds
    /// GET /api/search enforces), snapshotted on the pin.
    pub query: String,
    pub entity_kind: String,
    #[schema(value_type = uuid::Uuid)]
    pub entity_id: Uuid,
}

/// GET /api/search/pins — the caller's own pins, newest-first.
#[utoipa::path(operation_id = "search_pins_list", get, path = "/api/search/pins", tag = "search", security(("bearer" = [])),
    responses((status = 200, body = SearchPinList)))]
pub async fn pins_list(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> ApiResult<Json<SearchPinList>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    // A pin list shows kinds ONLY through the same permission gate the
    // search surface uses — never a wider sidebar.
    require_permission(&authed.principal, tickets(Action::View))?;
    let limit: i64 = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(50);
    let offset: i64 = q.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let rows = core_db::search_pins::list_for_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
        limit,
        offset,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(SearchPinList {
        pins: rows
            .into_iter()
            .map(|r| SearchPinView {
                id: r.id,
                query: r.query,
                entity_kind: r.entity_kind,
                entity_id: r.entity_id,
                parent_id: r.parent_id,
                title: r.title,
                created_at: r.created_at,
            })
            .collect(),
    }))
}

/// The per-kind permission gate (mirrors the search route's allow-list).
fn kind_permission(kind: core_db::search::SearchKind) -> Permission {
    match kind {
        core_db::search::SearchKind::Ticket | core_db::search::SearchKind::Comment => {
            tickets(Action::View)
        }
        core_db::search::SearchKind::Client
        | core_db::search::SearchKind::Site
        | core_db::search::SearchKind::Contact => clients(Action::View),
        core_db::search::SearchKind::KbArticle => Permission::Matrix {
            module: Module::Kb,
            action: Action::View,
        },
        core_db::search::SearchKind::Document => Permission::Matrix {
            module: Module::Documents,
            action: Action::View,
        },
    }
}

/// POST /api/search/pins — pins one stored result. The entity must EXIST
/// in the index (a pin is a stored result, never invented) and kind sits
/// behind the same permission gate the query ran behind. One pin per
/// (kind, id): re-pinning is idempotent AND refreshes the stored
/// snapshot (query + title + parent) to current — the created_at stays
/// the original pin time (Wave 23 lane 1, item 6).
#[utoipa::path(operation_id = "search_pins_create", post, path = "/api/search/pins", tag = "search", security(("bearer" = [])),
    request_body = CreateSearchPinRequest,
    responses((status = 200, body = SearchPinView),
              (status = 404, description = "that row is not in your org's search index"),
              (status = 403, description = "no permission for that kind"),
              (status = 422, description = "validation failed")))]
pub async fn pins_create(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateSearchPinRequest>,
) -> ApiResult<Json<SearchPinView>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let query = req.query.trim().to_string();
    if query.chars().count() < MIN_QUERY_CHARS || query.chars().count() > MAX_QUERY_CHARS {
        return Err(ApiError::Validation(format!(
            "query must be {MIN_QUERY_CHARS}..={MAX_QUERY_CHARS} characters"
        )));
    }
    let kind = core_db::search::SearchKind::parse(&req.entity_kind)
        .ok_or_else(|| ApiError::Validation("unknown entity_kind".into()))?;
    require_permission(&authed.principal, kind_permission(kind))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let pin = crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        // The pin snapshots the LIVE index row: no row → typed 404 (the
        // honest "that result is not pin-able"). Re-pin (same identity)
        // REFRESHES the stored snapshot in the same upsert — the id and
        // created_at stay stable, query/parent/title go current.
        let indexed = core_db::search::lookup_row(&mut *ctx.tx, org_id, kind, req.entity_id)
            .await?
            .ok_or(ApiError::NotFound)?;
        let pin = core_db::search_pins::upsert(
            &mut ctx.tx,
            core_db::search_pins::NewSearchPin {
                id: Uuid::now_v7(),
                org_id,
                user_id,
                query: &query,
                entity_kind: kind.as_str(),
                entity_id: req.entity_id,
                parent_id: indexed.parent_id,
                title: &indexed.title,
            },
        )
        .await?;
        Ok((ctx, pin))
    })
    .await?;
    Ok(Json(SearchPinView {
        id: pin.id,
        query: pin.query,
        entity_kind: pin.entity_kind,
        entity_id: pin.entity_id,
        parent_id: pin.parent_id,
        title: pin.title,
        created_at: pin.created_at,
    }))
}

/// DELETE /api/search/pins/{id} — unpins the caller's OWN pin; another
/// user's pin is simply not theirs (typed 404, never a conflict leak).
#[utoipa::path(operation_id = "search_pins_delete", delete, path = "/api/search/pins/{id}", tag = "search", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Object),
              (status = 404, description = "pin not visible to this user")))]
pub async fn pins_delete(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let deleted =
        core_db::search_pins::delete(&mut tx, org_id, authed.principal.user.id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    if deleted.is_none() {
        return Err(ApiError::NotFound);
    }
    Ok(Json(serde_json::json!({ "deleted": true })))
}

/// Mounts the pin routes onto the protected surface.
pub fn pin_routes(gen: crate::routes::ApiGeneration) -> axum::Router<crate::state::AppState> {
    use axum::routing::{delete, post};
    let base = gen.base();
    axum::Router::new()
        .route(
            &format!("{base}/search/pins"),
            post(pins_create).get(pins_list),
        )
        .route(&format!("{base}/search/pins/{{id}}"), delete(pins_delete))
}
