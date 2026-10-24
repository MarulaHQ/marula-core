//! KB rich-content endpoints (Wave 13 lane 4, KB1/KB2 per ADR 0004): the
//! oEmbed allowlist registry, per-article embed instances, and linked
//! external documents with link-check.
//!
//! Purpose: staff CRUD + the two action surfaces the ADR calls for —
//!
//! * embeds resolve through the REGISTRY only (`POST /kb/embeds/resolve`):
//!   the allowlist decision is `core-kb::rich`; with `fetch=true` the
//!   embed HTML is fetched server-side from the provider's oEmbed
//!   endpoint over HTTPS. User-supplied HTML never reaches a response —
//!   the only iframe source is the registered provider's own oEmbed
//!   answer.
//! * external documents record link-checks as data
//!   (`last_verified_at` + `last_verified_status`), never as a side
//!   effect of reads. A transport failure is recorded as `unreachable`,
//!   not surfaced as an error: the record exists, the link did not
//!   verify.
//!
//! Mutations ride the instant-first contract (IF1–IF3) exactly like the
//! KB article/category routes; events register in the contract registry
//! (`kb_embed_provider.*`, `kb_embed.*`, `kb_external_document.*`).
//!
//! Non-purpose: no portal-facing reads (portal v2 uses the client-gated
//! core-db queries), no rendering — the web layer renders the sanctioned
//! HTML returned here.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::Extensions,
    Json,
};
use core_db::kb_rich::{ExternalDocumentPatch, ProviderPatch};
use core_domain::entities::{KbEmbed, KbEmbedProvider, KbExternalDocument, LinkCheckStatus};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

fn kb(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Kb,
        action,
    }
}

/// Registry rule violations arrive as typed envelope errors: shape
/// problems map to `validation_failed`, the system-row lock is a state
/// conflict.
impl From<core_kb::rich::RichError> for ApiError {
    fn from(e: core_kb::rich::RichError) -> Self {
        match e {
            core_kb::rich::RichError::ProviderNotAllowlisted
            | core_kb::rich::RichError::ProviderDisabled => ApiError::Validation(format!(
                "url is not on the embed allowlist ({e}); raw arbitrary iframes are never allowed"
            )),
            other => ApiError::Validation(other.to_string()),
        }
    }
}

/// Rich-content unique violations pass through the CENTRAL mapping
/// (`map_unique_constraint` in error.rs) so they surface as typed reason
/// codes; only NotFound is remapped here.
fn rich_conflict(e: core_db::DbError) -> ApiError {
    match e {
        core_db::DbError::NotFound => ApiError::NotFound,
        other => other.into(),
    }
}

// --- oEmbed allowlist registry ------------------------------------------------

/// Instant-first mutation response (IF2) for provider rows.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProviderMutationResponse {
    #[serde(flatten)]
    pub provider: KbEmbedProvider,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateProviderRequest {
    pub name: String,
    /// Stable per-org handle; derived from the name when omitted.
    #[serde(default)]
    pub slug: Option<String>,
    /// Exact hosts this provider accepts (e.g. `["loom.com"]`).
    pub host_patterns: Vec<String>,
    /// Regex over the full resource URL (Rust regex syntax).
    pub url_regex: String,
    /// oEmbed endpoint template; exactly one `{url}` placeholder, https.
    pub oembed_endpoint: String,
    #[serde(default)]
    pub enabled: bool,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateProviderRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub host_patterns: Option<Vec<String>>,
    #[serde(default)]
    pub url_regex: Option<String>,
    #[serde(default)]
    pub oembed_endpoint: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// GET /api/kb/embed-providers — system rows + the org's extensions.
#[utoipa::path(operation_id = "kb_list_embed_providers", get, path = "/api/kb/embed-providers", tag = "kb", security(("bearer" = [])),
    responses((status = 200, body = [KbEmbedProvider])))]
pub async fn list_providers(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<Vec<KbEmbedProvider>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let providers =
        core_db::kb_rich::list_providers(&mut *tx, authed.principal.user.org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(providers))
}

