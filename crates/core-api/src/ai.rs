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
    CompletionError, CompletionProvider, CompletionRequest, CompletionResponse, CompletionUsage,
    EmbeddingRequest, EmbeddingResponse, NoneCompletionProvider, SEARCH_EMBEDDING_DIM,
};
use uuid::Uuid;

/// Base URL of the OpenAI embeddings API (override for tests/gateways).
const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
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
    /// The space this adapter's vectors belong to (Wave 52 lane R14):
    /// `model@version/dimensions`. Carried on the adapter so the writer and the
    /// reader cannot disagree about which model made a vector — the space is
    /// stamped beside the vector, in the same statement that writes it.
    space: String,
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

    /// The space tag this adapter writes into.
    pub fn space(&self) -> &str {
        &self.space
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

/// Why the vector leg is dark for an org. Typed rather than a string because two
/// of these are fixed by an operator and one of them is not: a search that
/// reports "no results" cannot tell them apart, and the re-index gauge has to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DarkReason {
    /// The org's own AI switch (`ai.completion_provider = none`).
    AiOff,
    /// The instance selected a cloud provider and no credential serves it.
    MissingCredential,
    /// The provider answered, and not with vectors.
    ProviderFailed,
    /// The org master switch (`ai.features_enabled`) is off (ADR 0010 §4).
    FeaturesDisabled,
}

impl DarkReason {
    /// Stable id for logs and the report.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AiOff => "ai_off",
            Self::MissingCredential => "missing_credential",
            Self::ProviderFailed => "provider_failed",
            Self::FeaturesDisabled => "features_disabled",
        }
    }
}

/// What serves one org's vector leg right now.
pub enum VectorLeg {
    /// The instance's model, and the space its vectors live in.
    Ready {
        provider: OrgCompletionProvider,
        space: String,
    },
    /// Nothing serves it, and this is the reason an operator can act on.
    Dark { reason: DarkReason, detail: String },
}

impl VectorLeg {
    /// The provider when the leg is ready.
    #[must_use]
    pub fn provider(self) -> Option<OrgCompletionProvider> {
        match self {
            Self::Ready { provider, .. } => Some(provider),
            Self::Dark { .. } => None,
        }
    }
}

/// Resolves the org's vector leg from the INSTANCE's selection plus the
/// credential that serves it (Wave 52 lane R14).
///
/// The order is the ruling, in this order:
///
/// 1. An org that set `ai.completion_provider = none` is dark. That is the off
///    switch and it stays one.
/// 2. The model and provider are the instance's, never the org's: two models in
///    one index are two incomparable spaces, and the ranking that compares them
///    is plausible and meaningless. `MARULA_EMBEDDING_TIER` / `_MODEL` /
///    `_PROVIDER` chose it at boot, and the catalogue validated it there.
/// 3. The local default needs no credential at all, which is why every org
///    reaches it on a fresh install. A cloud override is instance-wide and is
///    served by `MARULA_EMBEDDING_API_KEY` first, then by the org's own BYOK key
///    — and only for the provider the instance chose. An org key for some other
///    provider is not a second model; it is unused, and the leg stays dark
///    rather than quietly embedding with something else.
pub async fn resolve_vector_leg(state: &AppState, org_id: Uuid) -> VectorLeg {
    let instance = &state.config.embedding;
    let Ok((provider_setting, secret)) = read_org_ai_switch(state, org_id).await else {
        return VectorLeg::Dark {
            reason: DarkReason::AiOff,
            detail: "the organization's AI settings could not be read: vector leg stays dark"
                .to_string(),
        };
    };
    if provider_setting
        .as_deref()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("none"))
    {
        return VectorLeg::Dark {
            reason: DarkReason::AiOff,
            detail: "AI is switched off for this organization (ai.completion_provider = none): \
                     vector leg stays dark"
                .to_string(),
        };
    }

    let api_key = match &state.config.embedding_api_key {
        Some(key) => Some(key.clone()),
        None => match &secret {
            Some(ciphertext) => match crate::crypto::open(&state.config.secret_key, ciphertext) {
                Ok(bytes) => Some(String::from_utf8_lossy(&bytes).to_string()),
                Err(error) => {
                    tracing::warn!(
                        %org_id,
                        provider = %instance.provider,
                        %error,
                        "byok secret failed to decrypt; vector leg degraded"
                    );
                    return VectorLeg::Dark {
                        reason: DarkReason::MissingCredential,
                        detail: format!(
                            "the organization's stored key for `{}` could not be decrypted; \
                             store it again in Settings → AI",
                            instance.provider
                        ),
                    };
                }
            },
            None => None,
        },
    };

    let transport = transport_for(&instance.provider);
    let local = transport == EmbedTransport::Ollama;
    if !local && api_key.is_none() {
        return VectorLeg::Dark {
            reason: DarkReason::MissingCredential,
            detail: format!(
                "the instance runs `{}` on `{}`, which needs a credential: set \
                 MARULA_EMBEDDING_API_KEY on the instance, or store a key for `{}` in this \
                 organization's settings — the vector leg stays dark until one exists",
                instance.entry.id, instance.provider, instance.provider
            ),
        };
    }
    VectorLeg::Ready {
        space: instance.space_id(),
        provider: OrgCompletionProvider::Embeddings(OpenAiEmbeddings {
            http: reqwest::Client::new(),
            base_url: base_url_for(&instance.provider),
            api_key,
            model: instance.entry.id.to_string(),
            space: instance.space_id(),
            send_dimensions: !local,
            transport,
            keep_alive: embedding_keep_alive(),
            timeout: std::time::Duration::from_secs(if local {
                LOCAL_EMBED_TIMEOUT_SECS
            } else {
                CLOUD_EMBED_TIMEOUT_SECS
            }),
        }),
    }
}

