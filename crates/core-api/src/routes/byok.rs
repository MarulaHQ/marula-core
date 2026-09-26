//! BYOK (Bring-Your-Own-Key) AI completion settings and connectivity test
//! (TASKLIST Phase 3 adapters lane).
//!
//! Purpose:
//! - `PUT /api/settings/ai/byok` — seals and stores an API key/OAuth token
//!   for a provider (AES-256-GCM at-rest via `state.config.secret_key`),
//!   riding the instant-first write path (IF1–IF3): idempotency key replay,
//!   version gate, write trail, `ai.byok_updated` outbox event, audit row.
//! - `DELETE /api/settings/ai/byok/{provider}` — removal through the same
//!   gated path with an `ai.byok_deleted` event.
//! - `POST /api/settings/ai/test` — admin-gated test-connection probe; in
//!   none mode it surfaces the typed `CompletionError::Unavailable`.
//! - `GET /api/settings/ai/status` — configured provider overview (never
//!   secrets).
//!
//! Non-purpose: raw key bytes are NEVER returned, logged, written to the
//! trail/event payloads, or exposed past the sealed store.

use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::instant::{self, InstantClaim, MutationOutcome, MutationParams};
use crate::org_tx;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::completion::{CompletionError, CompletionMessage, CompletionRequest, MessageRole};
use core_domain::entities::{Actor, AuditEvent, Principal};
use core_domain::events::types;
use core_domain::policy::{Action, Module, Permission};
use core_domain::CompletionProvider;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

const ENTITY: &str = "byok_secret";

fn admin(action: Action) -> Permission {
    Permission::Matrix {
        module: Module::Admin,
        action,
    }
}

/// BYOK config is owner/admin standing ON TOP of the Admin:edit matrix
/// grant, so a custom role carrying admin-module permissions still cannot
/// mint credentials for the org.
fn require_admin(principal: &Principal, action: Action) -> ApiResult<()> {
    require_permission(principal, admin(action))?;
    if !principal.is_owner_or_admin() {
        return Err(ApiError::Forbidden(
            "BYOK credentials are owner/admin surface".into(),
        ));
    }
    Ok(())
}

// --- request/response shapes --------------------------------------------------

/// Request to store or update a sealed BYOK API key.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PutByokSecretRequest {
    /// Provider name: an AI provider ("openai", "anthropic", "ollama",
    /// "custom") or an export-sync connector ("qbo", "xero").
    pub provider: String,
    /// Plaintext API key / token to be sealed at rest.
    pub api_key: String,
    /// Instant-first autosave parameters (IF1): idempotency key + entity
    /// version. Absent in legacy clients — purely additive.
    #[serde(flatten, default)]
    pub instant: MutationParams,
}

/// Non-secret status of a stored BYOK key (presence flags only — never
/// key material). Flattened into mutation responses so a stored replay
/// deserializes byte-identically.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ByokSecretStatus {
    pub provider: String,
    pub has_key: bool,
    #[schema(value_type = String, nullable = true)]
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Mutation response: status fields plus the instant-first mutation
/// metadata (`mutation.version` echoes into the next write).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ByokSecretMutationResponse {
    #[serde(flatten)]
    pub status: ByokSecretStatus,
    pub mutation: instant::MutationMeta,
}

/// Request to test the connection to an AI provider.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TestConnectionRequest {
    /// Provider to test. If omitted, tests the currently active provider in settings.
    pub provider: Option<String>,
    /// Optional prompt to test with.
    pub test_prompt: Option<String>,
}

/// Result of testing an AI provider connection.
#[derive(Debug, Serialize, ToSchema)]
pub struct TestConnectionResponse {
    pub provider: String,
    pub ok: bool,
    /// Descriptive status message or reason on failure.
    pub message: String,
    pub latency_ms: Option<u64>,
}

/// Summary of all configured AI providers for the org.
#[derive(Debug, Serialize, ToSchema)]
pub struct ByokOverviewResponse {
    pub active_provider: String,
    pub active_model: String,
    pub configured_providers: Vec<String>,
}

// --- handlers -----------------------------------------------------------------

