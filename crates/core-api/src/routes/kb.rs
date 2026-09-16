//! Knowledge-base endpoints (native KB v1, TASKLIST Phase 3).
//!
//! Purpose: staff CRUD for categories and articles with client-facing
//! visibility flags (portal v2 consumes the flags later). Mutations ride
//! the instant-first contract (IF1–IF3): idempotency-keyed claims,
//! If-Match-style version gates on updates, read-your-write `mutation`
//! metadata, and typed errors from the central envelope — never prose.
//!
//! Rich content (ADR 0007, Wave 16 lane 2): article accepts `content_jsonb`
//! — the versioned Tiptap-JSON envelope `{"schema_ver":1,"doc":…}` —
//! validated server-side against the pinned published Tiptap JSON schema
//! (`core-kb::content`), embed URLs through the ADR 0004 allowlist
//! registry (`core-kb::rich` — the ONE path), and rendered read-only to
//! sanitized HTML by `core-kb::render` (no editor in the read bundle).
//! The deterministic plain-text projection stays in `body` so FTS and
//! portal reads keep the pre-ADR contract.
//!
//! Search: article writes emit `kb_article.*` domain events; the worker's
//! event-driven indexer (2.16 pattern) mirrors published content into the
//! unified FTS index as kind `kb_article`. Nothing here writes the index.
//!
//! Non-purpose: no portal-facing reads (portal v2), no Markdown/HTML
//! import (D5 rides the D2 machinery later), and no per-category ACLs —
//! visibility is the org-wide flag pair decided by `core_kb`'s rule.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_db::kb::ArticleFilter;
use core_domain::entities::{KbArticle, KbArticleStatus, KbCategory};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

fn kb(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Kb,
        action,
    }
}

/// KB rule violations arrive as typed envelope errors: shape problems map
/// to `validation_failed`, the archived content lock is a state conflict.
impl From<core_kb::KbError> for ApiError {
    fn from(e: core_kb::KbError) -> Self {
        match e {
            core_kb::KbError::ArchivedLocked => ApiError::Conflict(e.to_string()),
            other => ApiError::Validation(other.to_string()),
        }
    }
}

impl From<core_kb::content::ContentError> for ApiError {
    fn from(e: core_kb::content::ContentError) -> Self {
        ApiError::Validation(e.to_string())
    }
}

impl From<core_kb::render::RenderError> for ApiError {
    fn from(e: core_kb::render::RenderError) -> Self {
        ApiError::Validation(e.to_string())
    }
}

/// The ADR 0007 write gate for rich content: envelope validated against
/// the pinned published Tiptap JSON schema (allowlisted nodes/marks,
/// depth/byte bounds), every embed URL re-resolved through the allowlist
/// registry IN THIS TRANSACTION (a provider disabled a moment ago never
/// sanctions a write arriving later), and the deterministic plain-text
/// projection (FTS + portal back-compat) returned.
fn gate_content_jsonb(
    content: &Value,
    providers: &[core_domain::entities::KbEmbedProvider],
) -> ApiResult<String> {
    let bytes = serde_json::to_vec(content)
        .map_err(|_| ApiError::Validation("content_jsonb must serialize".into()))?;
    core_kb::content::validate_content(&bytes)?;
    // The envelope's doc also must pass embed registry validation.
    let Value::Object(map) = content else {
        return Err(ApiError::Validation(
            "content_jsonb must be an object".into(),
        ));
    };
    let doc = map.get("doc").cloned().unwrap_or(Value::Null);
    core_kb::content::validate_embeds(&doc, providers)?;
    Ok(core_kb::content::plain_text(&doc))
}

/// Derives read-only HTML from the canonical envelope (or the plain-text
/// fallback for legacy rows), running the registry in scope so a
/// currently-allowlisted YouTube embed renders as a generated iframe and
/// everything else degrades to blocked placeholders.
fn read_content_html(
    article: &KbArticle,
    providers: &[core_domain::entities::KbEmbedProvider],
) -> Option<String> {
    if let Some(doc) = article.content_jsonb.as_ref().and_then(|c| c.get("doc")) {
        return Some(core_kb::render::render(doc, providers));
    }
    // Legacy/no-envelope rows: render the plain body as one doc of
    // paragraph nodes — the 0077 backfill's exact shape.
    if article.body.is_empty() {
        return None;
    }
    let mut children = Vec::new();
    for line in article.body.split('\n') {
        children.push(serde_json::json!({
            "type": "paragraph",
            "content": [{ "type": "text", "text": line }]
        }));
    }
    Some(core_kb::render::render(
        &serde_json::json!({ "type": "doc", "content": children }),
        providers,
    ))
}