/// POST /api/kb/embed-providers — registry extension (admin-managed);
/// instant-first (IF1/IF2). System rows are shipped, never created here.
#[utoipa::path(operation_id = "kb_create_embed_provider", post, path = "/api/kb/embed-providers", tag = "kb", security(("bearer" = [])),
    request_body = CreateProviderRequest,
    responses((status = 200, body = ProviderMutationResponse),
              (status = 409, description = "slug already in use or idempotency key reused"),
              (status = 422, description = "provider shape rejected (hosts/regex/endpoint)")))]
pub async fn create_provider(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateProviderRequest>,
) -> ApiResult<Json<ProviderMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    let name = req.name.trim().to_string();
    let slug = core_kb::slugify(req.slug.as_deref().unwrap_or(&name));
    core_kb::rich::validate_provider(
        &name,
        &req.host_patterns,
        &req.url_regex,
        &req.oembed_endpoint,
    )?;
    core_kb::rich::validate_url(&req.oembed_endpoint).map_err(|_| {
        ApiError::Validation("oembed_endpoint must be an absolute https url".to_string())
    })?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "kb_embed_provider",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, instant::MutationOutcome::Replayed(stored)));
        }
        let provider = core_db::kb_rich::create_provider(
            &mut *ctx.tx,
            org_id,
            Uuid::now_v7(),
            &name,
            &slug,
            &req.host_patterns,
            &req.url_regex,
            &req.oembed_endpoint,
            req.enabled,
        )
        .await
        .map_err(rich_conflict)?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "kb_embed_provider",
                entity_id: provider.id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            types::KB_EMBED_PROVIDER_CREATED,
            "kb_embed_provider",
            Some(provider.id.to_string().as_str()),
            &serde_json::json!({ "provider_id": provider.id, "slug": provider.slug, "is_system": false }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "kb_embed_provider",
                entity_id: provider.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&provider).unwrap_or(serde_json::Value::Null),
                updated_at: provider.created_at,
                key: key.as_deref(),
                payload: &provider,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            instant::MutationOutcome::Applied(ProviderMutationResponse { provider, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/kb/embed-providers/{id} — org rows only; instant-first.
#[utoipa::path(operation_id = "kb_update_embed_provider", patch, path = "/api/kb/embed-providers/{id}", tag = "kb", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)), request_body = UpdateProviderRequest,
    responses((status = 200, body = ProviderMutationResponse),
              (status = 404, description = "provider not visible in scope (system rows are not patchable through the org filter)"),
              (status = 409, description = "system-row edit, version conflict, or idempotency key reused"),
              (status = 422, description = "provider shape rejected")))]