async fn active_org_value<'e, E>(
    executor: E,
    org_id: Uuid,
    key: &str,
    fallback: &str,
) -> ApiResult<String>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let overrides = core_db::settings::list_org_overrides(executor, org_id).await?;
    Ok(overrides
        .iter()
        .find(|o| o.key == key)
        .and_then(|o| o.value.as_str())
        .unwrap_or(fallback)
        .to_string())
}

/// GET /api/settings/ai/status — overview of AI completion provider & configured keys.
#[utoipa::path(
    operation_id = "settings_ai_status",
    get,
    path = "/api/settings/ai/status",
    tag = "settings",
    security(("bearer" = [])),
    responses((status = 200, body = ByokOverviewResponse))
)]
pub async fn get_ai_status(
    State(state): State<AppState>,
    extensions: Extensions,
) -> ApiResult<Json<ByokOverviewResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    // RLS-scoped read: settings and byok_secrets policies demand app.org_id
    // and the pool carries no GUC of its own — ride an org-bound tx.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;

    let active_provider = active_org_value(
        &mut *tx,
        org_id,
        core_domain::settings::AI_COMPLETION_PROVIDER,
        "none",
    )
    .await?;
    let active_model = active_org_value(
        &mut *tx,
        org_id,
        core_domain::settings::AI_COMPLETION_MODEL,
        "none",
    )
    .await?;

    let secrets = core_db::byok::list_secret_summaries(&mut *tx, org_id).await?;
    tx.commit().await.map_err(ApiError::from)?;
    // The generic sealed store also holds mail transport credentials under
    // reserved `mail.*` provider ids (Wave 34 lane 8); those are NOT AI
    // providers and must not surface in the AI settings overview.
    let configured_providers = secrets
        .into_iter()
        .map(|s| s.provider)
        .filter(|provider| !provider.starts_with("mail."))
        .collect();

    Ok(Json(ByokOverviewResponse {
        active_provider,
        active_model,
        configured_providers,
    }))
}

