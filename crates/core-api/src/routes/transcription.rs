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
//! * the usage ledger rides the SAME `record_usage` row shape and carries
//!   the call's REAL token counts: the provider's `usage` block when it
//!   sends one (the gpt-4o transcription models report input/output
//!   tokens), otherwise a count derived from the audio duration or size
//!   and the transcript — never zero, so the org's soft and hard caps
//!   bind on transcription the way they bind on chat. The generated
//!   `weighted_tokens` column prices the row from those counts;
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

    // Usage ledger row: transcription costs the org real money at the
    // provider, so it is metered like every other paid edge. The provider's
    // own `usage` block supplies the counts when it sends one; when it does
    // not, they are derived from the audio's duration or size and from the
    // transcript, so the row never lands at zero and the caps can bind. The
    // generated `weighted_tokens` column multiplies them by the catalog
    // weight — no second quota system.
    let metered = metered_tokens(
        transcript.usage,
        transcript.duration,
        bytes.len(),
        &transcript.text,
    );
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
            prompt_tokens: metered.prompt,
            completion_tokens: metered.completion,
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
                // The raw audio duration stays explainable to the client
                // even when the ledger's counts were derived from it.
                "duration_seconds": transcript.duration,
                "token_source": metered.source.as_str(),
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
    /// The provider's own token meter, when it sends one.
    pub usage: Option<ProviderUsage>,
}

/// The `usage` block a transcription response may carry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ProviderUsage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
}

impl ProviderUsage {
    /// Reads both naming conventions an OpenAI-compatible endpoint uses: the
    /// audio models report `input_tokens` / `output_tokens` (with an
    /// `input_token_details.audio_tokens` breakdown), while chat-shaped
    /// meters report `prompt_tokens` / `completion_tokens`. A block that
    /// carries neither is treated as no usage at all.
    fn from_wire(usage: &Value) -> Option<Self> {
        let read = |keys: &[&str]| -> Option<u32> {
            keys.iter()
                .find_map(|k| usage.get(*k).and_then(Value::as_u64))
                .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
        };
        let prompt_tokens = read(&["prompt_tokens", "input_tokens"]);
        let completion_tokens = read(&["completion_tokens", "output_tokens"]);
        if prompt_tokens.is_none() && completion_tokens.is_none() {
            return None;
        }
        Some(Self {
            prompt_tokens,
            completion_tokens,
        })
    }
}

/// Audio-input token basis: ten tokens per second of audio, the rate the
/// OpenAI-shaped transcription meters report through
/// `input_token_details.audio_tokens`. Used only when a provider sends no
/// `usage` block of its own.
const AUDIO_TOKENS_PER_SECOND: f64 = 10.0;

/// Duration basis for a response that reports neither usage nor duration:
/// 64 kbit/s, the middle of the range compressed voice notes arrive in. It
/// is a basis under a withheld measurement, not a claim about the file, so
/// it is picked to be within an order of magnitude of both a low-bitrate
/// Opus capture and a lossless one.
const AUDIO_BYTES_PER_SECOND_BASIS: f64 = 8_000.0;

/// Characters per token for the transcript itself — the usual English
/// ratio, used only when the provider reports no completion count.
const TRANSCRIPT_CHARS_PER_TOKEN: f64 = 4.0;

/// Where a ledger row's token counts came from, recorded in the audit detail
/// so a client-facing figure stays explainable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenSource {
    /// The provider reported both counts.
    ProviderUsage,
    /// At least one count was derived here (no usage block, or a partial
    /// one).
    Derived,
}

impl TokenSource {
    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            TokenSource::ProviderUsage => "provider_usage",
            TokenSource::Derived => "derived",
        }
    }
}

/// One transcription call's metered spend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MeteredTokens {
    pub prompt: i32,
    pub completion: i32,
    pub source: TokenSource,
}

impl MeteredTokens {
    /// The row's unweighted total; the database's generated
    /// `weighted_tokens` column multiplies it by the catalog weight.
    #[cfg(test)]
    pub(crate) fn total(self) -> i64 {
        i64::from(self.prompt) + i64::from(self.completion)
    }
}

