//! The TRANSCRIPTION edge over ADR 0008's registered `transcription`
//! plugin surface (Wave 22): voice-note transcription as the surface's
//! FIRST core capability — the surface has been seeded (0081) and
//! admin-grantable through the Wave 18 plugin-surface settings since
//! lane 2b; this lane lands the CALL edge.
//!
//! Posture:
//! * the SAME deny-by-default admission pipeline every other surface
//!   rides — org master switch, enabled surface, per-user grant,
//!   catalog allowlist + capability check, hard caps, sealed credential
//!   resolution (`authorize_surface_call` drives it all; nothing here
//!   bypasses the grant set);
//! * audio-only: the attachment's content type must be an audio/* type
//!   the blocker allows (wave/wav/webm/mpeg/mp3/m4a/ogg) — everything
//!   else is a typed 422 (`not_audio`) rather than a silent attempt;
//! * provider adapter = the OpenAI-compatible `audio/transcriptions`
//!   multipart call (openai / ollama(whisper) resolve like the other
//!   provider surfaces; unknown providers stay typed refusals, never
//!   guesses);
//! * the usage ledger rides the SAME `record_usage` row shape (zero
//!   prompt/completion tokens — transcript cost is audio-time, not
//!   tokens; the row is the audit anchor the progress views read);
//! * honest unconfigured posture (ADR 0011 §3): a disabled / ungranted /
//!   credential-less surface produces the TYPED not-available refusals
//!   the client renders as visibly-absent states — the endpoint never
//!   fakes a transcript;
//! * the transcript text is returned to the caller for an explicit
//!   insert choice (no automatic ticket writes).
use crate::auth::middleware::{authed, require_permission};
use crate::error::{ApiError, ApiResult};
use crate::routes::ai_policy::{authorize_surface_call, AiAdmissionAsk, AiEdge};
use crate::routes::attachments;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::Extensions,
    Json,
};
use core_domain::policy::Action;
use serde_json::Value;
use uuid::Uuid;

/// The audio families the blocker accepts; everything else is typed.
pub(crate) fn is_transcribable(content_type: &str) -> bool {
    let base = content_type.split(';').next().unwrap_or("").trim();
    matches!(
        base,
        "audio/wav"
            | "audio/wave"
            | "audio/x-wav"
            | "audio/webm"
            | "audio/mpeg"
            | "audio/mp3"
            | "audio/mp4"
            | "audio/x-m4a"
            | "audio/ogg"
            | "audio/flac"
    )
}

const MAX_TRANSCRIBE_BYTES: i64 = 25 * 1024 * 1024;

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct TranscriptionResponse {
    /// The typed surface the call rode (always `transcription` here).
    pub surface: String,
    pub provider: String,
    pub model: String,
    #[schema(value_type = Option<String>)]
    /// The audio duration the provider reports, when it sends one.
    pub duration: Option<f64>,
    pub text: String,
}

/// POST /api/v1/attachments/{id}/transcribe — transcribe one stored
/// audio attachment through the org's granted `transcription` surface.
#[utoipa::path(operation_id = "attachments_transcribe", post,
    path = "/api/attachments/{id}/transcribe", tag = "attachments",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = TranscriptionResponse),
              (status = 403, description = "typed AI refusals (surface/grant/credential/cap)"),
              (status = 404), (status = 422, description = "not_audio or size cap")))]