pub async fn update_provider(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProviderRequest>,
) -> ApiResult<Json<ProviderMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    if let Some(name) = &req.name {
        core_kb::rich::validate_provider(
            name,
            req.host_patterns.as_deref().unwrap_or(&[]),
            req.url_regex.as_deref().unwrap_or("."),
            req.oembed_endpoint
                .as_deref()
                .unwrap_or("https://e/?u={url}"),
        )?;
    }
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let instant::InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "kb_embed_provider",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, instant::MutationOutcome::Replayed(stored)));
        }
        // System rows are platform-shipped: they are visible in reads but
        // never patchable. Typed conflict, never a silent no-op.
        let before = core_db::kb_rich::get_provider(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        if before.is_system {
            return Err(ApiError::Conflict(
                "system embed providers are platform-shipped and cannot be edited; create an org provider instead"
                    .into(),
            ));
        }
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "kb_embed_provider",
                entity_id: id,
                version,
            },
        )
        .await?;
        core_db::kb_rich::update_provider(
            &mut *ctx.tx,
            org_id,
            id,
            &ProviderPatch {
                name: req.name.clone().map(|n| n.trim().to_string()),
                host_patterns: req.host_patterns.clone(),
                url_regex: req.url_regex.clone(),
                oembed_endpoint: req.oembed_endpoint.clone(),
                enabled: req.enabled,
            },
        )
        .await?;
        let after = core_db::kb_rich::get_provider(&mut *ctx.tx, org_id, id).await?;
        ctx.emit(
            types::KB_EMBED_PROVIDER_UPDATED,
            "kb_embed_provider",
            Some(after.id.to_string().as_str()),
            &serde_json::json!({ "provider_id": after.id, "slug": after.slug, "enabled": after.enabled }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "kb_embed_provider",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(serde_json::Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(serde_json::Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &after,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            instant::MutationOutcome::Applied(ProviderMutationResponse {
                provider: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/kb/embed-providers/{id} — org rows only. Deleting a
/// provider leaves its embeds in place with `provider_id` cleared (the
/// FK is ON DELETE SET NULL) so they degrade to blocked, never to
/// unvalidated.
#[utoipa::path(operation_id = "kb_delete_embed_provider", delete, path = "/api/kb/embed-providers/{id}", tag = "kb", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = serde_json::Value),
              (status = 404, description = "provider not visible in scope"),
              (status = 409, description = "system rows are not deletable")))]
pub async fn delete_provider(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let before = core_db::kb_rich::get_provider(&mut *ctx.tx, org_id, id).await?;
        if before.is_system {
            return Err(ApiError::Conflict(
                "system embed providers are platform-shipped and cannot be deleted".into(),
            ));
        }
        let deleted = core_db::kb_rich::delete_provider(&mut *ctx.tx, org_id, id).await?;
        if !deleted {
            return Err(ApiError::NotFound);
        }
        ctx.emit(
            types::KB_EMBED_PROVIDER_DELETED,
            "kb_embed_provider",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "provider_id": id }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;

    Ok(Json(serde_json::json!({ "deleted": id })))
}

// --- embed resolution --------------------------------------------------------

/// The allowlist decision plus (optionally) the provider's own oEmbed
/// embed HTML, fetched server-side. `html` is present only when
/// `fetch=true` and the provider endpoint answers 2xx.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EmbedResolution {
    pub provider_id: uuid::Uuid,
    pub provider_slug: String,
    /// The percent-encoded oEmbed URL this resolution would fetch.
    pub oembed_url: String,
    /// Sanctioned embed HTML straight from the provider's oEmbed endpoint.
    /// Never client-supplied; only present when `fetch` was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ResolveEmbedRequest {
    /// The embed URL to validate against the allowlist.
    pub url: String,
    /// Fetch the provider's oEmbed HTML now (server-side, https, 10s
    /// budget). Default false: registry-only resolution.
    #[serde(default)]
    pub fetch: bool,
}

/// POST /api/kb/embeds/resolve — validate a URL against the allowlist
/// without attaching it to an article (live preview in the editor).
#[utoipa::path(operation_id = "kb_resolve_embed", post, path = "/api/kb/embeds/resolve", tag = "kb", security(("bearer" = [])),
    request_body = ResolveEmbedRequest,
    responses((status = 200, body = EmbedResolution),
              (status = 422, description = "url is not on the allowlist"),
              (status = 503, description = "provider oEmbed endpoint unreachable")))]
pub async fn resolve_embed(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<ResolveEmbedRequest>,
) -> ApiResult<Json<EmbedResolution>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let org_id = authed.principal.user.org_id;

    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, authed.principal.user.id)
        .await
        .map_err(ApiError::from)?;
    let providers = core_db::kb_rich::list_providers(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;

    let provider = core_kb::rich::match_provider(&providers, &req.url)?;
    let url = core_kb::rich::validate_url(&req.url)?;
    let oembed_url = core_kb::rich::oembed_url(&provider.oembed_endpoint, &url)?;
    let html = if req.fetch {
        Some(fetch_oembed_html(&oembed_url).await?)
    } else {
        None
    };
    Ok(Json(EmbedResolution {
        provider_id: provider.id,
        provider_slug: provider.slug.clone(),
        oembed_url,
        html,
    }))
}

