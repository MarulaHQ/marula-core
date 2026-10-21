//! The org-resolved AI edge (§4.7 + §4.5): embeddings for the unified
//! search vector leg, generated through the EXISTING `CompletionProvider`
//! seam and the org's BYOK sealed keys — no new provider abstraction.
//!
//! Purpose:
//! - resolve one org's provider from a raw settings read plus the sealed BYOK
//!   secret (migration 0044), decrypting in-process only;
//! - ship the embeddings adapter: the OpenAI-compatible `/embeddings` shape
//!   for cloud providers, and Ollama's native `/api/embed` for the local
//!   default, which is the only shape that carries `keep_alive` and therefore
//!   the only one where load-on-demand and unload-when-idle are explicit;
//! - run the bundled model locally when the org configured nothing: an
//!   unset `ai.completion_provider` resolves to the bundled EmbeddingGemma 2
//!   on the local runtime, while an explicit `none` keeps the vector leg dark;
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
    resolve_embedding_config, CompletionError, CompletionProvider, CompletionRequest,
    CompletionResponse, CompletionUsage, EmbeddingRequest, EmbeddingResponse,
    NoneCompletionProvider, BUNDLED_EMBEDDING_MODEL, SEARCH_EMBEDDING_DIM,
};
use uuid::Uuid;

/// Base URL of the OpenAI embeddings API (override for tests/gateways).
const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
/// Model used when the org has not set `ai.completion_model` (or set it to
/// the "none" placeholder): the cheapest OpenAI-compatible text embedder.
const OPENAI_DEFAULT_MODEL: &str = "text-embedding-3-small";
/// Ollama's endpoint. The bundled embedder talks to its native `/api/embed`
/// (which carries `keep_alive`), and an operator who points the cloud adapter
/// at Ollama's OpenAI compatibility layer still gets `/v1` handled.
const OLLAMA_DEFAULT_BASE: &str = "http://localhost:11434";
/// Idle window for the local embedder. Ollama loads the model on the first
/// request that needs it and drops it after this much quiet, so an install
/// that searches twice an hour is not holding a model resident all day.
const LOCAL_EMBED_KEEP_ALIVE: &str = "5m";
/// Request budget for one local batch. A backfill batch is 128 inputs
/// (`core_db::search::EMBEDDING_BATCH`), and the slowest single batch measured
/// on the 4-core reference profile ran to double-digit seconds; the cloud
/// adapter's budget would cut that off mid-sweep on a host doing exactly what
/// the product asks of it.
const LOCAL_EMBED_TIMEOUT_SECS: u64 = 300;
/// Request budget for one cloud batch. A cloud embedder answers a batch of
/// this size in well under a second, so a request that takes longer is a
/// broken endpoint rather than a slow one.
const CLOUD_EMBED_TIMEOUT_SECS: u64 = 30;

/// Which HTTP shape the adapter speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbedTransport {
    /// `POST {base}/embeddings` — the OpenAI wire format.
    OpenAiCompatible,
    /// `POST {base}/api/embed` — Ollama's native format, the only one that
    /// takes `keep_alive`.
    Ollama,
}

/// The embeddings adapter behind `CompletionProvider::embed`. The OpenAI wire
/// format is the general case; the Ollama transport exists because load-on-
/// demand and unload-when-idle are request parameters on the native endpoint
/// and nowhere else, and the bundled default runs there.
pub struct OpenAiEmbeddings {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    model: String,
    send_dimensions: bool,
    transport: EmbedTransport,
    keep_alive: String,
    timeout: std::time::Duration,
}

impl OpenAiEmbeddings {
    /// The concrete embedding model this provider invokes (stored next to
    /// every vector as provenance; a change re-embeds on the next sweep).
    pub fn model_name(&self) -> &str {
        &self.model
    }

