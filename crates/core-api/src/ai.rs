//! The org-resolved AI edge (§4.7 + §4.5): embeddings for the unified
//! search vector leg, generated through the EXISTING `CompletionProvider`
//! seam and the org's BYOK sealed keys — no new provider abstraction.
//!
//! Purpose:
//! - resolve one org's provider from the typed settings registry
//!   (`ai.completion_provider` / `ai.completion_model`) plus the sealed
//!   BYOK secret (migration 0044), decrypting in-process only;
//! - ship the OpenAI-compatible embeddings adapter (`/embeddings`) that the
//!   `CompletionProvider::embed` default left to this lane — `openai` and
//!   `ollama`'s OpenAI-compatible endpoint are wired; providers without a
//!   usable embeddings API (none/anthropic/custom in v1) resolve to
//!   none-mode and the vector leg simply stays dark;
//! - degrade, never fail: a search or an index upsert that hits an
//!   unconfigured, broken or wrong-dimension provider logs and proceeds
//!   lexical-only. Search NEVER 5xxs because AI is down, and no vector of
//!   the wrong width is ever stored.
//!
//! Non-purpose: no vendor SDK dependency (plain reqwest + serde), no
//! chat-completions transport yet (the probe route validates credentials
//! without HTTP; a future lane wires chat), no key material in logs, trails,
//! or events.

use crate::error::ApiError;
use crate::state::AppState;
use core_domain::completion::{
    CompletionError, CompletionProvider, CompletionRequest, CompletionResponse, CompletionUsage,
    EmbeddingRequest, EmbeddingResponse, NoneCompletionProvider, SEARCH_EMBEDDING_DIM,
};
use uuid::Uuid;

/// Base URL of the OpenAI embeddings API (override for tests/gateways).
const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
/// Model used when the org has not set `ai.completion_model` (or set it to
/// the "none" placeholder): the cheapest OpenAI-compatible text embedder.
const OPENAI_DEFAULT_MODEL: &str = "text-embedding-3-small";
/// Ollama's OpenAI-compatible endpoint (documented self-host LLM serving).
const OLLAMA_DEFAULT_BASE: &str = "http://localhost:11434/v1";
/// Default self-host embedder: 768-dim, matching SEARCH_EMBEDDING_DIM.
const OLLAMA_DEFAULT_MODEL: &str = "nomic-embed-text";

/// The OpenAI-compatible embeddings adapter — the real HTTP adapter the
/// `CompletionProvider` trait comment anticipated. `send_dimensions` is set
/// only for providers that support Matryoshka truncation (OpenAI
/// text-embedding-3): the Ollama compatibility layer rejects unknown
/// request fields, so it gets the plain request and must return the
/// configured width natively.
pub struct OpenAiEmbeddings {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    send_dimensions: bool,
}

impl OpenAiEmbeddings {
    /// The concrete embedding model this provider invokes (stored next to
    /// every vector as provenance; a change re-embeds on the next sweep).
    pub fn model_name(&self) -> &str {
        &self.model
    }
}

#[derive(Debug, serde::Deserialize)]
struct EmbeddingsWire {
    #[serde(default)]
    data: Vec<EmbeddingWire>,
    model: Option<String>,
    usage: Option<UsageWire>,
}

#[derive(Debug, serde::Deserialize)]
struct EmbeddingWire {
    embedding: Vec<f32>,
    index: Option<usize>,
}

#[derive(Debug, serde::Deserialize)]
struct UsageWire {
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
    total_tokens: Option<u32>,
}

/// Parses an OpenAI-compatible `/embeddings` response into the domain
/// shape, enforcing the fixed vector width. Pure — unit-tested without I/O.
pub fn parse_embedding_response(
    fallback_model: &str,
    body: &serde_json::Value,
) -> Result<EmbeddingResponse, CompletionError> {
    let wire: EmbeddingsWire =
        serde_json::from_value(body.clone()).map_err(|error| CompletionError::Backend {
            reason: format!("malformed embeddings response: {error}"),
        })?;
    if wire.data.is_empty() {
        return Err(CompletionError::Backend {
            reason: "embeddings response carried no vectors".to_string(),
        });
    }
    // Honor `index` when present (batch responses are unordered in theory);
    // otherwise the document order is the input order.
    let mut items: Vec<(usize, Vec<f32>)> = wire
        .data
        .into_iter()
        .enumerate()
        .map(|(i, item)| (item.index.unwrap_or(i), item.embedding))
        .collect();
    items.sort_by_key(|(index, _)| *index);
    let embeddings: Vec<Vec<f32>> = items.into_iter().map(|(_, vector)| vector).collect();
    for embedding in &embeddings {
        if embedding.len() != SEARCH_EMBEDDING_DIM {
            return Err(CompletionError::InvalidRequest {
                reason: format!(
                    "provider returned {}-dim vectors, search requires exactly {SEARCH_EMBEDDING_DIM} \
                     (configure a conforming model or leave the provider at none)",
                    embedding.len()
                ),
            });
        }
    }
    Ok(EmbeddingResponse {
        embeddings,
        model: wire.model.unwrap_or_else(|| fallback_model.to_string()),
        usage: wire.usage.map(|u| CompletionUsage {
            prompt_tokens: u.prompt_tokens.unwrap_or(0),
            completion_tokens: u.completion_tokens.unwrap_or(0),
            total_tokens: u.total_tokens.unwrap_or(0),
        }),
    })
}