/// The org's provider, when the leg is ready. Kept for the call sites that only
/// want "is there a provider" — the reason a leg is dark lives in
/// [`resolve_vector_leg`], which is what a report should carry.
pub async fn resolve_org_completion_provider(
    state: &AppState,
    org_id: Uuid,
) -> OrgCompletionProvider {
    resolve_vector_leg(state, org_id)
        .await
        .provider()
        .unwrap_or(OrgCompletionProvider::None(NoneCompletionProvider))
}

/// Whether a provider id means the local runtime. Every other id speaks the
/// OpenAI wire shape: a custom catalogue entry names its provider and the
/// operator who added it pointed `OPENAI_BASE_URL` at something that does.
fn transport_for(provider: &str) -> EmbedTransport {
    if provider.eq_ignore_ascii_case("ollama") {
        EmbedTransport::Ollama
    } else {
        EmbedTransport::OpenAiCompatible
    }
}

/// The endpoint a provider id resolves to.
fn base_url_for(provider: &str) -> String {
    if provider.eq_ignore_ascii_case("ollama") {
        std::env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| OLLAMA_DEFAULT_BASE.to_string())
    } else {
        std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| OPENAI_BASE_URL.to_string())
    }
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
/// actually serve the vector leg.
///
/// That is the instance's selection, and the org's setting can do exactly one
/// thing to it — switch it off. Reporting an org's `ai.completion_model` here
/// would tell the operator their model is serving when the instance's is, which
/// is the shape of bug this lane exists to remove.
pub fn effective_embedding_labels(
    instance: &core_domain::embedding::InstanceEmbedding,
    provider_setting: Option<&str>,
) -> (String, String) {
    if provider_setting.is_some_and(|value| value.trim().eq_ignore_ascii_case("none")) {
        return ("none".to_string(), "none".to_string());
    }
    (instance.provider.clone(), instance.entry.id.to_string())
}

/// Reads the org's AI switch and the sealed key for the INSTANCE's provider.
/// `None` for the provider means the org never set the key, which is different
/// from setting it to "none".
async fn read_org_ai_switch(
    state: &AppState,
    org_id: Uuid,
) -> core_db::DbResult<(Option<String>, Option<Vec<u8>>)> {
    let mut tx = state.pool.begin().await?;
    core_db::set_org(&mut tx, org_id).await?;
    let overrides = core_db::settings::list_org_overrides(&mut *tx, org_id).await?;
    let provider = overrides
        .iter()
        .find(|o| o.key == core_domain::settings::AI_COMPLETION_PROVIDER)
        .and_then(|o| o.value.as_str())
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    // The key is looked up for the provider that will actually run. An
    // unconfigured org on the local default finds nothing, which is correct:
    // the local runtime needs no credential.
    let secret =
        core_db::byok::get_secret(&mut *tx, org_id, &state.config.embedding.provider).await?;
    tx.commit().await?;
    Ok((provider, secret))
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
    /// False when the sweep hit its iteration budget or the provider failed —
    /// re-run to continue. Resumption is the selection predicate, not a cursor:
    /// rows already written in `space` are simply not selected again.
    pub complete: bool,
    /// Why nothing was embedded, for operators.
    pub reason: Option<String>,
    /// The typed reason, when the leg is dark.
    pub dark: Option<DarkReason>,
    /// The space this pass wrote into.
    pub space: String,
    /// Rows still lacking a fresh vector in `space`. The gauge's value.
    pub remaining: i64,
    /// Every index row this org has: the gauge's denominator.
    pub total_rows: i64,
    /// The run this pass advanced, when the sweep asked for one.
    pub run_id: Option<Uuid>,
}