/// Resolves the ledger counts for one call. Provider figures win; a missing
/// one is derived so the row is never zero (a zero row is invisible to
/// `binding_caps`, which is the bug this exists to close).
pub(crate) fn metered_tokens(
    usage: Option<ProviderUsage>,
    duration_seconds: Option<f64>,
    audio_bytes: usize,
    transcript: &str,
) -> MeteredTokens {
    let derived_prompt = audio_prompt_tokens(duration_seconds, audio_bytes);
    let derived_completion = transcript_completion_tokens(transcript);
    let prompt_tokens = usage.and_then(|u| u.prompt_tokens);
    let completion_tokens = usage.and_then(|u| u.completion_tokens);
    let source = if prompt_tokens.is_some() && completion_tokens.is_some() {
        TokenSource::ProviderUsage
    } else {
        TokenSource::Derived
    };
    MeteredTokens {
        prompt: prompt_tokens
            .map(|n| i32::try_from(n).unwrap_or(i32::MAX))
            .unwrap_or(derived_prompt),
        completion: completion_tokens
            .map(|n| i32::try_from(n).unwrap_or(i32::MAX))
            .unwrap_or(derived_completion),
        source,
    }
}

/// Audio input tokens: the reported duration, else the byte size at the
/// documented basis. Never zero for a non-empty file.
fn audio_prompt_tokens(duration_seconds: Option<f64>, audio_bytes: usize) -> i32 {
    let seconds = match duration_seconds {
        Some(d) if d.is_finite() && d > 0.0 => d,
        _ => audio_bytes as f64 / AUDIO_BYTES_PER_SECOND_BASIS,
    }
    .max(1.0);
    let tokens = (seconds * AUDIO_TOKENS_PER_SECOND).ceil();
    if tokens >= f64::from(i32::MAX) {
        i32::MAX
    } else {
        tokens as i32
    }
}

/// Transcript output tokens: the usual characters-per-token ratio, at least
/// one for any non-empty transcript.
fn transcript_completion_tokens(transcript: &str) -> i32 {
    let chars = transcript.chars().count();
    if chars == 0 {
        return 0;
    }
    let tokens = (chars as f64 / TRANSCRIPT_CHARS_PER_TOKEN).ceil();
    if tokens >= f64::from(i32::MAX) {
        i32::MAX
    } else {
        (tokens as i32).max(1)
    }
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
    let usage = parsed.get("usage").and_then(ProviderUsage::from_wire);
    Ok(ProviderTranscript {
        text,
        duration,
        usage,
    })
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

#[cfg(test)]
mod metering_tests {
    use super::*;

    #[test]
    fn provider_usage_wins_over_every_derivation() {
        let usage = ProviderUsage {
            prompt_tokens: Some(120),
            completion_tokens: Some(40),
        };
        let metered = metered_tokens(Some(usage), Some(90.0), 1_000_000, "a long transcript");
        assert_eq!(metered.prompt, 120);
        assert_eq!(metered.completion, 40);
        assert_eq!(metered.source, TokenSource::ProviderUsage);
    }

    #[test]
    fn the_openai_audio_shape_parses_input_and_output_tokens() {
        let wire: Value = serde_json::json!({
            "type": "tokens",
            "input_tokens": 14,
            "input_token_details": { "text_tokens": 0, "audio_tokens": 14 },
            "output_tokens": 45,
            "total_tokens": 59,
        });
        let usage = ProviderUsage::from_wire(&wire).expect("an audio usage block parses");
        assert_eq!(usage.prompt_tokens, Some(14));
        assert_eq!(usage.completion_tokens, Some(45));
        assert_eq!(metered_tokens(Some(usage), None, 1_000, "hi").total(), 59);
    }

    #[test]
    fn a_response_without_usage_is_still_metered() {
        // The whisper shape: text and duration, no usage block.
        let metered = metered_tokens(None, Some(30.0), 900_000, "0123456789");
        assert_eq!(metered.prompt, 300, "30 s at the audio basis");
        assert_eq!(metered.completion, 3, "ten characters, four per token");
        assert_eq!(metered.source, TokenSource::Derived);
        assert!(metered.total() > 0, "a metered call never records zero");
    }

    #[test]
    fn a_response_without_usage_or_duration_uses_the_byte_basis() {
        // 160 kB at the documented basis is 20 s of audio.
        let metered = metered_tokens(None, None, 160_000, "x");
        assert_eq!(metered.prompt, 200);
        assert!(metered.completion >= 1);
    }

    #[test]
    fn the_weighted_total_is_the_counts_times_the_catalog_weight() {
        // Mirrors `weighted_tokens` in 0091_wave18_ai_gaps.sql:
        // (prompt_tokens + completion_tokens) * cost_weight.
        let metered = metered_tokens(
            Some(ProviderUsage {
                prompt_tokens: Some(120),
                completion_tokens: Some(40),
            }),
            None,
            0,
            "unused",
        );
        for weight in 1_i64..=5 {
            assert_eq!(metered.total() * weight, (120 + 40) * weight);
        }
    }
}