pub async fn transcribe_attachment(
    State(state): State<AppState>,
    extensions: Extensions,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<TranscriptionResponse>> {
    let authed = authed::<crate::auth::AuthedUser>(&extensions)?;
    require_permission(&authed.principal, attachments::tickets(Action::View))?;
    let org_id = authed.principal.user.org_id;
    let user_id = authed.principal.user.id;

    // Attachment + visibility: the SAME board-gate the download rides.
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id).await?;
    let attachment = core_db::attachments::get(&mut *tx, org_id, id).await?;
    if let Some(ticket_id) = attachment.ticket_id {
        let ticket = core_db::tickets::get_ticket(&mut *tx, org_id, ticket_id).await?;
        crate::routes::board_access::authorize_board_op(
            &mut tx,
            &authed.principal,
            ticket.board_id,
            core_domain::policy::BoardOp::View,
        )
        .await?;
    } else if !authed.principal.is_owner_or_admin() && attachment.email_message_id.is_none() {
        return Err(ApiError::Forbidden(
            "transcription requires owner or admin".into(),
        ));
    }
    let is_audio = is_transcribable(&attachment.content_type);
    tx.commit().await.map_err(ApiError::from)?;
    if !is_audio {
        return Err(ApiError::Validation(format!(
            "attachment is not transcribable audio ({})",
            attachment.content_type
        )));
    }

    // Admission: THE shared pipeline (grant → allowlist → caps → sealed
    // credential), Completion edge, plugin surface `transcription`.
    let required = [AiEdge::Completion];
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    let call = authorize_surface_call(
        &state,
        &mut tx,
        AiAdmissionAsk {
            org_id,
            user_id,
            surface_key: "transcription",
            provider_ask: None,
            model_ask: None,
            required_capabilities: &required,
            internal_core_call: false,
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;

    // Audio bytes: read through the SAME content-addressed store.
    let obj = state
        .store
        .get(&attachment.storage_key)
        .await
        .map_err(store_error)?;
    let owned_bytes = axum::body::to_bytes(
        axum::body::Body::from_stream(obj.stream),
        MAX_TRANSCRIBE_BYTES as usize,
    )
    .await
    .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))?;
    let bytes = owned_bytes.to_vec();
    if bytes.len() > MAX_TRANSCRIBE_BYTES as usize {
        return Err(ApiError::Validation(format!(
            "audio attachment exceeds the transcription cap ({} > {MAX_TRANSCRIBE_BYTES})",
            bytes.len()
        )));
    }

    let transcript = call_transcription_provider(
        &call.provider,
        &call.model,
        &String::from_utf8_lossy(&call.plaintext_secret),
        &attachment.content_type,
        &bytes,
    )
    .await?;

    // Usage ledger row (audio call = zero tokens; the caps read
    // cost-weighted tokens, so this lands as an audited event only).
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org_and_user(&mut tx, org_id, user_id)
        .await
        .map_err(ApiError::from)?;
    core_db::ai_policy::record_usage(
        &mut tx,
        &core_db::ai_policy::AiUsageNew {
            id: Uuid::now_v7(),
            org_id,
            user_id,
            surface: call.surface_key.clone(),
            provider: call.provider.clone(),
            model: call.model.clone(),
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_weight: call.cost_weight,
            request_id: Some(Uuid::now_v7().to_string()),
        },
    )
    .await?;
    core_db::audit::insert(
        &mut *tx,
        &core_domain::entities::AuditEvent {
            id: Uuid::now_v7(),
            org_id,
            actor: core_domain::entities::Actor::Human(user_id),
            action: "ai.transcription_performed".into(),
            entity_type: Some("attachment".into()),
            entity_id: Some(id.to_string()),
            detail: serde_json::json!({
                "provider": call.provider,
                "model": call.model,
                "size_bytes": bytes.len(),
            }),
            ip: None,
            occurred_at: chrono::Utc::now(),
        },
    )
    .await?;
    tx.commit().await.map_err(ApiError::from)?;

    let provider = call.provider.clone();
    let model = call.model.clone();
    Ok(Json(TranscriptionResponse {
        surface: "transcription".into(),
        provider,
        model,
        duration: transcript.duration,
        text: transcript.text,
    }))
}

/// NEVER logs or returns the API key; the auth carries in headers only.
pub(crate) struct ProviderTranscript {
    pub text: String,
    pub duration: Option<f64>,
}

async fn call_transcription_provider(
    provider: &str,
    model: &str,
    api_key: &str,
    content_type: &str,
    bytes: &[u8],
) -> ApiResult<ProviderTranscript> {
    let (base_url, default_model): (&str, &str) = match provider {
        "openai" => (
            &std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            "gpt-4o-transcribe",
        ),
        "ollama" => (
            &std::env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".into()),
            "whisper",
        ),
        _ => {
            return Err(ApiError::AiCallRefused {
                reason: "ai_model_not_capable",
                message: format!(
                    "the '{provider}' provider has no transcription endpoint on this deployment"
                ),
                surface: "transcription".into(),
            });
        }
    };
    let model_used = if model.trim().is_empty() || model == "none" {
        default_model
    } else {
        model
    };
    let filename = match base_file_ext(content_type) {
        Some(ext) => format!("audio.{ext}"),
        None => "audio.bin".into(),
    };
    let part = reqwest::multipart::Part::bytes(bytes.to_vec())
        .file_name(filename)
        .mime_str(content_type.split(';').next().unwrap_or("audio/*"))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.to_string())))?;
    let form = reqwest::multipart::Form::new()
        .text("model", model_used.to_string())
        .part("file", part);
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base_url}/audio/transcriptions"))
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| {
            ApiError::Unavailable(format!("the transcription provider is unreachable: {e}"))
        })?;
    let status = response.status();
    let body_text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ApiError::AiCallRefused {
            reason: "ai_provider_error",
            message: format!(
                "the transcription provider refused the call (HTTP {})",
                status.as_u16()
            ),
            surface: "transcription".into(),
        });
    }
    let parsed: Value = serde_json::from_str(&body_text).map_err(|_| {
        ApiError::Unavailable("the transcription provider returned unreadable JSON".into())
    })?;
    let text = parsed
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::AiCallRefused {
            reason: "ai_provider_error",
            message: "the transcription provider returned no text".into(),
            surface: "transcription".into(),
        })?
        .to_string();
    let duration = parsed.get("duration").and_then(|v| v.as_f64());
    Ok(ProviderTranscript { text, duration })
}

fn base_file_ext(content_type: &str) -> Option<&'static str> {
    match content_type.split(';').next().unwrap_or("").trim() {
        "audio/wav" | "audio/wave" | "audio/x-wav" => Some("wav"),
        "audio/webm" => Some("webm"),
        "audio/mpeg" | "audio/mp3" => Some("mp3"),
        "audio/mp4" | "audio/x-m4a" => Some("m4a"),
        "audio/ogg" => Some("ogg"),
        "audio/flac" => Some("flac"),
        _ => None,
    }
}

fn store_error(e: core_store::StoreError) -> ApiError {
    ApiError::Internal(anyhow::anyhow!("object store error: {e}"))
}