/// Server-side oEmbed fetch: screened by the shared egress guard (https only,
/// public host, judged resolution, pinned connection), short timeout, JSON
/// response extracted. A redirect is a refusal, not a hop. Any transport or
/// payload failure surfaces as typed `unavailable` — never a 5xx prose leak,
/// and never a client-controlled fallback.
async fn fetch_oembed_json(oembed_url: &str) -> ApiResult<serde_json::Value> {
    let guarded = crate::outbound::guard(oembed_url)
        .await
        .map_err(|denial| match denial {
            // The endpoint is registry data; a row that is not https is a bad
            // row rather than an unreachable provider.
            core_domain::egress::EgressDenial::SchemeDenied => {
                ApiError::Unavailable("embed provider endpoint must be https".into())
            }
            other => ApiError::Validation(format!("embed provider endpoint refused: {other}")),
        })?;
    let client = guarded
        .client(std::time::Duration::from_secs(10))
        .map_err(|denial| {
            ApiError::Internal(anyhow::anyhow!("embed provider client refused: {denial}"))
        })?;
    get_json(&client, guarded.url()).await
}

/// The mechanical half: one `GET` against an already-screened, pinned URL.
async fn get_json(client: &reqwest::Client, url: &str) -> ApiResult<serde_json::Value> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| ApiError::Unavailable("embed provider endpoint unreachable".into()))?;
    if crate::outbound::is_redirect(response.status().as_u16()) {
        return Err(ApiError::Unavailable(format!(
            "embed provider endpoint answered {} (redirects are never followed)",
            response.status().as_u16()
        )));
    }
    if !response.status().is_success() {
        return Err(ApiError::Unavailable(format!(
            "embed provider endpoint answered {}",
            response.status().as_u16()
        )));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|_| ApiError::Unavailable("embed provider response was not JSON".into()))
}

async fn fetch_oembed_html(oembed_url: &str) -> ApiResult<String> {
    let value = fetch_oembed_json(oembed_url).await?;
    value
        .get("html")
        .and_then(|h| h.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            ApiError::Unavailable("embed provider response carried no html field".into())
        })
}

async fn fetch_oembed_title(oembed_url: &str) -> ApiResult<String> {
    let value = fetch_oembed_json(oembed_url).await?;
    Ok(value
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .to_string())
}

// --- embed instances ---------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EmbedMutationResponse {
    #[serde(flatten)]
    pub embed: KbEmbed,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateEmbedRequest {
    /// Must resolve against the allowlist — a create with an unmatched
    /// URL is a typed 422, never a stored row.
    pub url: String,
    /// Fetch the provider title now; empty when the fetch is skipped.
    #[serde(default)]
    pub fetch: bool,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

/// GET /api/kb/articles/{article_id}/embeds
#[utoipa::path(operation_id = "kb_list_article_embeds", get, path = "/api/kb/articles/{article_id}/embeds", tag = "kb", security(("bearer" = [])),
    params(("article_id" = uuid::Uuid, Path)),
    responses((status = 200, body = [KbEmbed])))]
pub async fn list_embeds(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(article_id): Path<Uuid>,
) -> ApiResult<Json<Vec<KbEmbed>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let embeds =
        core_db::kb_rich::list_embeds(&mut *tx, authed.principal.user.org_id, article_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(embeds))
}

/// POST /api/kb/articles/{article_id}/embeds — resolve-then-store;
/// instant-first (IF1/IF2).
#[utoipa::path(operation_id = "kb_create_embed", post, path = "/api/kb/articles/{article_id}/embeds", tag = "kb", security(("bearer" = [])),
    params(("article_id" = uuid::Uuid, Path)), request_body = CreateEmbedRequest,
    responses((status = 200, body = EmbedMutationResponse),
              (status = 404, description = "article not visible in scope"),
              (status = 409, description = "url already attached or idempotency key reused"),
              (status = 422, description = "url is not on the allowlist")))]