impl CompletionProvider for OpenAiEmbeddings {
    fn name(&self) -> &str {
        "openai-compatible"
    }

    /// Chat completions are not wired in this lane (the BYOK probe path
    /// validates credentials without HTTP); only the embeddings half of
    /// the OpenAI-compatible API is implemented.
    async fn complete(
        &self,
        _req: &CompletionRequest,
    ) -> Result<CompletionResponse, CompletionError> {
        Err(CompletionError::Unavailable {
            reason: "chat completions adapter not wired; embeddings only".to_string(),
        })
    }

    async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse, CompletionError> {
        if req.inputs.is_empty() {
            return Ok(EmbeddingResponse {
                embeddings: Vec::new(),
                model: self.model.clone(),
                usage: None,
            });
        }
        let url = format!("{}/embeddings", self.base_url.trim_end_matches('/'));
        // Providers with Matryoshka truncation (OpenAI text-embedding-3)
        // get the explicit target width; others must return it natively.
        let mut body = serde_json::json!({
            "model": req.model.as_deref().unwrap_or(&self.model),
            "input": req.inputs,
            "encoding_format": "float",
        });
        if self.send_dimensions {
            body["dimensions"] = serde_json::json!(core_domain::SEARCH_EMBEDDING_DIM);
        }
        let mut request = self.http.post(url).json(&body);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|error| CompletionError::Backend {
                reason: format!("embeddings request failed: {error}"),
            })?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.as_u16() == 429 {
            return Err(CompletionError::RateLimited {
                reason: format!("embeddings provider returned {status}"),
            });
        }
        if !status.is_success() {
            return Err(CompletionError::Backend {
                reason: format!("embeddings provider returned {status}"),
            });
        }
        let body: serde_json::Value =
            response
                .json()
                .await
                .map_err(|error| CompletionError::Backend {
                    reason: format!("embeddings response was not JSON: {error}"),
                })?;
        parse_embedding_response(self.model_name(), &body)
    }
}

/// The one provider an org can resolve to right now: none mode, or the
/// OpenAI-compatible embeddings adapter built from its BYOK key. A concrete
/// enum (not dyn) keeps the RPITIT trait dyn-free.
pub enum OrgCompletionProvider {
    None(NoneCompletionProvider),
    Embeddings(OpenAiEmbeddings),
}

impl CompletionProvider for OrgCompletionProvider {
    fn name(&self) -> &str {
        match self {
            OrgCompletionProvider::None(provider) => provider.name(),
            OrgCompletionProvider::Embeddings(provider) => provider.name(),
        }
    }

    async fn complete(
        &self,
        req: &CompletionRequest,
    ) -> Result<CompletionResponse, CompletionError> {
        match self {
            OrgCompletionProvider::None(provider) => provider.complete(req).await,
            OrgCompletionProvider::Embeddings(provider) => provider.complete(req).await,
        }
    }

    async fn embed(&self, req: &EmbeddingRequest) -> Result<EmbeddingResponse, CompletionError> {
        match self {
            OrgCompletionProvider::None(provider) => provider.embed(req).await,
            OrgCompletionProvider::Embeddings(provider) => provider.embed(req).await,
        }
    }
}

/// Resolves the org's provider from the typed settings registry + the sealed
/// BYOK store. Every failure mode — none mode, unsupported embeddings
/// provider, missing or undecryptable key — degrades to none mode with a
/// warning, never an error: the vector leg stays dark, search keeps working.
pub async fn resolve_org_completion_provider(
    state: &AppState,
    org_id: Uuid,
) -> OrgCompletionProvider {
    let Ok((provider_name, model_setting, secret)) = read_provider_config(state, org_id).await
    else {
        return OrgCompletionProvider::None(NoneCompletionProvider);
    };
    if provider_name_is_none(&provider_name) {
        return OrgCompletionProvider::None(NoneCompletionProvider);
    }
    let (base_url, default_model, send_dimensions) = match provider_name.as_str() {
        "openai" => (
            std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| OPENAI_BASE_URL.to_string()),
            OPENAI_DEFAULT_MODEL,
            true,
        ),
        "ollama" => (
            std::env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| OLLAMA_DEFAULT_BASE.to_string()),
            OLLAMA_DEFAULT_MODEL,
            false,
        ),
        // anthropic has no embeddings API; "custom" has no base-URL setting
        // yet — both stay none mode rather than guess an endpoint.
        _ => return OrgCompletionProvider::None(NoneCompletionProvider),
    };
    let api_key = match &secret {
        Some(ciphertext) => match crate::crypto::open(&state.config.secret_key, ciphertext) {
            Ok(bytes) => Some(String::from_utf8_lossy(&bytes).to_string()),
            Err(error) => {
                tracing::warn!(%org_id, %error, "byok secret failed to decrypt; vector leg degraded");
                return OrgCompletionProvider::None(NoneCompletionProvider);
            }
        },
        None => None,
    };
    let model = if model_setting_is_none(&model_setting) {
        default_model.to_string()
    } else {
        model_setting
    };
    OrgCompletionProvider::Embeddings(OpenAiEmbeddings {
        http: reqwest::Client::new(),
        base_url,
        api_key,
        model,
        send_dimensions,
    })
}