impl BackfillReport {
    /// The counters before any embedding happened.
    fn snapshot(space: String, remaining: i64, total_rows: i64, run_id: Option<Uuid>) -> Self {
        Self {
            embedded: 0,
            complete: true,
            reason: None,
            dark: None,
            space,
            remaining,
            total_rows,
            run_id,
        }
    }

    /// A dark leg: nothing will be embedded until the reason is addressed.
    fn dark(mut self, reason: DarkReason, detail: impl Into<String>) -> Self {
        self.dark = Some(reason);
        self.reason = Some(detail.into());
        self
    }
}

/// Embeds at most `max_batches * core_db::search::EMBEDDING_BATCH` rows per
/// call so one request cannot run for minutes. Idempotent by construction: the
/// sweep only selects rows with no fresh vector in the instance's space, or
/// whose content hash has moved on — re-running a completed sweep embeds
/// nothing.
///
/// `run_id` is the re-index run this pass advances, when the worker's sweep
/// asked for one. The run's completion is marked in the same transaction that
/// deletes the previous space's rows for this org: a run that says "complete"
/// while the old population is still readable would be a lie about what search
/// can see.
pub async fn backfill_org_embeddings(
    state: &AppState,
    org_id: Uuid,
    max_batches: i64,
    run_id: Option<Uuid>,
) -> Result<BackfillReport, ApiError> {
    let space = state.config.embedding.space_id();

    // One read pass under RLS: the org switch, and the two counts the report
    // and the gauge need. Cheap (two indexed counts) and honest even when the
    // leg is dark — a dark leg still has a backlog, and that backlog is what an
    // operator is looking at when they ask why search misses.
    let (features_enabled, remaining, total_rows) = {
        let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
        core_db::set_org(&mut tx, org_id)
            .await
            .map_err(ApiError::from)?;
        let enabled = core_db::ai_policy::features_enabled(&mut *tx, org_id)
            .await
            .map_err(ApiError::from)?;
        let remaining = core_db::search::count_remaining_embeddings(&mut *tx, org_id, &space)
            .await
            .map_err(ApiError::from)?;
        let total = core_db::search::count_index_rows(&mut *tx, org_id)
            .await
            .map_err(ApiError::from)?;
        tx.commit().await.map_err(ApiError::from)?;
        (enabled, remaining, total)
    };
    let report = BackfillReport::snapshot(space.clone(), remaining, total_rows, run_id);
    if !features_enabled {
        return Ok(report.dark(
            DarkReason::FeaturesDisabled,
            "AI features are switched off for this organization: vector leg stays dark",
        ));
    }
    if remaining == 0 {
        // Nothing to do — and if this pass belongs to a run, the run is done.
        return finish_run_if_complete(state, org_id, &space, report).await;
    }

    let provider = match resolve_vector_leg(state, org_id).await {
        VectorLeg::Ready { provider, .. } => provider,
        VectorLeg::Dark { reason, detail } => return Ok(report.dark(reason, detail)),
    };
    let model = match &provider {
        OrgCompletionProvider::Embeddings(embeddings) => embeddings.model_name().to_string(),
        OrgCompletionProvider::None(_) => {
            return Ok(report.dark(
                DarkReason::AiOff,
                "no embedding provider configured (none mode): vector leg stays dark",
            ))
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
                &space,
                core_db::search::EMBEDDING_BATCH,
            )
            .await
            .map_err(ApiError::from)?;
            tx.commit().await.map_err(ApiError::from)?;
            batch
        };
        if targets.is_empty() {
            break;
        }
        let texts: Vec<String> = targets
            .iter()
            .map(|target| search_embed_text(&target.title, &target.body))
            .collect();
        let Some(vectors) = embed_batch(&provider, texts).await else {
            let remaining = count_remaining(state, org_id, &space).await?;
            let mut failed = BackfillReport::snapshot(space, remaining, total_rows, run_id);
            failed.embedded = embedded;
            failed.complete = false;
            return Ok(failed.dark(
                DarkReason::ProviderFailed,
                "embedding provider failed mid-sweep; the rows already written stay, \
                 and re-running resumes at the first row without a fresh vector",
            ));
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
                &space,
                &model,
                &vector,
            )
            .await
            .map_err(ApiError::from)?;
            embedded += 1;
        }
        tx.commit().await.map_err(ApiError::from)?;
    }

    let remaining = count_remaining(state, org_id, &space).await?;
    let mut report = BackfillReport::snapshot(space, remaining, total_rows, run_id);
    report.embedded = embedded;
    report.complete = remaining == 0;
    if !report.complete {
        report.reason = Some("iteration budget reached; re-run to continue".to_string());
        return Ok(report);
    }
    let space = report.space.clone();
    finish_run_if_complete(state, org_id, &space, report).await
}