    /// Root of the native Ollama API: the configured base with any OpenAI
    /// compatibility suffix removed, so one setting serves both shapes.
    fn native_base(&self) -> &str {
        let base = self.base_url.trim_end_matches('/');
        base.strip_suffix("/v1").unwrap_or(base)
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

/// Ollama's native `/api/embed` response: vectors in input order, with no
/// per-item index and no usage block.
#[derive(Debug, serde::Deserialize)]
struct OllamaEmbedWire {
    #[serde(default)]
    embeddings: Vec<Vec<f32>>,
    model: Option<String>,
    prompt_eval_count: Option<u32>,
}

/// Refuses any vector that is not [`SEARCH_EMBEDDING_DIM`] wide, so a
/// misconfigured provider leaves the vector leg dark rather than writing a
/// vector the rest of the search path cannot compare against.
fn check_embedding_width(embeddings: &[Vec<f32>]) -> Result<(), CompletionError> {
    for embedding in embeddings {
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
    Ok(())
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
    check_embedding_width(&embeddings)?;
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

/// Parses Ollama's native `/api/embed` response. The vectors come back in
/// input order, so arity is the whole ordering contract. Pure, unit-tested.
pub fn parse_ollama_embed_response(
    fallback_model: &str,
    body: &serde_json::Value,
) -> Result<EmbeddingResponse, CompletionError> {
    let wire: OllamaEmbedWire =
        serde_json::from_value(body.clone()).map_err(|error| CompletionError::Backend {
            reason: format!("malformed ollama embed response: {error}"),
        })?;
    if wire.embeddings.is_empty() {
        return Err(CompletionError::Backend {
            reason: "embeddings response carried no vectors".to_string(),
        });
    }
    check_embedding_width(&wire.embeddings)?;
    Ok(EmbeddingResponse {
        embeddings: wire.embeddings,
        model: wire.model.unwrap_or_else(|| fallback_model.to_string()),
        usage: wire.prompt_eval_count.map(|count| CompletionUsage {
            prompt_tokens: count,
            completion_tokens: 0,
            total_tokens: count,
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
        let model = req.model.as_deref().unwrap_or(&self.model);
        let (url, body) = match self.transport {
            EmbedTransport::OpenAiCompatible => {
                // Providers with Matryoshka truncation (OpenAI
                // text-embedding-3) get the explicit target width; others
                // must return it natively.
                let mut body = serde_json::json!({
                    "model": model,
                    "input": req.inputs,
                    "encoding_format": "float",
                });
                if self.send_dimensions {
                    body["dimensions"] = serde_json::json!(SEARCH_EMBEDDING_DIM);
                }
                (
                    format!("{}/embeddings", self.base_url.trim_end_matches('/')),
                    body,
                )
            }
            EmbedTransport::Ollama => (
                format!("{}/api/embed", self.native_base()),
                serde_json::json!({
                    "model": model,
                    "input": req.inputs,
                    // The model loads on this request and is dropped again
                    // after this much quiet. The window is the difference
                    // between a resident model and a warm one.
                    "keep_alive": self.keep_alive,
                }),
            ),
        };
        let mut request = self.http.post(url).json(&body);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request
            .timeout(self.timeout)
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
        match self.transport {
            EmbedTransport::OpenAiCompatible => parse_embedding_response(self.model_name(), &body),
            EmbedTransport::Ollama => parse_ollama_embed_response(self.model_name(), &body),
        }
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
///
/// An org that configured nothing gets the bundled local embedder rather than
/// a dark vector leg. That is the default the product ships: EmbeddingGemma 2
/// on the `ollama` runtime, loaded on demand. `ai.features_enabled` still
/// gates whether any AI edge runs at all, so this does not turn AI on for an
/// org that left it off.
pub async fn resolve_org_completion_provider(
    state: &AppState,
    org_id: Uuid,
) -> OrgCompletionProvider {
    let Ok((provider_setting, model_setting, secret)) = read_provider_config(state, org_id).await
    else {
        return OrgCompletionProvider::None(NoneCompletionProvider);
    };
    let config = resolve_embedding_config(provider_setting.as_deref(), model_setting.as_deref());
    let (base_url, default_model, send_dimensions, transport) = match config.provider.as_str() {
        "openai" => (
            std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| OPENAI_BASE_URL.to_string()),
            OPENAI_DEFAULT_MODEL.to_string(),
            true,
            EmbedTransport::OpenAiCompatible,
        ),
        "ollama" => (
            std::env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| OLLAMA_DEFAULT_BASE.to_string()),
            bundled_embedding_model(),
            false,
            EmbedTransport::Ollama,
        ),
        // "none" is the documented off switch. anthropic has no embeddings
        // API and "custom" has no base-URL setting yet, so both stay none
        // mode rather than guess an endpoint.
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
    let local = transport == EmbedTransport::Ollama;
    OrgCompletionProvider::Embeddings(OpenAiEmbeddings {
        http: reqwest::Client::new(),
        base_url,
        api_key,
        model: config.model.unwrap_or(default_model),
        send_dimensions,
        transport,
        keep_alive: embedding_keep_alive(),
        timeout: std::time::Duration::from_secs(if local {
            LOCAL_EMBED_TIMEOUT_SECS
        } else {
            CLOUD_EMBED_TIMEOUT_SECS
        }),
    })
}

/// The model an install runs when no org names one. `MARULA_EMBEDDING_MODEL`
/// is the instance-level lever; the catalogue lane owns the selection surface
/// that replaces it.
fn bundled_embedding_model() -> String {
    std::env::var("MARULA_EMBEDDING_MODEL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| BUNDLED_EMBEDDING_MODEL.to_string())
}

/// How long the local model stays resident after its last request.
/// `MARULA_EMBEDDING_KEEP_ALIVE` takes Ollama's duration syntax (`30s`,
/// `5m`, `0` to unload immediately after every batch).
fn embedding_keep_alive() -> String {
    std::env::var("MARULA_EMBEDDING_KEEP_ALIVE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| LOCAL_EMBED_KEEP_ALIVE.to_string())
}

/// What the settings surface should report: the provider and model that would
/// actually serve the vector leg, not the raw override. Without this the card
/// shows "none (none)" while the bundled embedder is the thing running.
pub fn effective_embedding_labels(
    provider_setting: Option<&str>,
    model_setting: Option<&str>,
) -> (String, String) {
    let config = resolve_embedding_config(provider_setting, model_setting);
    let model = match config.provider.as_str() {
        "openai" => config
            .model
            .unwrap_or_else(|| OPENAI_DEFAULT_MODEL.to_string()),
        "ollama" => config.model.unwrap_or_else(bundled_embedding_model),
        _ => return ("none".to_string(), "none".to_string()),
    };
    (config.provider, model)
}

/// Reads the org's raw provider and model settings. `None` means the org
/// never set the key, which is different from setting it to "none".
async fn read_provider_config(
    state: &AppState,
    org_id: Uuid,
) -> core_db::DbResult<(Option<String>, Option<String>, Option<Vec<u8>>)> {
    let mut tx = state.pool.begin().await?;
    core_db::set_org(&mut tx, org_id).await?;
    let overrides = core_db::settings::list_org_overrides(&mut *tx, org_id).await?;
    let provider = overrides
        .iter()
        .find(|o| o.key == core_domain::settings::AI_COMPLETION_PROVIDER)
        .and_then(|o| o.value.as_str())
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let model = overrides
        .iter()
        .find(|o| o.key == core_domain::settings::AI_COMPLETION_MODEL)
        .and_then(|o| o.value.as_str())
        .map(str::to_string);
    // The sealed key is looked up for the provider that will actually run:
    // an unconfigured org looks up the bundled provider and finds nothing,
    // which is correct — the local runtime needs no credential.
    let secret_provider = resolve_embedding_config(provider.as_deref(), model.as_deref()).provider;
    let secret = core_db::byok::get_secret(&mut *tx, org_id, &secret_provider).await?;
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
/// provider AND the org master switch is on (ADR 0010 §4: OFF ⇒ the org
/// sees exactly the pre-AI product; the vector leg degrades to `None`
/// and the caller runs the legacy lexical path unchanged).
pub async fn embed_search_query(state: &AppState, org_id: Uuid, query: &str) -> Option<Vec<f32>> {
    let mut conn = state.pool.acquire().await.ok()?;
    if !core_db::ai_policy::features_enabled(conn.as_mut(), org_id)
        .await
        .ok()?
    {
        return None;
    }
    drop(conn);
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
    let mut conn = state.pool.acquire().await.map_err(ApiError::from)?;
    if !core_db::ai_policy::features_enabled(conn.as_mut(), org_id)
        .await
        .map_err(ApiError::from)?
    {
        return Ok(BackfillReport {
            embedded: 0,
            complete: true,
            reason: Some(
                "AI features are switched off for this organization: vector leg stays dark"
                    .to_string(),
            ),
        });
    }
    drop(conn);
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

    #[test]
    fn bundled_default_is_an_embeddinggemma_model() {
        // The bundled default has to produce exactly the pinned width, or the
        // first search it serves would write vectors nothing can compare to.
        assert_eq!(core_domain::SEARCH_EMBEDDING_DIM, 768);
        assert!(
            BUNDLED_EMBEDDING_MODEL.starts_with("embeddinggemma-2:"),
            "bundled default moved off EmbeddingGemma 2: {BUNDLED_EMBEDDING_MODEL}"
        );
        assert!(
            BUNDLED_EMBEDDING_MODEL.contains("-text"),
            "the default should be the text-only variant, which is the cheaper \
             resident set for the same text vectors: {BUNDLED_EMBEDDING_MODEL}"
        );
    }

    #[test]
    fn unset_settings_report_the_bundled_local_default() {
        let (provider, model) = effective_embedding_labels(None, None);
        assert_eq!(
            provider,
            core_domain::completion::BUNDLED_EMBEDDING_PROVIDER
        );
        assert_eq!(model, BUNDLED_EMBEDDING_MODEL);
    }

    #[test]
    fn explicit_none_stays_off() {
        assert_eq!(
            effective_embedding_labels(Some("none"), None),
            ("none".to_string(), "none".to_string())
        );
    }

    #[test]
    fn cloud_override_is_reported_as_chosen() {
        let (provider, model) = effective_embedding_labels(Some("openai"), None);
        assert_eq!(provider, "openai");
        assert_eq!(model, OPENAI_DEFAULT_MODEL);
        let (provider, model) =
            effective_embedding_labels(Some("openai"), Some("text-embedding-3-large"));
        assert_eq!(provider, "openai");
        assert_eq!(model, "text-embedding-3-large");
    }

    #[test]
    fn a_provider_without_an_embeddings_api_reports_none() {
        assert_eq!(
            effective_embedding_labels(Some("anthropic"), Some("claude")),
            ("none".to_string(), "none".to_string())
        );
    }

    #[test]
    fn ollama_base_url_serves_both_wire_shapes() {
        // One setting, two shapes: the native endpoint strips the OpenAI
        // compatibility suffix, and a bare host is left alone.
        let with_suffix = OpenAiEmbeddings {
            http: reqwest::Client::new(),
            base_url: "http://localhost:11434/v1".to_string(),
            api_key: None,
            model: BUNDLED_EMBEDDING_MODEL.to_string(),
            send_dimensions: false,
            transport: EmbedTransport::Ollama,
            keep_alive: LOCAL_EMBED_KEEP_ALIVE.to_string(),
            timeout: std::time::Duration::from_secs(LOCAL_EMBED_TIMEOUT_SECS),
        };
        assert_eq!(with_suffix.native_base(), "http://localhost:11434");
        let bare = OpenAiEmbeddings {
            base_url: "http://ollama:11434".to_string(),
            ..with_suffix
        };
        assert_eq!(bare.native_base(), "http://ollama:11434");
    }

    #[test]
    fn parse_reads_ollama_native_embeddings_in_input_order() {
        let body = json!({
            "model": BUNDLED_EMBEDDING_MODEL,
            "embeddings": [vector(1.0), vector(2.0)],
            "prompt_eval_count": 11
        });
        let parsed = parse_ollama_embed_response("fallback", &body).expect("parse");
        assert_eq!(parsed.model, BUNDLED_EMBEDDING_MODEL);
        assert_eq!(parsed.embeddings.len(), 2);
        assert_eq!(parsed.embeddings[1][0], 2.0);
        assert_eq!(parsed.usage.expect("usage").prompt_tokens, 11);
    }

    #[test]
    fn parse_refuses_wrong_width_ollama_vectors() {
        let body = json!({
            "model": "matryoshka-truncated",
            "embeddings": [vec![0.5_f32; 512]]
        });
        match parse_ollama_embed_response("fallback", &body) {
            Err(CompletionError::InvalidRequest { reason }) => {
                assert!(
                    reason.contains("768"),
                    "error must name the width: {reason}"
                );
            }
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    #[test]
    fn parse_refuses_an_empty_ollama_payload() {
        assert!(parse_ollama_embed_response("f", &json!({ "embeddings": [] })).is_err());
        assert!(parse_ollama_embed_response("f", &json!({ "embeddings": "nope" })).is_err());
    }
}