pub async fn create_embed(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(article_id): Path<Uuid>,
    Json(req): Json<CreateEmbedRequest>,
) -> ApiResult<Json<EmbedMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let instant::InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "kb_embed",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, instant::MutationOutcome::Replayed(stored)));
        }
        // The article gate first: a nonexistent article is a typed 404.
        core_db::kb::get_article(&mut *ctx.tx, org_id, article_id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let providers = core_db::kb_rich::list_providers(&mut *ctx.tx, org_id).await?;
        let provider = core_kb::rich::match_provider(&providers, &req.url)?;
        let url = core_kb::rich::validate_url(&req.url)?;
        let title = if req.fetch {
            let oembed_url = core_kb::rich::oembed_url(&provider.oembed_endpoint, &url)?;
            fetch_oembed_title(&oembed_url).await.unwrap_or_default()
        } else {
            String::new()
        };
        let embed = core_db::kb_rich::create_embed(
            &mut *ctx.tx,
            org_id,
            Uuid::now_v7(),
            article_id,
            url.as_str(),
            Some(provider.id),
            Some(&provider.slug),
            &title,
            user_id,
        )
        .await
        .map_err(rich_conflict)?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "kb_embed",
                entity_id: embed.id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            types::KB_EMBED_CREATED,
            "kb_embed",
            Some(embed.id.to_string().as_str()),
            &serde_json::json!({ "embed_id": embed.id, "article_id": embed.article_id, "provider_slug": embed.provider_slug }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "kb_embed",
                entity_id: embed.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&embed).unwrap_or(serde_json::Value::Null),
                updated_at: embed.created_at,
                key: key.as_deref(),
                payload: &embed,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            instant::MutationOutcome::Applied(EmbedMutationResponse { embed, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/kb/embeds/{id}
#[utoipa::path(operation_id = "kb_delete_embed", delete, path = "/api/kb/embeds/{id}", tag = "kb", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = serde_json::Value),
              (status = 404, description = "embed not visible in scope")))]
pub async fn delete_embed(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let embed = core_db::kb_rich::get_embed(&mut *ctx.tx, org_id, id).await?;
        let deleted = core_db::kb_rich::delete_embed(&mut *ctx.tx, org_id, id).await?;
        if !deleted {
            return Err(ApiError::NotFound);
        }
        ctx.emit(
            types::KB_EMBED_DELETED,
            "kb_embed",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "embed_id": id, "article_id": embed.article_id }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;

    Ok(Json(serde_json::json!({ "deleted": id })))
}

// --- linked external documents ----------------------------------------------

/// Instant-first mutation response (IF2) for external documents.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ExternalDocumentMutationResponse {
    #[serde(flatten)]
    pub document: KbExternalDocument,
    pub mutation: instant::MutationMeta,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateExternalDocumentRequest {
    pub title: String,
    /// Absolute https URL; SharePoint/OneDrive/Drive-class hosts are
    /// classified server-side — the client cannot lie about the kind.
    pub url: String,
    /// Optional attachment to one KB article. When attached, the row
    /// rides that article's client-facing gate for portal reads.
    #[serde(default)]
    pub article_id: Option<Uuid>,
    /// Who may open the link and what they need (e.g. "M365 group
    /// ACME-IT"). Documentation of access, never enforcement.
    #[serde(default)]
    pub permission_note: String,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateExternalDocumentRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub permission_note: Option<String>,
    /// `null` detaches the article; omit to leave it untouched.
    #[serde(default)]
    pub article_id: Option<Option<Uuid>>,
    /// Instant-first autosave parameters (IF1).
    #[serde(flatten, default)]
    pub instant: instant::MutationParams,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct ExternalDocumentListQuery {
    /// Narrow to one article's attachments.
    #[serde(default)]
    pub article_id: Option<Uuid>,
}

/// GET /api/kb/external-documents
#[utoipa::path(operation_id = "kb_list_external_documents", get, path = "/api/kb/external-documents", tag = "kb", security(("bearer" = [])),
    params(ExternalDocumentListQuery),
    responses((status = 200, body = [KbExternalDocument])))]
pub async fn list_external_documents(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(q): Query<ExternalDocumentListQuery>,
) -> ApiResult<Json<Vec<KbExternalDocument>>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let documents = core_db::kb_rich::list_external_documents(
        &mut *tx,
        authed.principal.user.org_id,
        q.article_id,
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(documents))
}