fn provider_name_is_none(value: &str) -> bool {
    value.trim().is_empty() || value == "none"
}

fn model_setting_is_none(value: &str) -> bool {
    value.trim().is_empty() || value == "none"
}

async fn read_provider_config(
    state: &AppState,
    org_id: Uuid,
) -> core_db::DbResult<(String, String, Option<Vec<u8>>)> {
    let mut tx = state.pool.begin().await?;
    core_db::set_org(&mut tx, org_id).await?;
    let overrides = core_db::settings::list_org_overrides(&mut *tx, org_id).await?;
    let provider = overrides
        .iter()
        .find(|o| o.key == core_domain::settings::AI_COMPLETION_PROVIDER)
        .and_then(|o| o.value.as_str())
        .unwrap_or("none")
        .to_ascii_lowercase();
    let model = overrides
        .iter()
        .find(|o| o.key == core_domain::settings::AI_COMPLETION_MODEL)
        .and_then(|o| o.value.as_str())
        .unwrap_or("none")
        .to_string();
    let secret = core_db::byok::get_secret(&mut *tx, org_id, &provider).await?;
    tx.commit().await?;
    Ok((provider, model, secret))
}

/// Embeds one batch of texts through the org's provider. `None` means "the
/// vector leg is dark for this org right now" (none mode, provider error,
/// or wrong-width output) — callers proceed without vectors, never fail.
/// Wrong-width vectors are refused here too, so a misbehaving provider can
/// never poison the index.
pub async fn embed_batch(
    provider: &OrgCompletionProvider,
    texts: Vec<String>,
) -> Option<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Some(Vec::new());
    }
    let count = texts.len();
    match provider
        .embed(&EmbeddingRequest {
            inputs: texts,
            model: None,
        })
        .await
    {
        Ok(response) if response.embeddings.len() == count => {
            if response
                .embeddings
                .iter()
                .all(|vector| vector.len() == core_domain::SEARCH_EMBEDDING_DIM)
            {
                Some(response.embeddings)
            } else {
                tracing::warn!(provider = %provider.name(), "wrong-width embedding batch refused");
                None
            }
        }
        Ok(_) => {
            tracing::warn!(provider = %provider.name(), "embedding batch arity mismatch refused");
            None
        }
        Err(CompletionError::Unavailable { reason }) => {
            // None mode is quiet: the documented default, not a warning.
            tracing::debug!(provider = %provider.name(), %reason, "vector leg unavailable");
            None
        }
        Err(error) => {
            tracing::warn!(provider = %provider.name(), %error, "embedding batch failed");
            None
        }
    }
}

/// Embeds the current search query, if the org has an embedding-capable
/// provider. `None` ⇒ the caller runs the legacy lexical path unchanged.
pub async fn embed_search_query(state: &AppState, org_id: Uuid, query: &str) -> Option<Vec<f32>> {
    let provider = resolve_org_completion_provider(state, org_id).await;
    let mut vectors = embed_batch(&provider, vec![query.to_string()]).await?;
    vectors.pop()
}

// --- backfill -----------------------------------------------------------------

/// Result of one idempotent backfill sweep for one org.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackfillReport {
    /// Rows that now carry a fresh vector.
    pub embedded: usize,
    /// False when the sweep hit its iteration budget — re-run to continue.
    pub complete: bool,
    /// Why nothing was embedded (none mode / provider error), for operators.
    pub reason: Option<String>,
}