/// Counts rows still missing a fresh vector in `space`.
async fn count_remaining(state: &AppState, org_id: Uuid, space: &str) -> Result<i64, ApiError> {
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let remaining = core_db::search::count_remaining_embeddings(&mut *tx, org_id, space)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(remaining)
}

/// Closes out a run whose org now has nothing left to embed.
///
/// Three statements, one transaction: delete this org's rows in the previous
/// space, mark the run complete, then clear `previous_space` once no run
/// anywhere is unfinished. The delete comes first so a crash between the two
/// repeats it (idempotent) rather than leaving orphan rows behind a closed run.
async fn finish_run_if_complete(
    state: &AppState,
    org_id: Uuid,
    space: &str,
    mut report: BackfillReport,
) -> Result<BackfillReport, ApiError> {
    report.complete = true;
    let Some(run_id) = report.run_id else {
        return Ok(report);
    };
    let mut tx = state.pool.begin().await.map_err(ApiError::from)?;
    core_db::set_org(&mut tx, org_id)
        .await
        .map_err(ApiError::from)?;
    let previous = core_db::embedding_space::read_state(&mut *tx)
        .await
        .map_err(ApiError::from)?
        .previous_space;
    if let Some(previous) = previous {
        if previous != space {
            let deleted = core_db::search::delete_space_embeddings(&mut *tx, org_id, &previous)
                .await
                .map_err(ApiError::from)?;
            tracing::info!(
                %org_id,
                previous_space = %previous,
                target_space = %space,
                deleted,
                "re-index complete for org; previous space rows removed"
            );
        }
    }
    core_db::embedding_space::record_progress(&mut tx, run_id, report.total_rows, 0, None)
        .await
        .map_err(ApiError::from)?;
    core_db::embedding_space::clear_previous_when_done(&mut *tx)
        .await
        .map_err(ApiError::from)?;
    tx.commit().await.map_err(ApiError::from)?;
    Ok(report)
}

// --- boot: the instance's space -----------------------------------------------