/// GET /api/kb/external-documents/{id}
#[utoipa::path(operation_id = "kb_get_external_document", get, path = "/api/kb/external-documents/{id}", tag = "kb", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = KbExternalDocument),
              (status = 404, description = "external document not visible in scope")))]
pub async fn get_external_document(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<KbExternalDocument>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::View))?;
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(
        &mut tx,
        authed.principal.user.org_id,
        authed.principal.user.id,
    )
    .await
    .map_err(ApiError::from)?;
    let document =
        core_db::kb_rich::get_external_document(&mut *tx, authed.principal.user.org_id, id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(Json(document))
}

/// POST /api/kb/external-documents — instant-first (IF1/IF2).
#[utoipa::path(operation_id = "kb_create_external_document", post, path = "/api/kb/external-documents", tag = "kb", security(("bearer" = [])),
    request_body = CreateExternalDocumentRequest,
    responses((status = 200, body = ExternalDocumentMutationResponse),
              (status = 404, description = "attached article not visible in scope"),
              (status = 409, description = "idempotency key reused"),
              (status = 422, description = "title/url rejected by KB rules")))]
pub async fn create_external_document(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<CreateExternalDocumentRequest>,
) -> ApiResult<Json<ExternalDocumentMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    core_kb::validate_title(&req.title)?;
    let url = core_kb::rich::validate_url(&req.url)?;
    let url_kind = core_kb::rich::classify_url_kind(url.as_str());
    let title = req.title.trim().to_string();
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let instant::InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "kb_external_document",
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, instant::MutationOutcome::Replayed(stored)));
        }
        if let Some(article_id) = req.article_id {
            core_db::kb::get_article(&mut *ctx.tx, org_id, article_id)
                .await
                .map_err(|e| instant::mutation_scope_err(e.into()))?;
        }
        let document = core_db::kb_rich::create_external_document(
            &mut *ctx.tx,
            org_id,
            Uuid::now_v7(),
            req.article_id,
            &title,
            url.as_str(),
            url_kind,
            req.permission_note.trim(),
            user_id,
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "kb_external_document",
                entity_id: document.id,
                version: None,
            },
        )
        .await?;
        ctx.emit(
            types::KB_EXTERNAL_DOCUMENT_CREATED,
            "kb_external_document",
            Some(document.id.to_string().as_str()),
            &serde_json::json!({ "external_document_id": document.id, "article_id": document.article_id, "url_kind": url_kind.as_str() }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "kb_external_document",
                entity_id: document.id,
                op: "create",
                seq: write_seq,
                before: None,
                after: serde_json::to_value(&document).unwrap_or(serde_json::Value::Null),
                updated_at: document.created_at,
                key: key.as_deref(),
                payload: &document,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            instant::MutationOutcome::Applied(ExternalDocumentMutationResponse {
                document,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// PATCH /api/kb/external-documents/{id} — instant-first (IF1/IF2). A
/// URL change resets the link-check columns: the old verification is
/// about the old URL.
#[utoipa::path(operation_id = "kb_update_external_document", patch, path = "/api/kb/external-documents/{id}", tag = "kb", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)), request_body = UpdateExternalDocumentRequest,
    responses((status = 200, body = ExternalDocumentMutationResponse),
              (status = 404, description = "external document not visible in scope"),
              (status = 409, description = "version conflict or idempotency key reused"),
              (status = 422, description = "title/url rejected by KB rules")))]
pub async fn update_external_document(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateExternalDocumentRequest>,
) -> ApiResult<Json<ExternalDocumentMutationResponse>> {
    if let Some(title) = &req.title {
        core_kb::validate_title(title)?;
    }
    if let Some(url) = &req.url {
        core_kb::rich::validate_url(url)?;
    }
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let fingerprint = serde_json::to_value(&req).unwrap_or(serde_json::Value::Null);
    let key = req.instant.idempotency_key.clone();
    let version = req.instant.version;

    let outcome = crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let instant::InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: "kb_external_document",
                entity_id: Some(id),
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, instant::MutationOutcome::Replayed(stored)));
        }
        let before = core_db::kb_rich::get_external_document(&mut *ctx.tx, org_id, id)
            .await
            .map_err(|e| instant::mutation_scope_err(e.into()))?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "kb_external_document",
                entity_id: id,
                version,
            },
        )
        .await?;
        core_db::kb_rich::update_external_document(
            &mut *ctx.tx,
            org_id,
            id,
            &ExternalDocumentPatch {
                title: req.title.clone().map(|t| t.trim().to_string()),
                url: req.url.clone(),
                url_kind: req.url.as_deref().map(core_kb::rich::classify_url_kind),
                permission_note: req.permission_note.clone().map(|n| n.trim().to_string()),
                article_id: req.article_id,
            },
        )
        .await?;
        let after = core_db::kb_rich::get_external_document(&mut *ctx.tx, org_id, id).await?;
        ctx.emit(
            types::KB_EXTERNAL_DOCUMENT_UPDATED,
            "kb_external_document",
            Some(after.id.to_string().as_str()),
            &serde_json::json!({ "external_document_id": after.id }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "kb_external_document",
                entity_id: id,
                op: "update",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(serde_json::Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(serde_json::Value::Null),
                updated_at: after.updated_at,
                key: key.as_deref(),
                payload: &after,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            instant::MutationOutcome::Applied(ExternalDocumentMutationResponse {
                document: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/kb/external-documents/{id}
#[utoipa::path(operation_id = "kb_delete_external_document", delete, path = "/api/kb/external-documents/{id}", tag = "kb", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = serde_json::Value),
              (status = 404, description = "external document not visible in scope")))]
pub async fn delete_external_document(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<serde_json::Value>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Delete))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let deleted = core_db::kb_rich::delete_external_document(&mut *ctx.tx, org_id, id).await?;
        if !deleted {
            return Err(ApiError::NotFound);
        }
        ctx.emit(
            types::KB_EXTERNAL_DOCUMENT_DELETED,
            "kb_external_document",
            Some(id.to_string().as_str()),
            &serde_json::json!({ "external_document_id": id }),
        )
        .await?;
        Ok((ctx, ()))
    })
    .await?;

    Ok(Json(serde_json::json!({ "deleted": id })))
}