/// The single-article read surface (GET): article fields at the top
/// level plus the sanitized read-only HTML projection the web client
/// renders server-side — the editor package never ships in bundles that
/// only read.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ArticleDetail {
    #[serde(flatten)]
    pub article: KbArticle,
    /// Sanitized server-rendered HTML (core-kb render whitelist).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_html: Option<String>,
}

/// Remaps the slug UNIQUE violation onto a readable typed conflict; every
/// other error passes through the standard mapping.
fn slug_conflict(e: core_db::DbError, what: &str) -> ApiError {
    match &e {
        core_db::DbError::Sql(sqlx::Error::Database(db))
            if db.constraint().unwrap_or("").contains("slug") =>
        {
            ApiError::Conflict(format!("slug already in use ({what})"))
        }
        core_db::DbError::NotFound => ApiError::NotFound,
        _ => e.into(),
    }
}

// --- categories ---------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateCategoryRequest {
    pub name: String,
    /// Stable per-org handle; derived from the name when omitted.
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub position: Option<i32>,
    /// Portal v2 gate: only categories with this flag can expose articles
    /// to client users.
    #[serde(default)]
    pub client_visible: bool,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateCategoryRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub position: Option<i32>,
    #[serde(default)]
    pub client_visible: Option<bool>,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2): category fields at the top level
/// plus the additive `mutation` metadata.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CategoryMutationResponse {
    #[serde(flatten)]
    pub category: KbCategory,
    pub mutation: instant::MutationMeta,
}

/// GET /api/kb/categories
#[utoipa::path(operation_id = "kb_list_categories", get, path = "/api/kb/categories", tag = "kb", security(("bearer" = [])),
    responses((status = 200, body = [KbCategory])))]
pub async fn list_categories(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<KbCategory>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let categories = core_db::kb::list_categories(&mut *tx, authed.principal.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(categories))
}

/// POST /api/kb/categories — instant-first (IF1/IF2).
#[utoipa::path(operation_id = "kb_create_category", post, path = "/api/kb/categories", tag = "kb", security(("bearer" = [])),
    request_body = CreateCategoryRequest,
    responses((status = 200, body = CategoryMutationResponse),
              (status = 409, description = "slug already in use or idempotency key reused")))]
pub async fn create_category(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateCategoryRequest>,
) -> ApiResult<Json<CategoryMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Create))?;
    let name = req.name.trim().to_string();
    core_kb::validate_title(&name)?;
    let slug = core_kb::slugify(req.slug.as_deref().unwrap_or(&name));
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "kb_category",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let category = core_db::kb::create_category(
            &mut *ctx.tx,
            org_id,
            Uuid::now_v7(),
            &name,
            &slug,
            req.description.as_deref().unwrap_or(""),
            req.position.unwrap_or(0),
            req.client_visible,
        )
        .await
        .map_err(|e| slug_conflict(e, "category"))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "kb_category",
                entity_id: category.id,
                version: None,
            },
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "kb_category",
                entity_id: category.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&category).unwrap_or(Value::Null),
                updated_at: category.created_at,
                key: key.as_deref(),
                payload: &category,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(CategoryMutationResponse { category, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/kb/categories/{id} — instant-first (IF1/IF2).
#[utoipa::path(operation_id = "kb_update_category", patch, path = "/api/kb/categories/{id}", tag = "kb", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdateCategoryRequest,
    responses((status = 200, body = CategoryMutationResponse),
              (status = 404, description = "category not visible in scope"),
              (status = 409, description = "version conflict, slug reuse, or idempotency key reused")))]
pub async fn update_category(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateCategoryRequest>,
) -> ApiResult<Json<CategoryMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    if let Some(name) = &req.name {
        core_kb::validate_title(name)?;
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "kb_category",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::kb::get_category(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "kb_category",
                entity_id: id,
                version,
            },
        )
        .await?;
        // Slug is immutable after creation: it is the stable handle
        // portals and imports reference. Renames go through `name`.
        core_db::kb::update_category(
            &mut *ctx.tx,
            org_id,
            id,
            req.name.as_deref().unwrap_or(&before.name).trim(),
            req.description.as_deref().unwrap_or(&before.description),
            req.position.unwrap_or(before.position),
            req.client_visible.unwrap_or(before.client_visible),
        )
        .await
        .map_err(|e| slug_conflict(e, "category"))?;
        let after = core_db::kb::get_category(&mut *ctx.tx, org_id, id).await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "kb_category",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &after,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(CategoryMutationResponse {
                category: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/kb/categories/{id} — articles keep existing (category_id
/// clears; ON DELETE SET NULL), so no content is lost behind a removal.
#[utoipa::path(operation_id = "kb_delete_category", delete, path = "/api/kb/categories/{id}", tag = "kb", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = serde_json::Value),
              (status = 404, description = "category not visible in scope")))]