/// Records the space this instance writes and opens the one re-index window a
/// model change needs (Wave 52 lane R14).
///
/// Three outcomes:
/// - the recorded target is already this model's space: nothing happens, which
///   is what makes a restart not a re-index;
/// - the target changed to another BUILD of the same model id: the old rows stay
///   readable beside the new ones, because their vectors are comparable at the
///   ranking level the product measures (identical top-10 on 23 of 23 queries);
/// - the target changed to another MODEL: the old rows stop being readable
///   immediately. Search is under-filled until each org's run completes, and
///   under-filled is the honest state — a ranking assembled from two
///   incomparable spaces answers every query with noise.
///
/// The runs themselves are the worker's to create (its owner role is the one
/// that may insert across orgs); this only records the switch they hang off.
pub async fn ensure_instance_embedding_space(state: &AppState) -> Result<(), ApiError> {
    use core_domain::embedding::EmbeddingSpace;

    let target = state.config.embedding.space_id();
    let mut conn = state.pool.acquire().await.map_err(ApiError::from)?;
    let current = core_db::embedding_space::read_state(&mut *conn)
        .await
        .map_err(ApiError::from)?;
    let previous_readable = match (
        current.target_space.as_deref(),
        EmbeddingSpace::parse(&target),
    ) {
        (Some(old), Some(new)) => {
            EmbeddingSpace::parse(old).is_some_and(|old| old.same_model(&new))
        }
        _ => false,
    };
    let Some(switch) =
        core_db::embedding_space::record_switch(&mut conn, &target, previous_readable)
            .await
            .map_err(ApiError::from)?
    else {
        tracing::debug!(space = %target, "embedding space unchanged");
        return Ok(());
    };
    drop(conn);
    tracing::info!(
        switch_id = %switch.switch_id,
        from_space = switch.from_space.as_deref().unwrap_or("<none recorded>"),
        to_space = %switch.to_space,
        previous_readable = switch.previous_readable,
        "embedding space changed; the worker's sweep will queue one re-index run per org"
    );
    Ok(())
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

    fn instance(
        tier: Option<&str>,
        model: Option<&str>,
        provider: Option<&str>,
    ) -> core_domain::embedding::InstanceEmbedding {
        let catalogue = core_domain::embedding::Catalogue::builtin();
        core_domain::embedding::select_instance(&catalogue, tier, model, provider)
            .expect("selection resolves")
    }

    #[test]
    fn the_default_instance_is_embeddinggemma_text_only_on_the_local_path() {
        // The instance model has to produce exactly the pinned width, or the
        // first search it serves would write vectors nothing can compare to.
        assert_eq!(core_domain::SEARCH_EMBEDDING_DIM, 768);
        let selection = instance(None, None, None);
        assert_eq!(selection.entry.id, "embeddinggemma-2:270m-q8_0-text");
        assert_eq!(selection.provider, "ollama");
        assert!(selection.is_local(), "the default runs on this host");
        assert_eq!(
            selection.space_id(),
            "embeddinggemma-2:270m-q8_0-text@1/768"
        );
    }

    #[test]
    fn the_settings_card_reports_the_instances_model_and_the_org_can_only_turn_it_off() {
        let selection = instance(None, None, None);
        assert_eq!(
            effective_embedding_labels(&selection, None),
            ("ollama".to_string(), selection.entry.id.to_string())
        );
        assert_eq!(
            effective_embedding_labels(&selection, Some("none")),
            ("none".to_string(), "none".to_string()),
            "the org's own switch is the one thing its setting still does"
        );
        assert_eq!(
            effective_embedding_labels(&selection, Some("openai")),
            ("ollama".to_string(), selection.entry.id.to_string()),
            "naming another provider does not move the org onto another model"
        );
    }

    #[test]
    fn a_cloud_instance_reports_its_catalogue_model_and_has_no_local_floor() {
        let selection = instance(Some("standard"), None, Some("openai"));
        assert_eq!(selection.entry.id, "text-embedding-3-large");
        assert_eq!(
            effective_embedding_labels(&selection, None),
            ("openai".to_string(), "text-embedding-3-large".to_string())
        );
        assert!(!selection.is_local());
        assert_eq!(
            transport_for(&selection.provider),
            EmbedTransport::OpenAiCompatible
        );
    }

    #[test]
    fn the_local_provider_id_picks_the_native_shape_and_every_other_id_the_wire_one() {
        assert_eq!(transport_for("ollama"), EmbedTransport::Ollama);
        assert_eq!(transport_for("OLLAMA"), EmbedTransport::Ollama);
        assert_eq!(transport_for("openai"), EmbedTransport::OpenAiCompatible);
        assert_eq!(
            transport_for("acme-embeddings"),
            EmbedTransport::OpenAiCompatible
        );
    }

    #[test]
    fn the_adapter_carries_the_space_it_writes() {
        let selection = instance(Some("minimal"), None, None);
        let adapter = OpenAiEmbeddings {
            http: reqwest::Client::new(),
            base_url: base_url_for(&selection.provider),
            api_key: None,
            model: selection.entry.id.to_string(),
            space: selection.space_id(),
            send_dimensions: false,
            transport: EmbedTransport::Ollama,
            keep_alive: LOCAL_EMBED_KEEP_ALIVE.to_string(),
            timeout: std::time::Duration::from_secs(LOCAL_EMBED_TIMEOUT_SECS),
        };
        assert_eq!(adapter.space(), selection.space_id());
        assert_eq!(
            core_domain::embedding::EmbeddingSpace::parse(adapter.space())
                .expect("a stored tag parses")
                .model,
            selection.entry.id
        );
    }

    #[test]
    fn a_dark_leg_names_a_reason_an_operator_can_act_on() {
        let reasons = [
            DarkReason::AiOff,
            DarkReason::MissingCredential,
            DarkReason::ProviderFailed,
            DarkReason::FeaturesDisabled,
        ];
        let mut ids: Vec<&str> = reasons.iter().map(|r| r.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), reasons.len(), "one stable id per reason");
    }

    #[test]
    fn ollama_base_url_serves_both_wire_shapes() {
        // One setting, two shapes: the native endpoint strips the OpenAI
        // compatibility suffix, and a bare host is left alone.
        let with_suffix = OpenAiEmbeddings {
            http: reqwest::Client::new(),
            base_url: "http://localhost:11434/v1".to_string(),
            api_key: None,
            model: "embeddinggemma-2:270m-q8_0-text".to_string(),
            space: "embeddinggemma-2:270m-q8_0-text@1/768".to_string(),
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
            "model": "embeddinggemma-2:270m-q8_0-text",
            "embeddings": [vector(1.0), vector(2.0)],
            "prompt_eval_count": 11
        });
        let parsed = parse_ollama_embed_response("fallback", &body).expect("parse");
        assert_eq!(parsed.model, "embeddinggemma-2:270m-q8_0-text");
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