/// Embeds at most `max_batches * core_db::search::EMBEDDING_BATCH` rows per
/// call so one request cannot run for minutes. Idempotent by construction:
/// the sweep only selects rows whose vector is missing, was produced by a
/// different model, or whose content hash has moved on — re-running a
/// completed sweep embeds nothing.
pub async fn backfill_org_embeddings(
    state: &AppState,
    org_id: Uuid,
    max_batches: i64,
) -> Result<BackfillReport, ApiError> {
    let provider = resolve_org_completion_provider(state, org_id).await;
    let model = match &provider {
        OrgCompletionProvider::Embeddings(embeddings) => embeddings.model.clone(),
        OrgCompletionProvider::None(_) => {
            return Ok(BackfillReport {
                embedded: 0,
                complete: true,
                reason: Some(
                    "no embedding provider configured (none mode): vector leg stays dark"
                        .to_string(),
                ),
            })
        }
    };

    let mut embedded = 0_usize;
    for _ in 0..max_batches {
        // Read the batch under RLS in one short tx; the provider call and
        // the write happen OUTSIDE it — no HTTP while holding a DB tx.
        let targets = {
            let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
            core_db::set_org(&mut tx, org_id)
                .await
                .map_err(ApiError::from)?;
            let batch = core_db::search::rows_needing_embedding(
                &mut *tx,
                org_id,
                &model,
                core_db::search::EMBEDDING_BATCH,
            )
            .await
            .map_err(ApiError::from)?;
            tx.commit().await.map_err(ApiError::from)?;
            batch
        };
        if targets.is_empty() {
            return Ok(BackfillReport {
                embedded,
                complete: true,
                reason: None,
            });
        }
        let texts: Vec<String> = targets
            .iter()
            .map(|target| search_embed_text(&target.title, &target.body))
            .collect();
        let Some(vectors) = embed_batch(&provider, texts).await else {
            return Ok(BackfillReport {
                embedded,
                complete: false,
                reason: Some("embedding provider failed mid-sweep; re-run to resume".to_string()),
            });
        };
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org(&mut tx, org_id)
            .await
            .map_err(ApiError::from)?;
        for (target, vector) in targets.iter().zip(vectors) {
            core_db::search::upsert_embedding(
                &mut *tx,
                org_id,
                target.kind,
                target.entity_id,
                &model,
                &vector,
            )
            .await
            .map_err(ApiError::from)?;
            embedded += 1;
        }
        tx.commit().await.map_err(ApiError::from)?;
    }
    Ok(BackfillReport {
        embedded,
        complete: false,
        reason: Some("iteration budget reached; re-run to continue".to_string()),
    })
}

/// The exact text a vector is computed from for one index row — the same
/// shape the FTS leg ranks on (title + body).
pub fn search_embed_text(title: &str, body: &str) -> String {
    format!("{title}\n{body}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vector(seed: f32) -> Vec<f32> {
        (0..core_domain::SEARCH_EMBEDDING_DIM)
            .map(|i| seed + i as f32 / 1000.0)
            .collect()
    }

    #[test]
    fn parse_maps_indexed_vectors_and_model() {
        let body = json!({
            "object": "list",
            "model": "text-embedding-3-small",
            "data": [
                { "object": "embedding", "index": 1, "embedding": vector(1.0) },
                { "object": "embedding", "index": 0, "embedding": vector(0.0) }
            ],
            "usage": { "prompt_tokens": 5, "total_tokens": 5 }
        });
        let parsed = parse_embedding_response("fallback", &body).expect("parse");
        assert_eq!(parsed.model, "text-embedding-3-small");
        assert_eq!(parsed.embeddings.len(), 2);
        assert_eq!(
            parsed.embeddings[0][0], 0.0,
            "index order wins, not doc order"
        );
        assert_eq!(parsed.usage.unwrap().prompt_tokens, 5);
    }

    #[test]
    fn parse_refuses_wrong_width_vectors() {
        let body = json!({
            "model": "mxbai-embed-large",
            "data": [ { "index": 0, "embedding": vec![0.5_f32; 1024] } ]
        });
        match parse_embedding_response("fallback", &body) {
            Err(CompletionError::InvalidRequest { reason }) => {
                assert!(reason.contains("1024"));
            }
            other => panic!("expected InvalidRequest, got {:?}", other.map(|_| ())),
        }
    }

    #[test]
    fn parse_refuses_empty_and_malformed_payloads() {
        assert!(parse_embedding_response("f", &json!({ "data": [] })).is_err());
        assert!(parse_embedding_response("f", &json!({ "data": "nope" })).is_err());
    }

    #[test]
    fn missing_index_falls_back_to_document_order() {
        let body = json!({
            "model": "nomic-embed-text",
            "data": [
                { "embedding": vector(1.0) },
                { "embedding": vector(2.0) }
            ]
        });
        let parsed = parse_embedding_response("f", &body).expect("parse");
        assert_eq!(parsed.embeddings[1][0], 2.0);
        assert_eq!(parsed.model, "nomic-embed-text");
    }

    #[test]
    fn embed_text_joins_title_and_body() {
        assert_eq!(search_embed_text("Mail relay", "down"), "Mail relay\ndown");
    }
}