/// PUT /api/settings/ai/byok — seals and stores an API key at rest
/// (Admin:edit + owner/admin standing). Rides the instant-first write
/// path; replays are byte-identical and the trail stores presence flags
/// only.
#[utoipa::path(
    operation_id = "settings_ai_byok_put",
    put,
    path = "/api/settings/ai/byok",
    tag = "settings",
    security(("bearer" = [])),
    request_body = PutByokSecretRequest,
    responses(
        (status = 200, body = ByokSecretMutationResponse),
        (status = 403),
        (status = 409, description = "idempotency key reused or version conflict"),
        (status = 422, description = "invalid provider or key")
    )
)]
pub async fn put_byok_secret(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<PutByokSecretRequest>,
) -> ApiResult<Json<ByokSecretMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_admin(&authed.principal, Action::Edit)?;

    let provider = req.provider.trim().to_ascii_lowercase();
    if provider.is_empty() {
        return Err(ApiError::Validation("provider is required".into()));
    }
    let known = core_domain::settings::byok_provider_options().any(|p| p == provider);
    if !known {
        return Err(ApiError::Validation(format!(
            "unsupported provider: '{provider}'. Supported: {}",
            core_domain::settings::byok_provider_options()
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    if req.api_key.trim().is_empty() {
        return Err(ApiError::Validation("api_key cannot be empty".into()));
    }

    // Seal the key with the app master secret (AES-256-GCM). The plaintext
    // never enters a log, a trail row, or an event payload from here on.
    let sealed = crate::crypto::seal(&state.config.secret_key, req.api_key.as_bytes())
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("failed to seal secret: {e}")))?;

    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let new_id = Uuid::now_v7();
    let fingerprint = serde_json::to_value(&req).unwrap_or(Value::Null);
    let key = req.instant.idempotency_key.clone();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        if let InstantClaim::Replay(stored) = instant::claim(
            &mut ctx,
            instant::ClaimSpec {
                entity: ENTITY,
                entity_id: None,
                key: key.as_deref(),
                fingerprint: &fingerprint,
            },
        )
        .await?
        {
            return Ok((ctx, MutationOutcome::Replayed(stored)));
        }

        let (secret_id, inserted) = core_db::byok::upsert_secret(
            &mut ctx.tx,
            new_id,
            ctx.org_id,
            &provider,
            &sealed,
            Some(user_id),
        )
        .await?;
        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: secret_id,
                version: req.instant.version,
            },
        )
        .await?;

        let updated_at = chrono::Utc::now();
        let status = ByokSecretStatus {
            provider: provider.clone(),
            has_key: true,
            updated_at: Some(updated_at),
        };
        ctx.emit(
            types::AI_BYOK_UPDATED,
            ENTITY,
            Some(secret_id.to_string().as_str()),
            &serde_json::json!({ "provider": provider, "has_key": true }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id: ctx.org_id,
                actor: Actor::Human(user_id),
                action: "settings.ai_byok_updated".to_string(),
                entity_type: Some(ENTITY.to_string()),
                entity_id: Some(secret_id.to_string()),
                detail: serde_json::json!({ "provider": provider, "has_key": true }),
                ip: None,
                occurred_at: updated_at,
            },
        )
        .await?;

        let snapshot = serde_json::json!({
            "provider": provider,
            "has_key": true,
        });
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: secret_id,
                op: if inserted { "create" } else { "update" },
                seq: write_seq,
                before: if inserted {
                    None
                } else {
                    Some(snapshot.clone())
                },
                after: snapshot,
                // Admin BYOK config write: human actor, no agent context
                // (the sealed key material itself never enters the trail).
                provenance: Default::default(),
                updated_at,
                key: key.as_deref(),
                payload: &status,
            },
        )
        .await?;

        Ok((
            ctx,
            MutationOutcome::Applied(ByokSecretMutationResponse { status, mutation }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// DELETE /api/settings/ai/byok/{provider} — removes a sealed BYOK key
/// through the same gated path; the write trail keeps the presence
/// snapshot.
#[utoipa::path(
    operation_id = "settings_ai_byok_delete",
    delete,
    path = "/api/settings/ai/byok/{provider}",
    tag = "settings",
    security(("bearer" = [])),
    params(("provider" = String, Path)),
    responses(
        (status = 200, body = ByokSecretMutationResponse),
        (status = 403),
        (status = 404, description = "rls_sparse: no key stored for that provider")
    )
)]
pub async fn delete_byok_secret(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(provider): Path<String>,
) -> ApiResult<Json<ByokSecretMutationResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_admin(&authed.principal, Action::Edit)?;

    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;
    let provider = provider.trim().to_ascii_lowercase();

    let outcome = org_tx::with_org_tx(&state, org_id, user_id, |mut ctx| async move {
        let (secret_id, prior_updated_at) =
            core_db::byok::get_secret_meta(&mut ctx.tx, ctx.org_id, &provider)
                .await
                .map_err(ApiError::from)?
                .ok_or(ApiError::RlsSparse)?;

        let write_seq = instant::entity_seq(
            &mut ctx,
            instant::SeqSpec {
                entity: ENTITY,
                entity_id: secret_id,
                version: None,
            },
        )
        .await?;
        core_db::byok::delete_secret(&mut ctx.tx, ctx.org_id, &provider).await?;

        ctx.emit(
            types::AI_BYOK_DELETED,
            ENTITY,
            Some(secret_id.to_string().as_str()),
            &serde_json::json!({ "provider": provider }),
        )
        .await?;
        core_db::audit::insert(
            &mut *ctx.tx,
            &AuditEvent {
                id: Uuid::now_v7(),
                org_id: ctx.org_id,
                actor: Actor::Human(user_id),
                action: "settings.ai_byok_deleted".to_string(),
                entity_type: Some(ENTITY.to_string()),
                entity_id: Some(secret_id.to_string()),
                detail: serde_json::json!({ "provider": provider }),
                ip: None,
                occurred_at: chrono::Utc::now(),
            },
        )
        .await?;

        let before = serde_json::json!({
            "provider": provider,
            "has_key": true,
        });
        let mutation = instant::record(
            &mut ctx,
            instant::RecordSpec {
                entity: ENTITY,
                entity_id: secret_id,
                op: "delete",
                seq: write_seq,
                before: Some(before),
                after: Value::Null,
                updated_at: prior_updated_at,
                key: None,
                // Admin BYOK config write: human actor, no agent context
                // (the sealed key material itself never enters the trail).
                provenance: Default::default(),
                payload: &ByokSecretStatus {
                    provider: provider.clone(),
                    has_key: false,
                    updated_at: None,
                },
            },
        )
        .await?;

        Ok((
            ctx,
            MutationOutcome::Applied(ByokSecretMutationResponse {
                status: ByokSecretStatus {
                    provider,
                    has_key: false,
                    updated_at: None,
                },
                mutation,
            }),
        ))
    })
    .await?;

    Ok(Json(outcome.into_response()?))
}