pub async fn delete_category(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let deleted = core_db::kb::delete_category(&mut *ctx.tx, org_id, id).await?;
        if !deleted {
            return Err(ApiError::NotFound);
        }
        Ok((ctx, ()))
    })
    .await?;

    Ok(Json(serde_json::json!({ "deleted": id })))
}

// --- articles ---------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateArticleRequest {
    pub title: String,
    /// Stable per-org handle; derived from the title when omitted.
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    /// Canonical rich content (ADR 0007): the versioned Tiptap-JSON
    /// envelope `{"schema_ver": 1, "doc": <tiptap-doc>}`. Validated
    /// server-side on write; when present it WINS over `body` (the
    /// plain-text projection is derived deterministically).
    #[serde(default)]
    pub content_jsonb: Option<Value>,
    #[serde(default)]
    pub category_id: Option<Uuid>,
    #[serde(default)]
    pub status: Option<KbArticleStatus>,
    /// Portal v2 gate: effective client visibility additionally requires a
    /// published status and a client-visible category (core_kb rule).
    #[serde(default)]
    pub client_visible: bool,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateArticleRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    /// Canonical rich content (ADR 0007); validated on every write. The
    /// editor's debounced autosave rides THIS field over the instant-first
    /// envelope (IF1–IF3 unchanged: idempotency key + version/If-Match).
    #[serde(default)]
    pub content_jsonb: Option<Value>,
    /// `null` clears the category; omit to leave it untouched.
    #[serde(default)]
    pub category_id: Option<Option<Uuid>>,
    #[serde(default)]
    pub status: Option<KbArticleStatus>,
    #[serde(default)]
    pub client_visible: Option<bool>,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// Instant-first mutation response (IF2).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ArticleMutationResponse {
    #[serde(flatten)]
    pub article: KbArticle,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct ArticleListQuery {
    #[serde(default)]
    pub category_id: Option<Uuid>,
    #[serde(default)]
    pub status: Option<KbArticleStatus>,
    /// Only articles passing the effective client-facing gate.
    #[serde(default)]
    pub client_visible: bool,
    /// Title substring narrowing; ranked full-text search rides /api/search.
    #[serde(default)]
    pub q: Option<String>,
}

/// GET /api/kb/articles
#[utoipa::path(operation_id = "kb_list_articles", get, path = "/api/kb/articles", tag = "kb", security(("bearer" = [])),
    params(ArticleListQuery),
    responses((status = 200, body = [KbArticle])))]
pub async fn list_articles(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ArticleListQuery>,
) -> ApiResult<Json<Vec<KbArticle>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let articles = core_db::kb::list_articles(
        &mut *tx,
        authed.principal.user.org_id,
        &ArticleFilter {
            category_id: q.category_id,
            status: q.status,
            client_visible: q.client_visible,
            q: q.q,
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(articles))
}

/// GET /api/kb/articles/{id} — with the sanitized read-only HTML
/// projection (no editor package in any read bundle).
#[utoipa::path(operation_id = "kb_get_article", get, path = "/api/kb/articles/{id}", tag = "kb", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ArticleDetail),
              (status = 404, description = "article not visible in scope")))]
pub async fn get_article(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ArticleDetail>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    // RLS-scoped read (pool carries no app.org_id GUC).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let article = core_db::kb::get_article(&mut *tx, authed.principal.user.org_id, id).await?;
    let providers =
        core_db::kb_rich::list_providers(&mut *tx, authed.principal.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(ArticleDetail {
        content_html: read_content_html(&article, &providers),
        article,
    }))
}

/// POST /api/kb/articles — instant-first (IF1/IF2).
#[utoipa::path(operation_id = "kb_create_article", post, path = "/api/kb/articles", tag = "kb", security(("bearer" = [])),
    request_body = CreateArticleRequest,
    responses((status = 200, body = ArticleMutationResponse),
              (status = 409, description = "slug already in use or idempotency key reused"),
              (status = 422, description = "title/body rejected by KB rules")))]