/// The link-check response: the refreshed row plus the outcome.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct LinkCheckResponse {
    #[serde(flatten)]
    pub document: KbExternalDocument,
    pub mutation: instant::MutationMeta,
}

/// POST /api/kb/external-documents/{id}/link-check — performs one
/// server-side HTTPS HEAD (GET fallback) and records the outcome as
/// data. `unreachable` covers DNS/TLS/timeouts/non-2xx; the endpoint
/// never fails on an unreachable link — the record is the result.
#[utoipa::path(operation_id = "kb_link_check_external_document", post, path = "/api/kb/external-documents/{id}/link-check", tag = "kb", security(("bearer" = [])),
    params(("id" = uuid::Uuid, Path)),
    responses((status = 200, body = LinkCheckResponse),
              (status = 404, description = "external document not visible in scope")))]
pub async fn link_check_external_document(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<LinkCheckResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, kb(Action::Edit))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    let outcome = crate::org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let before = core_db::kb_rich::get_external_document(&mut *ctx.tx, org_id, id).await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: "kb_external_document",
                entity_id: id,
                version: None,
            },
        )
        .await?;
        let status = probe_link(&before.url).await?;
        let verified_at = chrono::Utc::now();
        core_db::kb_rich::record_link_check(&mut *ctx.tx, org_id, id, verified_at, status).await?;
        let after = core_db::kb_rich::get_external_document(&mut *ctx.tx, org_id, id).await?;
        ctx.emit(
            types::KB_EXTERNAL_DOCUMENT_LINK_CHECKED,
            "kb_external_document",
            Some(after.id.to_string().as_str()),
            &serde_json::json!({ "external_document_id": after.id, "status": status.as_str() }),
        )
        .await?;
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: "kb_external_document",
                entity_id: id,
                op: "link_check",
                seq: write_seq,
                before: Some(serde_json::to_value(&before).unwrap_or(serde_json::Value::Null)),
                after: serde_json::to_value(&after).unwrap_or(serde_json::Value::Null),
                updated_at: after.updated_at,
                key: None,
                payload: &after,
                provenance: Default::default(),
            },
        )
        .await?;
        Ok((
            ctx,
            instant::MutationOutcome::Applied(LinkCheckResponse {
                document: after,
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// One server-side HTTPS probe.
///
/// The stored URL is screened by the shared egress guard before a socket
/// exists, its host is resolved once and every answer judged, and the
/// connection is pinned to the judged addresses. A redirect is not followed: the
/// document's URL is the only target this check may touch, so a `3xx` is
/// recorded as reachable without a hop. Everything the guard refuses — a
/// loopback, link-local or metadata address, a name that resolves into one —
/// records as `unreachable`, the same as a DNS or TLS failure, because the
/// record is the result.
async fn probe_link(raw_url: &str) -> ApiResult<LinkCheckStatus> {
    let guarded = match crate::outbound::guard(raw_url.trim()).await {
        Ok(guarded) => guarded,
        Err(denial) => {
            tracing::warn!(
                reason = denial.as_str(),
                "link-check target refused before the request"
            );
            return Ok(LinkCheckStatus::Unreachable);
        }
    };
    let client = guarded
        .client(std::time::Duration::from_secs(10))
        .map_err(|denial| {
            ApiError::Internal(anyhow::anyhow!("link-check client refused: {denial}"))
        })?;
    Ok(probe_target(&client, guarded.url()).await)
}

/// The mechanical half: one `HEAD` against an already-screened, pinned URL.
///
/// A `3xx` counts as reachable and is never followed: the link works, and a hop
/// would be a second host the guard never judged. The status enum has no third
/// value, and adding one would change the record's vocabulary for a case the
/// check can answer today.
async fn probe_target(client: &reqwest::Client, url: &str) -> LinkCheckStatus {
    match client.head(url).send().await {
        Ok(response) if response.status().is_success() => LinkCheckStatus::Ok,
        Ok(response) if crate::outbound::is_redirect(response.status().as_u16()) => {
            tracing::warn!(
                status = response.status().as_u16(),
                "link-check target answered a redirect: reachable, and not followed"
            );
            LinkCheckStatus::Ok
        }
        _ => LinkCheckStatus::Unreachable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A loopback listener that answers every request with a `302` and counts
    /// the connections it accepts, so "refused, not followed" is a count rather
    /// than an inference. The guard would refuse this URL before a socket
    /// existed, which is why the mechanics half is driven directly here — the
    /// same split `plugin_http` uses.
    fn redirector(location: &'static str) -> (std::net::SocketAddr, Arc<AtomicUsize>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let count = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&count);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                seen.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                );
            }
        });
        (addr, count)
    }

    fn test_client() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("client")
    }

    /// A `3xx` is a working link and the check never follows it. The mechanics
    /// half is driven directly with a loopback listener because the guard
    /// refuses a loopback URL before a socket exists — the same split
    /// `plugin_http` uses. The listener counts connections, so "not followed"
    /// is a measurement rather than an inference.
    #[tokio::test]
    async fn a_link_check_reports_a_redirect_as_reachable_without_following_it() {
        let (addr, connections) = redirector("https://elsewhere.example/runbook");
        let status = probe_target(&test_client(), &format!("http://{addr}/runbook")).await;
        assert_eq!(
            status,
            LinkCheckStatus::Ok,
            "a redirecting link is reachable, not broken"
        );
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert_eq!(
            connections.load(Ordering::SeqCst),
            1,
            "the 302 was answered once and its Location never dialled"
        );
    }

    #[tokio::test]
    async fn an_oembed_redirect_is_refused_rather_than_followed() {
        let (addr, connections) = redirector("https://elsewhere.example/oembed");
        let result = get_json(&test_client(), &format!("http://{addr}/oembed")).await;
        assert!(
            matches!(result, Err(ApiError::Unavailable(_))),
            "a redirect is a typed unavailable, never a followed hop"
        );
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert_eq!(
            connections.load(Ordering::SeqCst),
            1,
            "the 302 was answered once and its Location never dialled"
        );
    }
}