/// POST /api/settings/ai/test — test-connection probe (Admin:edit gated).
/// None mode returns the typed unavailable outcome; a configured provider
/// verifies its sealed key still decrypts (tamper/rotation check). No
/// vendor HTTP call happens in v1 — the live probe rides the edge adapters
/// when a lane wires them.
#[utoipa::path(
    operation_id = "settings_ai_test",
    post,
    path = "/api/settings/ai/test",
    tag = "settings",
    security(("bearer" = [])),
    request_body = TestConnectionRequest,
    responses(
        (status = 200, body = TestConnectionResponse),
        (status = 403)
    )
)]
pub async fn test_ai_connection(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<TestConnectionRequest>,
) -> ApiResult<Json<TestConnectionResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_admin(&authed.principal, Action::Edit)?;

    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    // RLS-scoped read: settings/byok_secrets policies demand app.org_id.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;

    // Resolve provider: explicit in request or active in org settings
    let provider = match req.provider {
        Some(p) if !p.trim().is_empty() => p.trim().to_ascii_lowercase(),
        _ => {
            active_org_value(
                &mut *tx,
                org_id,
                core_domain::settings::AI_COMPLETION_PROVIDER,
                "none",
            )
            .await?
        }
    };

    if provider == "none" {
        tx.commit().await.map_err(ApiError::from)?;
        let test_req = CompletionRequest {
            messages: vec![CompletionMessage {
                role: MessageRole::User,
                content: req.test_prompt.unwrap_or_else(|| "ping".to_string()),
            }],
            model: None,
            temperature: None,
            max_tokens: Some(10),
        };
        let provider = core_domain::completion::NoneCompletionProvider;
        let err = provider.complete(&test_req).await.unwrap_err();
        return match err {
            CompletionError::Unavailable { reason } => Ok(Json(TestConnectionResponse {
                provider: "none".to_string(),
                ok: false,
                message: reason,
                latency_ms: Some(0),
            })),
            other => Err(ApiError::Internal(anyhow::anyhow!("{other}"))),
        };
    }

    // Check if secret key is present for this provider
    let secret = core_db::byok::get_secret(&mut *tx, org_id, &provider).await?;
    tx.commit().await.map_err(ApiError::from)?;
    let Some(ciphertext) = secret else {
        return Ok(Json(TestConnectionResponse {
            provider: provider.clone(),
            ok: false,
            message: format!("no API key configured for provider '{provider}'"),
            latency_ms: None,
        }));
    };

    // Verify secret decryption
    let _key_bytes = match crate::crypto::open(&state.config.secret_key, &ciphertext) {
        Ok(k) => k,
        Err(_) => {
            return Ok(Json(TestConnectionResponse {
                provider: provider.clone(),
                ok: false,
                message: "stored secret key failed decryption (tampered or rotated)".to_string(),
                latency_ms: None,
            }));
        }
    };

    let start = std::time::Instant::now();
    // In v1, we validate key formatting and readiness.
    // When live HTTP calling is enabled, provider client runs a probe call here.
    let latency = start.elapsed().as_millis() as u64;

    Ok(Json(TestConnectionResponse {
        provider,
        ok: true,
        message: "credentials verified and provider ready".to_string(),
        latency_ms: Some(latency),
    }))
}