pub async fn create_article(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateArticleRequest>,
) -> ApiResult<Json<ArticleMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Create))?;
    let title = req.title.trim().to_string();
    core_kb::validate_title(&title)?;
    let slug = core_kb::slugify(req.slug.as_deref().unwrap_or(&title));
    let status = req.status.unwrap_or(KbArticleStatus::Draft);
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let content = req.content_jsonb.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "kb_article",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        // Rich content is gated INSIDE the transaction against the
        // live allowlist registry (create carries its providers).
        // Text-only creates still land canonically: the body wraps into
        // the v1 envelope (the 0077 backfill's exact shape) on write.
        let content_jsonb = match &content {
            Some(c) => {
                let providers = core_db::kb_rich::list_providers(&mut *ctx.tx, org_id).await?;
                gate_content_jsonb(c, &providers)?;
                Some(c.clone())
            }
            None => {
                let wrap =
                    core_kb::content::plain_text_envelope(&req.body.clone().unwrap_or_default());
                core_kb::content::validate_content(
                    &serde_json::to_vec(&wrap)
                        .map_err(|_| ApiError::Validation("content must serialize".into()))?,
                )?;
                Some(wrap)
            }
        };
        // body: the explicit plain-text field when given (imports,
        // fallback writers keep their own contract, byte-bounded); the
        // deterministic projection of the rich doc otherwise so FTS and
        // portal reads stay in step with the canonical envelope.
        let body = match &content {
            Some(c) => core_kb::content::plain_text(c.get("doc").expect("envelope validated")),
            None => req.body.clone().unwrap_or_default(),
        };
        core_kb::validate_body(&body)?;
        let article = core_db::kb::create_article(
            &mut *ctx.tx,
            org_id,
            Uuid::now_v7(),
            req.category_id,
            &title,
            &slug,
            &body,
            content_jsonb,
            status,
            req.client_visible,
            user_id,
        )
        .await
        .map_err(|e| slug_conflict(e, "article"))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "kb_article",
                entity_id: article.id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            types::KB_ARTICLE_CREATED,
            "kb_article",
            Some(article.id.to_string().as_str()),
            &serde_json::json!({ "article_id": article.id, "status": article.status.as_str() }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "kb_article",
                entity_id: article.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&article).unwrap_or(Value::Null),
                updated_at: article.created_at,
                key: key.as_deref(),
                payload: &article,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ArticleMutationResponse { article, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/kb/articles/{id} — instant-first (IF1/IF2). Archived
/// articles refuse title/body edits until unarchived (typed conflict);
/// status flips stay legal.
#[utoipa::path(operation_id = "kb_update_article", patch, path = "/api/kb/articles/{id}", tag = "kb", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = UpdateArticleRequest,
    responses((status = 200, body = ArticleMutationResponse),
              (status = 404, description = "article not visible in scope"),
              (status = 409, description = "archived content lock, version conflict, slug reuse, or idempotency key reused"),
              (status = 422, description = "title/body rejected by KB rules")))]
pub async fn update_article(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateArticleRequest>,
) -> ApiResult<Json<ArticleMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    if let Some(title) = &req.title {
        core_kb::validate_title(title)?;
    }
    if let Some(body) = &req.body {
        core_kb::validate_body(body)?;
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "kb_article",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }
        let before = core_db::kb::get_article(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        // Archived articles are read-only except for the unarchive flip
        // itself (core-kb rule; typed conflict, never prose).
        // Rich content gates against the live registry IN this tx, and
        // its projection re-syncs `body` so FTS/portal never drift.
        let content_projection = match &req.content_jsonb {
            Some(c) => {
                let providers = core_db::kb_rich::list_providers(&mut *ctx.tx, org_id).await?;
                Some(gate_content_jsonb(c, &providers)?)
            }
            None => None,
        };
        let touches_content =
            req.title.is_some() || req.body.is_some() || req.content_jsonb.is_some();
        if touches_content {
            core_kb::ensure_content_editable(before.status)?;
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "kb_article",
                entity_id: id,
                version,
            },
        )
        .await?;
        core_db::kb::update_article(
            &mut *ctx.tx,
            org_id,
            id,
            &core_db::kb::ArticlePatch {
                title: req.title.clone().map(|t| t.trim().to_string()),
                body: if content_projection.is_some() {
                    content_projection
                } else {
                    req.body.clone()
                },
                content_jsonb: req.content_jsonb.clone(),
                category_id: req.category_id,
                status: req.status,
                client_visible: req.client_visible,
                updated_by: Some(user_id),
            },
        )
        .await
        .map_err(|e| slug_conflict(e, "article"))?;
        let after = core_db::kb::get_article(&mut *ctx.tx, org_id, id).await?;
        ctx.emit(
            types::KB_ARTICLE_UPDATED,
            "kb_article",
            Some(after.id.to_string().as_str()),
            &serde_json::json!({ "article_id": after.id, "status": after.status.as_str() }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "kb_article",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &after,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            MutationOutcome::Applied(ArticleMutationResponse {
                article: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/kb/articles/{id} — emits `kb_article.deleted` so the
/// unified-search indexer drops the article's index entry.
#[utoipa::path(operation_id = "kb_delete_article", delete, path = "/api/kb/articles/{id}", tag = "kb", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = serde_json::Value),
              (status = 404, description = "article not visible in scope")))]
pub async fn delete_article(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let deleted = core_db::kb::delete_article(&mut *ctx.tx, org_id, id).await?;
        if !deleted {
            return Err(ApiError::NotFound);
        }
        ctx.emit(
            types::KB_ARTICLE_DELETED,
            "kb_article",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "article_id": id }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;

    Ok(Json(serde_json::json!({ "deleted": id })))
}
